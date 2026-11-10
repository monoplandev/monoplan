// Shared in-memory `ReplicaStorage` test double. Same shape the wasm
// extern calls, same semantics as `core::MemStorage`. Method names line
// up with the `ReplicaStorage` interface generated into the wasm `.d.ts`.
//
// Every test that constructs a `Replica` must hand it a real
// storage — WAL capture, snapshots, and the sync cursors all flow
// through it. Cast to `ReplicaStorage` at the call site with `as
// unknown as ReplicaStorage` (the wasm interface is structural; the
// cast keeps each test pinned to its own wasm import).
//
// ⚠️ Byte args (`ciphertext` / `nonce` / `pushId` / VV encodings)
// arrive as `Uint8Array` **views into wasm linear memory**, valid only
// for the synchronous extern call. Because this mirror retains them
// past that call, it MUST copy on entry — exactly like the real
// `IdbStorage` (`copyBytes` = `.slice()`). Without the copy, bytes
// read back after the wasm allocator reuses that memory are garbage.
// (See `spec/local-storage.md`.)

interface MirrorOp {
  localSeq: number;
  serverSeq?: number;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
}

export interface MirrorInFlight {
  pushId: string;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
  fromVv: Uint8Array;
  toVv: Uint8Array;
}

function copy(bytes: Uint8Array): Uint8Array {
  return bytes.slice();
}

export class MemReplicaStorage {
  nextLocalSeq = 0;
  ops: MirrorOp[] = [];
  snapshot: { upToLocalSeq: number; ciphertext: Uint8Array; nonce: Uint8Array } | null = null;
  lastAckedServerSeq = 0;
  serverKnownVv: Uint8Array = new Uint8Array(0);
  inFlight: MirrorInFlight | null = null;

  appendLocalWal(ciphertext: Uint8Array, nonce: Uint8Array): number {
    const localSeq = ++this.nextLocalSeq;
    this.ops.push({ localSeq, ciphertext: copy(ciphertext), nonce: copy(nonce) });
    return localSeq;
  }

  appendRemoteWal(
    serverSeq: number,
    ciphertext: Uint8Array,
    nonce: Uint8Array,
    serverKnownVv: Uint8Array,
  ): { localSeq: number; inserted: boolean } {
    // VV advances on the duplicate path too — a re-delivered blob
    // still proves server possession.
    this.serverKnownVv = copy(serverKnownVv);
    const existing = this.ops.find((o) => o.serverSeq === serverSeq);
    if (existing) return { localSeq: existing.localSeq, inserted: false };
    const localSeq = ++this.nextLocalSeq;
    // Cursor advances only via writeAckedSeq (mirrors core::MemStorage).
    this.ops.push({ localSeq, serverSeq, ciphertext: copy(ciphertext), nonce: copy(nonce) });
    return { localSeq, inserted: true };
  }

  writeSnapshot(cutoff: number, ciphertext: Uint8Array, nonce: Uint8Array): void {
    // High-water = the counter, not the cutoff (see IdbStorage). Prune
    // the whole prefix — unsent ops survive inside the full-history
    // snapshot and re-derive from serverKnownVv.
    this.snapshot = { upToLocalSeq: this.nextLocalSeq, ciphertext: copy(ciphertext), nonce: copy(nonce) };
    this.ops = this.ops.filter((o) => o.localSeq > cutoff);
  }

  writeAckedSeq(serverSeq: number): void {
    this.lastAckedServerSeq = serverSeq;
  }

  writeServerKnownVv(vv: Uint8Array): void {
    this.serverKnownVv = copy(vv);
  }

  putInFlightPush(
    pushId: Uint8Array,
    ciphertext: Uint8Array,
    nonce: Uint8Array,
    fromVv: Uint8Array,
    toVv: Uint8Array,
  ): void {
    this.inFlight = {
      pushId: hex(pushId),
      ciphertext: copy(ciphertext),
      nonce: copy(nonce),
      fromVv: copy(fromVv),
      toVv: copy(toVv),
    };
  }

  completePush(pushId: Uint8Array, serverKnownVv: Uint8Array): void {
    this.serverKnownVv = copy(serverKnownVv);
    if (this.inFlight?.pushId === hex(pushId)) this.inFlight = null;
  }
}

export function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function unhex(s: string): Uint8Array {
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}
