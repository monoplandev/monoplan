// The web client's IndexedDB schema — one database, every store.
//
// All persistent-at-rest browser state for monoplan lives in the single
// `monoplan-web` database. Two layers share it:
//
//   Config plane (small, stable):
//     - `vault`  store  — wrapped DEK            (`dek-vault.ts`)
//     - `device` store  — device identity + `lastSyncAt` (`device-store.ts`)
//     - `prefs`  store  — per-account UI preferences (`@monoplan/web`)
//
//   Engine data plane (high-churn op log) — the web implementation of
//   the Rust `LocalStorage` trait (`spec/local-storage.md`), consumed by
//   `idb-storage.ts`:
//     - `docs`      store — one row per doc; carries the resume cursor
//     - `ops`       store — append-only op log keyed `(docId, localSeq)`
//     - `snapshots` store — one compacted snapshot per doc
//
// The two layers can't share an IDB transaction across *databases*, so
// they live in one database here: a single version line, a single
// `onupgradeneeded`, and the option of an atomic write spanning both
// planes if we ever need it. This module is the single source of truth
// for the schema; every caller goes through `openMonoplanDb()`.
//
// IDB's compound-index quirk — records where any key element is
// `undefined` are skipped — gives the engine `ops` store a partial
// unique index for free (`serverSeq` is unset on local-origin rows).

const DB_NAME = "monoplan-web";
// v1–v6 built up (and re-keyed) the config stores plus a now-defunct
// op-log-on-OPFS data plane (`ops` / `snapshot_meta`). v7 retired that
// data plane and briefly homed the engine op log in a *separate*
// `monoplan-engine` database. v8 collapsed that split back in: the engine
// stores (`docs` / `ops` / `snapshots`) were created here, and
// `monoplan-engine` abandoned. v9 is the WAL/VV separation
// (spec/vv-wal-separation.md): `ops` becomes the pure crash-recovery
// WAL (no more `clientOpId` / outbox role — it was recreated fresh at
// v9, its v8 rows abandoned; authed devices re-pull, and the snapshot
// baseline carries anonymous docs), the `docs` row gains
// `serverKnownVv`, and the `inflight` store holds the single durable
// in-flight push. v10 adds `peer_slots` (stable Loro peer id per tab
// slot, `spec/peer-id-plan.md`) — a pure addition; v9's WAL survives.
const DB_VERSION = 10;

// Config-plane stores.
export const STORE_VAULT = "vault";
export const STORE_DEVICE = "device";
export const STORE_PREFS = "prefs";

// Engine-data-plane stores + indexes.
export const STORE_DOCS = "docs";
export const STORE_OPS = "ops";
export const STORE_SNAPSHOTS = "snapshots";
export const STORE_INFLIGHT = "inflight";
export const STORE_PEER_SLOTS = "peer_slots";
export const INDEX_OPS_SERVER_SEQ = "docIdServerSeq";

// Pre-v7 oplog/OPFS stores, deleted on upgrade if present. `ops` is in
// this list *and* recreated below as an engine store — the delete runs
// first, so the legacy schema never survives into the engine store.
const LEGACY_STORES = ["snapshot_meta"];

// The retired separate engine database (v7 only). Deleted best-effort
// after the consolidated DB opens so it doesn't linger as an orphan.
const RETIRED_ENGINE_DB = "monoplan-engine";

/** One WAL row in the engine `ops` store. `serverSeq` is set on
 *  server-delivered rows (idempotent re-delivery detection) and unset
 *  on local-origin rows. Pure crash recovery — pruned whenever a
 *  snapshot contains it; upload is derived from `serverKnownVv`, never
 *  from these rows. */
export interface OpRow {
  docId: string;
  localSeq: number;
  serverSeq?: number;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
  createdAt: number;
}

export interface SnapshotRow {
  docId: string;
  upToLocalSeq: number;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
  createdAt: number;
}

/** One row in the `peer_slots` store — the stable Loro peer id for one
 *  tab slot (`spec/peer-id-plan.md`), the web analogue of the CLI's
 *  sqlite `peer_slots` table. `peerIdHex` is the u64 as 16 lowercase
 *  hex digits (IDB numbers can't hold a u64; BigInt can't be compared
 *  as a key). Minted once per slot, then read forever. */
export interface PeerSlotRow {
  slot: number;
  peerIdHex: string;
  createdAt: number;
}

/** One row in the `docs` store. `lastAckedServerSeq` is the persisted
 *  resume cursor — the highest *contiguous* serverSeq the engine has
 *  durably applied. Set explicitly via `writeAckedSeq` (never derived
 *  from `MAX(serverSeq)`, which over-shoots gaps and under-shoots after
 *  a snapshot prunes rows). `serverKnownVv` is the encoded Loro
 *  VersionVector of operations proven to exist in the server update stream
 *  — the outbound-delta base. See `spec/local-storage.md`. */
export interface DocRow {
  id: string;
  createdAt: number;
  lastAckedServerSeq: number;
  serverKnownVv?: Uint8Array;
}

/** The single durable in-flight push for a doc (`inflight` store,
 *  keyed by `docId`). Written before the blob's first send; cleared on
 *  ack. `pushId` is hex uuid bytes; `fromVv`/`toVv` are encoded Loro
 *  VersionVectors. */
export interface InFlightRow {
  docId: string;
  pushId: string;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
  fromVv: Uint8Array;
  toVv: Uint8Array;
  createdAt: number;
}

let cached: Promise<IDBDatabase> | null = null;
let cleanedUpRetiredDb = false;

/**
 * Open (or return the cached) `monoplan-web` database with every known
 * store materialised. The handle is shared across modules in the same
 * tab; a `versionchange` event from a peer tab triggering an upgrade
 * closes our handle and clears the cache so the next call opens a fresh
 * one.
 */
export function openMonoplanDb(): Promise<IDBDatabase> {
  if (!cached) {
    cached = openOnce().then((db) => {
      db.onversionchange = () => {
        db.close();
        cached = null;
      };
      dropRetiredEngineDb();
      return db;
    });
  }
  return cached;
}

function openOnce(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = (ev) => {
      const db = req.result;
      // Drop the retired pre-v7 oplog/OPFS stores. Pre-v9 the `ops`
      // store also carried outbox-era rows under the engine store's
      // name, so upgrades from before v9 delete it too (its rows are
      // abandoned — see the version history above) before the engine
      // store is recreated. From v9 on `ops` is the crash-recovery WAL
      // and MUST survive upgrades.
      const dropOps = ev.oldVersion < 9;
      for (const name of dropOps ? [...LEGACY_STORES, STORE_OPS] : LEGACY_STORES) {
        if (db.objectStoreNames.contains(name)) {
          db.deleteObjectStore(name);
        }
      }
      // Config-plane stores — each created only if missing so a DB at
      // any prior version converges to the current shape.
      if (!db.objectStoreNames.contains(STORE_VAULT)) {
        db.createObjectStore(STORE_VAULT);
      }
      if (!db.objectStoreNames.contains(STORE_DEVICE)) {
        // Device config keyed per account — devices belong to accounts,
        // not docs (one account, possibly many docs). The device row
        // carries `primaryDocId` as a field so it points at the
        // account's Home doc without needing a separate lookup.
        db.createObjectStore(STORE_DEVICE, { keyPath: "account_id" });
      }
      if (!db.objectStoreNames.contains(STORE_PREFS)) {
        // UI preferences keyed per account, deliberately separate from
        // `device` so view-change writes don't churn the sync-frontier
        // row and vice versa.
        db.createObjectStore(STORE_PREFS, { keyPath: "account_id" });
      }
      // Engine-data-plane stores. The logical model mirrors
      // `SqliteStorage` (CLI): one `docs` row per doc, an append-only
      // WAL (`ops`) keyed by `(docId, localSeq)`, one `snapshots` row
      // per doc, one `inflight` push row per doc, one `peer_slots` row
      // per tab slot.
      if (!db.objectStoreNames.contains(STORE_DOCS)) {
        db.createObjectStore(STORE_DOCS, { keyPath: "id" });
      }
      if (!db.objectStoreNames.contains(STORE_OPS)) {
        const ops = db.createObjectStore(STORE_OPS, {
          keyPath: ["docId", "localSeq"],
        });
        // Partial-unique by IDB's undefined-skipping rule (see header):
        // serverSeq set on server-delivered rows → uniqueness among them.
        ops.createIndex(INDEX_OPS_SERVER_SEQ, ["docId", "serverSeq"], {
          unique: true,
        });
      }
      if (!db.objectStoreNames.contains(STORE_SNAPSHOTS)) {
        db.createObjectStore(STORE_SNAPSHOTS, { keyPath: "docId" });
      }
      if (!db.objectStoreNames.contains(STORE_INFLIGHT)) {
        db.createObjectStore(STORE_INFLIGHT, { keyPath: "docId" });
      }
      if (!db.objectStoreNames.contains(STORE_PEER_SLOTS)) {
        db.createObjectStore(STORE_PEER_SLOTS, { keyPath: "slot" });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
    req.onblocked = () =>
      reject(new Error("monoplan-web open blocked by another tab"));
  });
}

/** Best-effort, once-per-session cleanup of the retired `monoplan-engine`
 *  database (the v7-only separate op-log DB, now folded into
 *  `monoplan-web`). Fire-and-forget: a peer tab on old code may block the
 *  delete, in which case it lands when that tab closes; either way boot
 *  never waits on it. */
function dropRetiredEngineDb(): void {
  if (cleanedUpRetiredDb) return;
  cleanedUpRetiredDb = true;
  try {
    indexedDB.deleteDatabase(RETIRED_ENGINE_DB);
  } catch {
    // Ignore — orphan DB is harmless, just untidy.
  }
}

/** Test-seam: forget the cached handle so a fresh `openMonoplanDb`
 *  re-acquires it. Don't use in product code. */
export function _resetMonoplanDbForTests(): void {
  cached = null;
}
