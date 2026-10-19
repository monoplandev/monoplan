// The web client drives the engine through a JS-implemented
// `EngineStorage` (`spec/local-storage.md`) — the in-memory mirror
// behind `IdbStorage`. This test exercises that wasm boundary —
// `captureLocalOps` / `snapshotIfWalExceeds` / the seed setters plus
// the `EngineStorage` extern — without IndexedDB or a server, using a
// hand-rolled in-memory mirror that mirrors `core::MemStorage`'s
// semantics. It locks in the contract the real `IdbStorage` satisfies
// (spec/vv-wal-separation.md):
//
//   - a captured local commit lands as a WAL row, and the push is
//     derived from `serverKnownVv` (a durable in-flight record with a
//     pushId, persisted before the send);
//   - an `OpsAck` naming that pushId clears the record and persists
//     the merged `serverKnownVv`;
//   - snapshot folding prunes the WAL prefix regardless of sync state
//     — unsent ops survive inside the full-history snapshot;
//   - remote ops applied from the wire are mirrored via
//     `appendRemoteWal` together with the advanced `serverKnownVv`.

import { describe, expect, test } from "bun:test";
import { decode, encode } from "@msgpack/msgpack";

import { Dek, Doc, EncryptedBlob, SyncEngine } from "../wasm/monoplan_core_web.js";
import type { EngineStorage } from "../wasm/monoplan_core_web.js";
import { MemEngineStorage, unhex } from "./mem-engine-storage.ts";

const LIST_MAIN = "inbox";
const DOC_ID = "00000000-0000-0000-0000-000000000000";
/** `DOC_ID` as the 16 raw bytes rmp_serde expects for a `Uuid`. */
const DOC_ID_BYTES = new Uint8Array(16);

function docOpenedBytes(): Uint8Array {
  return encode({ type: "DocOpened", doc_id: DOC_ID_BYTES, epoch: 0 });
}
function emptyBatchCompleteBytes(): Uint8Array {
  return encode({ type: "OpsBatch", doc_id: DOC_ID_BYTES, ops: [], complete: true });
}
function broadcastBytes(seq: number, blob: EncryptedBlob): Uint8Array {
  return encode({
    type: "OpsBroadcast",
    doc_id: DOC_ID_BYTES,
    ops: [{ seq, blob: { nonce: blob.nonce, ciphertext: blob.ciphertext } }],
  });
}
/** Build the OpsAck for a decoded PushOps frame at `seq`. */
function ackBytesFor(pushFrame: Uint8Array, seq: number): Uint8Array {
  const decoded = decode(pushFrame) as {
    type: string;
    ops: { push_id: Uint8Array; blob: unknown }[];
  };
  if (decoded.type !== "PushOps") throw new Error(`expected PushOps, got ${decoded.type}`);
  if (decoded.ops.length !== 1) throw new Error("engine ships one blob per push");
  return encode({
    type: "OpsAck",
    doc_id: DOC_ID_BYTES,
    acks: [{ push_id: decoded.ops[0]!.push_id, seq }],
  });
}

/** OpenDoc → DocOpened → PullOps → empty OpsBatch → Idle. (The
 *  `Hello` handshake is the host's, per socket — not the engine's.) */
function driveToIdle(eng: SyncEngine): void {
  eng.handleConnected();
  eng.popOutbox(); // OpenDoc
  eng.handleServerBytes(docOpenedBytes());
  eng.popOutbox(); // PullOps
  eng.handleServerBytes(emptyBatchCompleteBytes());
}

describe("engine ↔ EngineStorage (VV-derived web push path)", () => {
  test("capture → durable in-flight push → OpsAck → threshold fold", () => {
    const dek = Dek.generate();
    const storage = new MemEngineStorage();
    const doc = Doc.create();
    const engine = new SyncEngine(
      doc,
      DOC_ID,
      dek.clone(),
      0n,
      storage as unknown as EngineStorage,
    );
    engine.setLastLocalSeq(0);
    driveToIdle(engine);
    // Fresh doc: nothing pending, engine is Idle.
    if (!engine.isIdle()) throw new Error("expected engine idle after empty pull");

    // A local mutation, then capture it as one durable WAL row.
    engine.addItem(LIST_MAIN, "task one");
    const seq = engine.captureLocalOps();
    expect(seq).toBe(1);
    expect(storage.ops.length).toBe(1);
    // Nothing uncaptured in the doc now — the capture cursor advanced.
    expect(engine.hasUncapturedOps()).toBe(false);

    // Flush derives the delta from serverKnownVv, persists the
    // in-flight record BEFORE the frame is queued, and ships it.
    engine.flush();
    const frame = engine.popOutbox();
    expect(frame).toBeDefined();
    const decoded = decode(frame!) as {
      type: string;
      ops: { push_id: Uint8Array }[];
    };
    expect(decoded.type).toBe("PushOps");
    expect(decoded.ops.length).toBe(1);
    expect(storage.inFlight).not.toBeNull();
    expect(unhex(storage.inFlight!.pushId)).toEqual(
      Uint8Array.from(decoded.ops[0]!.push_id),
    );

    // Server assigns seq=1; the OpsAck (by pushId) clears the record
    // and persists the merged serverKnownVv.
    engine.handleServerBytes(ackBytesFor(frame!, 1));
    expect(storage.inFlight).toBeNull();
    expect(storage.serverKnownVv.length).toBeGreaterThan(0);
    expect(engine.lastContiguousSeq()).toBe(1n);

    // Nothing further to push.
    engine.flush();
    expect(engine.popOutbox()).toBeUndefined();

    // WAL row count 1 ≥ threshold 1 → fold; prunes the whole prefix.
    const wrote = engine.snapshotIfWalExceeds(1, Number.MAX_SAFE_INTEGER);
    expect(wrote).toBe(true);
    expect(storage.snapshot).not.toBeNull();
    expect(storage.snapshot!.upToLocalSeq).toBe(1);
    expect(storage.ops.length).toBe(0);

    // A second fold with nothing new is a no-op.
    expect(engine.snapshotIfWalExceeds(1, Number.MAX_SAFE_INTEGER)).toBe(false);
  });

  test("folding an UNSENT op still pushes it (VV-derived upload)", () => {
    const dek = Dek.generate();
    const storage = new MemEngineStorage();
    const doc = Doc.create();
    const engine = new SyncEngine(
      doc,
      DOC_ID,
      dek.clone(),
      0n,
      storage as unknown as EngineStorage,
    );
    engine.setLastLocalSeq(0);

    // Offline: mutate, capture, force-fold — the pending WAL row is
    // pruned into the snapshot.
    engine.addItem(LIST_MAIN, "offline work");
    engine.captureLocalOps();
    expect(engine.forceSnapshot()).toBe(true);
    expect(storage.ops.length).toBe(0);

    // Going online still derives the unsent op from Loro history.
    driveToIdle(engine);
    const frame = engine.popOutbox();
    expect(frame).toBeDefined();
    const decoded = decode(frame!) as { type: string };
    expect(decoded.type).toBe("PushOps");
  });

  test("remote ops are mirrored via appendRemoteWal with the advanced VV", () => {
    const dek = Dek.generate();

    // Scratch peer doc: produce an encrypted remote blob.
    const peer = Doc.create();
    peer.markPersisted();
    peer.addItem(LIST_MAIN, "from-peer");
    const blob = peer.pendingExport(dek)!;
    expect(blob).toBeDefined();

    const storage = new MemEngineStorage();
    const doc = Doc.empty();
    doc.markPersisted();
    const engine = new SyncEngine(
      doc,
      DOC_ID,
      dek.clone(),
      0n,
      storage as unknown as EngineStorage,
    );
    engine.setLastLocalSeq(0);
    driveToIdle(engine);

    engine.handleServerBytes(broadcastBytes(1, blob));

    expect(storage.ops.length).toBe(1);
    expect(storage.ops[0]!.serverSeq).toBe(1);
    // The VV write landed atomically with the row.
    expect(storage.serverKnownVv.length).toBeGreaterThan(0);
    expect(engine.lastContiguousSeq()).toBe(1n);

    // The remote op materialised into the doc, and nothing pends: the
    // remote history is server-known, so a flush ships nothing.
    const items = JSON.parse(engine.itemsInListJson(LIST_MAIN, false)) as Array<{ text: string }>;
    expect(items.map((i) => i.text)).toContain("from-peer");
    engine.flush();
    expect(engine.popOutbox()).toBeUndefined();
  });
});
