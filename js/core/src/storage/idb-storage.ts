// Web implementation of the Rust `LocalStorage` trait
// (`spec/local-storage.md`, model in `spec/vv-wal-separation.md`).
//
// The engine's trait is **synchronous** — `appendLocalWal` must return
// a `localSeq` immediately — but IndexedDB is async. We bridge the gap
// with an in-memory mirror of the WAL: the synchronous methods
// read/write the mirror and return at once, while the underlying IDB
// write is queued onto a background flush chain. Real durability (the
// IDB transaction committing) is surfaced to the engine out-of-band via
// `whenFlushed()` → the host's `notifyOplogDurable`, so the server's
// `Ack` isn't shipped until the bytes are actually on disk.
//
// Writes that the trait requires to be atomic (remote WAL append +
// `serverKnownVv` advance; snapshot replace + prefix prune; push
// completion + VV) each run as ONE queued IDB transaction spanning the
// involved stores.
//
// This object is handed to the wasm `SyncEngine` constructor as the
// `EngineStorage` — its method names line up with the extern interface
// declared in `core/web/src/lib.rs`. Encrypted payloads cross as
// `(ciphertext, nonce)` byte pairs; `pushId` as the raw 16 UUID bytes;
// encoded VersionVectors as opaque byte arrays. Everything stays opaque
// (the DEK never crosses this boundary — the engine seals/opens
// before/after).

import {
  type DocRow,
  type InFlightRow,
  type OpRow,
  openMonoplanDb,
  type SnapshotRow,
  STORE_DOCS,
  STORE_INFLIGHT,
  STORE_OPS,
  STORE_SNAPSHOTS,
} from "./web-db.ts";

/** In-memory mirror WAL row. `serverSeq` is set on server-delivered
 *  rows only. */
interface MirrorOp {
  localSeq: number;
  serverSeq?: number;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
  createdAt: number;
}

/** The in-flight push in the boot shape the host seeds the engine
 *  with (`engine.seedInFlightPush`). `pushId` is the raw 16 uuid
 *  bytes. */
export interface InFlightPushJs {
  pushId: Uint8Array;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
  fromVv: Uint8Array;
  toVv: Uint8Array;
}

/** What the host needs to rebuild the `Doc` and seed the engine before
 *  constructing it: the snapshot (if any), every WAL row after it, and
 *  the persisted sync cursors. */
export interface EngineBootRows {
  snapshot: { ciphertext: Uint8Array; nonce: Uint8Array } | null;
  replay: { ciphertext: Uint8Array; nonce: Uint8Array }[];
  /** Highest `localSeq` ever assigned (max of snapshot frontier and
   *  the WAL) — seeds `engine.setLastLocalSeq`. */
  lastLocalSeq: number;
  /** Persisted resume cursor — the highest contiguous serverSeq the
   *  engine durably applied last session. Seeds `SyncEngine`'s
   *  `lastAckedSeq` (the `since_seq` of the resume `PullOps`). */
  lastAckedSeq: number;
  /** Encoded `serverKnownVv` (empty = never synced) — seeds
   *  `engine.seedServerKnownVv`. */
  serverKnownVv: Uint8Array;
  /** The durable in-flight push, if the previous session left one —
   *  seeds `engine.seedInFlightPush`. */
  inFlightPush: InFlightPushJs | null;
  /** Payload bytes across the surviving WAL rows — with
   *  `replay.length`, seeds `engine.seedWalStats`. */
  walBytes: number;
}

const HEX: string[] = Array.from({ length: 256 }, (_, i) =>
  i.toString(16).padStart(2, "0"),
);

function bytesToHex(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += HEX[bytes[i]];
  return s;
}

function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

// wasm-bindgen passes `&[u8]` args as a Uint8Array that is a *view*
// into wasm linear memory, valid only for the duration of the
// synchronous call. We retain these bytes (in the mirror and in a
// deferred IDB write), so we must copy them into a JS-owned buffer
// immediately — otherwise wasm reuses that memory and we persist
// garbage (which then fails to decrypt on the next boot).
function copyBytes(view: Uint8Array): Uint8Array {
  return view.slice();
}

export class IdbStorage {
  private nextLocalSeq = 0;
  private snapshot: SnapshotRow | null = null;
  private ops: MirrorOp[] = [];
  private lastAckedServerSeq = 0;
  private serverKnownVv: Uint8Array = new Uint8Array(0);
  private inFlight: InFlightRow | null = null;
  // Preserved across doc-row rewrites so updating a cursor doesn't
  // clobber the original creation time.
  private docCreatedAt = 0;
  // Background IDB writes are serialised through this chain so on-disk
  // order matches mirror order. Per-segment errors are logged and
  // swallowed to keep the chain alive (a poisoned chain would stall
  // every future write); `whenFlushed()` resolves once it settles.
  private flushChain: Promise<void> = Promise.resolve();

  private constructor(
    private readonly db: IDBDatabase,
    private readonly docId: string,
  ) {}

  /** Open the database, ensure the `docs` row, and load this doc's
   *  full WAL + snapshot + cursors into the mirror. */
  static async open(docId: string): Promise<IdbStorage> {
    const db = await openMonoplanDb();
    const storage = new IdbStorage(db, docId);
    await storage.load();
    return storage;
  }

  /** Drop every row of `docId` from the data plane: the `docs` row,
   *  its WAL, snapshot and in-flight push. For a doc deleted on the
   *  server (or no longer listed for this account). Any live
   *  `IdbStorage` for the doc must be abandoned first; this does not
   *  touch the vault or prefs. */
  static async wipe(docId: string): Promise<void> {
    const db = await openMonoplanDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(
        [STORE_DOCS, STORE_OPS, STORE_SNAPSHOTS, STORE_INFLIGHT],
        "readwrite",
      );
      tx.objectStore(STORE_DOCS).delete(docId);
      tx.objectStore(STORE_OPS).delete(
        IDBKeyRange.bound([docId, 0], [docId, Number.MAX_SAFE_INTEGER]),
      );
      tx.objectStore(STORE_SNAPSHOTS).delete(docId);
      tx.objectStore(STORE_INFLIGHT).delete(docId);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  }

  private load(): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(
        [STORE_DOCS, STORE_OPS, STORE_SNAPSHOTS, STORE_INFLIGHT],
        "readwrite",
      );
      const docs = tx.objectStore(STORE_DOCS);
      const docGet = docs.get(this.docId);
      docGet.onsuccess = () => {
        if (!docGet.result) {
          docs.put({
            id: this.docId,
            createdAt: Date.now(),
            lastAckedServerSeq: 0,
          });
        }
      };
      const range = IDBKeyRange.bound(
        [this.docId, 0],
        [this.docId, Number.MAX_SAFE_INTEGER],
      );
      const opsReq = tx.objectStore(STORE_OPS).getAll(range);
      const snapReq = tx.objectStore(STORE_SNAPSHOTS).get(this.docId);
      const inFlightReq = tx.objectStore(STORE_INFLIGHT).get(this.docId);
      tx.oncomplete = () => {
        const rows = (opsReq.result as OpRow[]) ?? [];
        this.ops = rows.map((r) => ({
          localSeq: r.localSeq,
          serverSeq: r.serverSeq,
          ciphertext: r.ciphertext,
          nonce: r.nonce,
          createdAt: r.createdAt,
        }));
        this.snapshot = (snapReq.result as SnapshotRow) ?? null;
        this.inFlight = (inFlightReq.result as InFlightRow) ?? null;
        // `upToLocalSeq` is the snapshot's stored high-water, not a replay
        // cutoff — take the max with the surviving rows so a prune that
        // deleted the row carrying the old max doesn't reset the counter.
        const highWater = this.snapshot?.upToLocalSeq ?? 0;
        const maxOp = rows.reduce((m, r) => Math.max(m, r.localSeq), 0);
        this.nextLocalSeq = Math.max(highWater, maxOp);
        // Read the persisted cursors from the docs row — NOT derived
        // from the WAL rows, which snapshot folding prunes. The engine
        // is the authority; we just replay what it last wrote.
        const docRow = docGet.result as DocRow | undefined;
        this.docCreatedAt = docRow?.createdAt ?? Date.now();
        this.lastAckedServerSeq = docRow?.lastAckedServerSeq ?? 0;
        this.serverKnownVv = docRow?.serverKnownVv ?? new Uint8Array(0);
        resolve();
      };
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error ?? new Error("idb load aborted"));
    });
  }

  // ---------- host-facing (async) ----------

  /** Snapshot + post-snapshot WAL rows for boot, plus every persisted
   *  cursor the engine seeds from. Read straight from the mirror
   *  loaded by `open()`. */
  bootRows(): EngineBootRows {
    // Replay every surviving row: `writeSnapshot` already pruned the
    // prefix the snapshot contains.
    const replay = this.ops
      .slice()
      .sort((a, b) => a.localSeq - b.localSeq)
      .map((o) => ({ ciphertext: o.ciphertext, nonce: o.nonce }));
    const walBytes = this.ops.reduce(
      (n, o) => n + o.ciphertext.length + o.nonce.length,
      0,
    );
    return {
      snapshot: this.snapshot
        ? { ciphertext: this.snapshot.ciphertext, nonce: this.snapshot.nonce }
        : null,
      replay,
      lastLocalSeq: this.nextLocalSeq,
      lastAckedSeq: this.lastAckedServerSeq,
      serverKnownVv: this.serverKnownVv,
      inFlightPush: this.inFlight
        ? {
            pushId: hexToBytes(this.inFlight.pushId),
            ciphertext: this.inFlight.ciphertext,
            nonce: this.inFlight.nonce,
            fromVv: this.inFlight.fromVv,
            toVv: this.inFlight.toVv,
          }
        : null,
      walBytes,
    };
  }

  /** Resolves once every queued IDB write has settled. The host awaits
   *  this before `notifyOplogDurable` so the server is told "I have seq
   *  N" only after N's bytes are on disk. */
  whenFlushed(): Promise<void> {
    return this.flushChain;
  }

  // ---------- synchronous LocalStorage surface (called from wasm) ----------

  appendLocalWal(ciphertext: Uint8Array, nonce: Uint8Array): number {
    const localSeq = ++this.nextLocalSeq;
    const row: OpRow = {
      docId: this.docId,
      localSeq,
      ciphertext: copyBytes(ciphertext),
      nonce: copyBytes(nonce),
      createdAt: Date.now(),
    };
    this.ops.push(toMirror(row));
    this.enqueuePut(STORE_OPS, row);
    return localSeq;
  }

  appendRemoteWal(
    serverSeq: number,
    ciphertext: Uint8Array,
    nonce: Uint8Array,
    serverKnownVv: Uint8Array,
  ): { localSeq: number; inserted: boolean } {
    // The VV advances even on the duplicate path — a re-delivered blob
    // still proves the server has those ops.
    this.serverKnownVv = copyBytes(serverKnownVv);
    const docRow = this.docRow();
    // Idempotent: a re-delivered serverSeq (resume re-pull, broadcast
    // overlap) is already stored — return its localSeq rather than
    // minting a phantom row. The VV write still lands.
    const existing = this.ops.find((o) => o.serverSeq === serverSeq);
    if (existing) {
      this.enqueuePut(STORE_DOCS, docRow);
      return { localSeq: existing.localSeq, inserted: false };
    }
    const localSeq = ++this.nextLocalSeq;
    // Appending does NOT advance the resume cursor — that's
    // `writeAckedSeq`'s job (an op above a gap would jump it past the
    // hole). Mirrors `core::MemStorage` / `SqliteStorage`.
    const row: OpRow = {
      docId: this.docId,
      localSeq,
      serverSeq,
      ciphertext: copyBytes(ciphertext),
      nonce: copyBytes(nonce),
      createdAt: Date.now(),
    };
    this.ops.push(toMirror(row));
    // One transaction: WAL row + advanced serverKnownVv commit together.
    this.enqueue([STORE_OPS, STORE_DOCS], (tx) => {
      tx.objectStore(STORE_OPS).put(row);
      tx.objectStore(STORE_DOCS).put(docRow);
    });
    return { localSeq, inserted: true };
  }

  writeSnapshot(cutoff: number, ciphertext: Uint8Array, nonce: Uint8Array): void {
    // High-water is the local counter, not the cutoff — pruning may
    // delete the row carrying the current max localSeq, so stamp the
    // counter to keep future appends monotonic.
    this.snapshot = {
      docId: this.docId,
      upToLocalSeq: this.nextLocalSeq,
      ciphertext: copyBytes(ciphertext),
      nonce: copyBytes(nonce),
      createdAt: Date.now(),
    };
    // Prune the folded prefix unconditionally — the full-history
    // snapshot provably contains every row at or below the cutoff,
    // including unsent local ops (upload is VV-derived, not
    // row-derived). Rows appended after the cutoff survive. The
    // cursors (`serverKnownVv`, `lastAckedServerSeq`) and any
    // in-flight push are untouched.
    const pruned = this.ops.filter((o) => o.localSeq <= cutoff);
    this.ops = this.ops.filter((o) => o.localSeq > cutoff);
    const snap = this.snapshot;
    const docId = this.docId;
    this.enqueue([STORE_OPS, STORE_SNAPSHOTS], (tx) => {
      tx.objectStore(STORE_SNAPSHOTS).put(snap);
      const ops = tx.objectStore(STORE_OPS);
      for (const o of pruned) ops.delete([docId, o.localSeq]);
    });
  }

  writeAckedSeq(serverSeq: number): void {
    this.lastAckedServerSeq = serverSeq;
    this.enqueuePut(STORE_DOCS, this.docRow());
  }

  writeServerKnownVv(vv: Uint8Array): void {
    this.serverKnownVv = copyBytes(vv);
    this.enqueuePut(STORE_DOCS, this.docRow());
  }

  putInFlightPush(
    pushId: Uint8Array,
    ciphertext: Uint8Array,
    nonce: Uint8Array,
    fromVv: Uint8Array,
    toVv: Uint8Array,
  ): void {
    const row: InFlightRow = {
      docId: this.docId,
      pushId: bytesToHex(pushId),
      ciphertext: copyBytes(ciphertext),
      nonce: copyBytes(nonce),
      fromVv: copyBytes(fromVv),
      toVv: copyBytes(toVv),
      createdAt: Date.now(),
    };
    this.inFlight = row;
    this.enqueuePut(STORE_INFLIGHT, row);
  }

  completePush(pushId: Uint8Array, serverKnownVv: Uint8Array): void {
    this.serverKnownVv = copyBytes(serverKnownVv);
    const docRow = this.docRow();
    // Clear only the matching record — a stale complete must not drop
    // a newer push's record.
    const hexId = bytesToHex(pushId);
    const clear = this.inFlight?.pushId === hexId;
    if (clear) this.inFlight = null;
    const docId = this.docId;
    // One transaction: VV advance + in-flight clear commit together.
    this.enqueue([STORE_DOCS, STORE_INFLIGHT], (tx) => {
      tx.objectStore(STORE_DOCS).put(docRow);
      if (clear) tx.objectStore(STORE_INFLIGHT).delete(docId);
    });
  }

  // ---------- IDB flush plumbing ----------

  private docRow(): DocRow {
    return {
      id: this.docId,
      createdAt: this.docCreatedAt,
      lastAckedServerSeq: this.lastAckedServerSeq,
      serverKnownVv: this.serverKnownVv,
    };
  }

  private enqueuePut(store: string, value: unknown): void {
    this.enqueue([store], (tx) => {
      tx.objectStore(store).put(value);
    });
  }

  private enqueue(
    stores: string[],
    body: (tx: IDBTransaction) => void,
  ): void {
    this.flushChain = this.flushChain
      .then(
        () =>
          new Promise<void>((resolve, reject) => {
            const tx = this.db.transaction(stores, "readwrite");
            body(tx);
            tx.oncomplete = () => resolve();
            tx.onerror = () => reject(tx.error);
            tx.onabort = () => reject(tx.error ?? new Error("idb tx aborted"));
          }),
      )
      .catch((e) => {
        console.error("[idb-storage] flush failed:", e);
      });
  }
}

function toMirror(row: OpRow): MirrorOp {
  return {
    localSeq: row.localSeq,
    serverSeq: row.serverSeq,
    ciphertext: row.ciphertext,
    nonce: row.nonce,
    createdAt: row.createdAt,
  };
}
