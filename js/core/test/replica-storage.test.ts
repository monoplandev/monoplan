// The web client drives the replica through a JS-implemented
// `ReplicaStorage` (`spec/local-storage.md`) — the in-memory mirror
// behind `IdbStorage`. This test exercises that wasm boundary —
// `captureLocalOps` / `snapshotIfWalExceeds` / the seed setters plus
// the `ReplicaStorage` extern — without IndexedDB or a server, using a
// hand-rolled in-memory mirror that mirrors `core::MemStorage`'s
// semantics. It locks in the contract the real `IdbStorage` satisfies
// (spec/vv-wal-separation.md):
//
//   - a captured local commit lands as a WAL row, and the push is
//     derived from `serverKnownVv` (a durable in-flight record with a
//     pushId, persisted before the send);
//   - an `UpdatesAck` naming that pushId clears the record and persists
//     the merged `serverKnownVv`;
//   - snapshot folding prunes the WAL prefix regardless of sync state
//     — unsent updates survive inside the full-history snapshot;
//   - remote updates applied from the wire are mirrored via
//     `appendRemoteWal` together with the advanced `serverKnownVv`.

import { describe, expect, test } from "bun:test";
import { decode, encode } from "@msgpack/msgpack";

import { Dek, Doc, EncryptedBlob, Replica } from "../wasm/monoplan_core_web.js";
import type { ReplicaStorage } from "../wasm/monoplan_core_web.js";
import { MemReplicaStorage, unhex } from "./mem-replica-storage.ts";

const LIST_MAIN = "inbox";
const DOC_ID = "00000000-0000-0000-0000-000000000000";
/** `DOC_ID` as the 16 raw bytes rmp_serde expects for a `Uuid`. */
const DOC_ID_BYTES = new Uint8Array(16);

function docOpenedBytes(): Uint8Array {
  return encode({ type: "DocOpened", doc_id: DOC_ID_BYTES, epoch: 0 });
}
function emptyBatchCompleteBytes(): Uint8Array {
  return encode({ type: "UpdatesBatch", doc_id: DOC_ID_BYTES, updates: [], complete: true });
}
function broadcastBytes(seq: number, blob: EncryptedBlob): Uint8Array {
  return encode({
    type: "UpdatesBroadcast",
    doc_id: DOC_ID_BYTES,
    updates: [{ seq, blob: { nonce: blob.nonce, ciphertext: blob.ciphertext } }],
  });
}
/** Build the UpdatesAck for a decoded PushUpdates frame at `seq`. */
function ackBytesFor(pushFrame: Uint8Array, seq: number): Uint8Array {
  const decoded = decode(pushFrame) as {
    type: string;
    updates: { push_id: Uint8Array; blob: unknown }[];
  };
  if (decoded.type !== "PushUpdates") throw new Error(`expected PushUpdates, got ${decoded.type}`);
  if (decoded.updates.length !== 1) throw new Error("replica ships one blob per push");
  return encode({
    type: "UpdatesAck",
    doc_id: DOC_ID_BYTES,
    acks: [{ push_id: decoded.updates[0]!.push_id, seq }],
  });
}

/** OpenDoc → DocOpened → PullUpdates → empty UpdatesBatch → Idle. (The
 *  `Hello` handshake is the host's, per socket — not the replica's.) */
function driveToIdle(eng: Replica): void {
  eng.handleConnected();
  eng.popOutbox(); // OpenDoc
  eng.handleServerBytes(docOpenedBytes());
  eng.popOutbox(); // PullUpdates
  eng.handleServerBytes(emptyBatchCompleteBytes());
}

describe("replica ↔ ReplicaStorage (VV-derived web push path)", () => {
  test("capture → durable in-flight push → UpdatesAck → threshold fold", () => {
    const dek = Dek.generate();
    const storage = new MemReplicaStorage();
    const doc = Doc.create();
    const replica = new Replica(
      doc,
      DOC_ID,
      dek.clone(),
      0n,
      storage as unknown as ReplicaStorage,
    );
    replica.setLastLocalSeq(0);
    driveToIdle(replica);
    // Fresh doc: nothing pending, replica is Idle.
    if (!replica.isIdle()) throw new Error("expected replica idle after empty pull");

    // A local mutation, then capture it as one durable WAL row.
    replica.addItem(LIST_MAIN, "task one");
    const seq = replica.captureLocalOps();
    expect(seq).toBe(1);
    expect(storage.ops.length).toBe(1);
    // Nothing uncaptured in the doc now — the capture cursor advanced.
    expect(replica.hasUncapturedOps()).toBe(false);

    // Flush derives the delta from serverKnownVv, persists the
    // in-flight record BEFORE the frame is queued, and ships it.
    replica.flush();
    const frame = replica.popOutbox();
    expect(frame).toBeDefined();
    const decoded = decode(frame!) as {
      type: string;
      updates: { push_id: Uint8Array }[];
    };
    expect(decoded.type).toBe("PushUpdates");
    expect(decoded.updates.length).toBe(1);
    expect(storage.inFlight).not.toBeNull();
    expect(unhex(storage.inFlight!.pushId)).toEqual(
      Uint8Array.from(decoded.updates[0]!.push_id),
    );

    // Server assigns seq=1; the UpdatesAck (by pushId) clears the record
    // and persists the merged serverKnownVv.
    replica.handleServerBytes(ackBytesFor(frame!, 1));
    expect(storage.inFlight).toBeNull();
    expect(storage.serverKnownVv.length).toBeGreaterThan(0);
    expect(replica.lastContiguousSeq()).toBe(1n);

    // Nothing further to push.
    replica.flush();
    expect(replica.popOutbox()).toBeUndefined();

    // WAL row count 1 ≥ threshold 1 → fold; prunes the whole prefix.
    const wrote = replica.snapshotIfWalExceeds(1, Number.MAX_SAFE_INTEGER);
    expect(wrote).toBe(true);
    expect(storage.snapshot).not.toBeNull();
    expect(storage.snapshot!.upToLocalSeq).toBe(1);
    expect(storage.ops.length).toBe(0);

    // A second fold with nothing new is a no-op.
    expect(replica.snapshotIfWalExceeds(1, Number.MAX_SAFE_INTEGER)).toBe(false);
  });

  test("folding an UNSENT op still pushes it (VV-derived upload)", () => {
    const dek = Dek.generate();
    const storage = new MemReplicaStorage();
    const doc = Doc.create();
    const replica = new Replica(
      doc,
      DOC_ID,
      dek.clone(),
      0n,
      storage as unknown as ReplicaStorage,
    );
    replica.setLastLocalSeq(0);

    // Offline: mutate, capture, force-fold — the pending WAL row is
    // pruned into the snapshot.
    replica.addItem(LIST_MAIN, "offline work");
    replica.captureLocalOps();
    expect(replica.forceSnapshot()).toBe(true);
    expect(storage.ops.length).toBe(0);

    // Going online still derives the unsent op from Loro history.
    driveToIdle(replica);
    const frame = replica.popOutbox();
    expect(frame).toBeDefined();
    const decoded = decode(frame!) as { type: string };
    expect(decoded.type).toBe("PushUpdates");
  });

  test("remote updates are mirrored via appendRemoteWal with the advanced VV", () => {
    const dek = Dek.generate();

    // Scratch peer doc: produce an encrypted remote blob.
    const peer = Doc.create();
    peer.markPersisted();
    peer.addItem(LIST_MAIN, "from-peer");
    const blob = peer.pendingExport(dek)!;
    expect(blob).toBeDefined();

    const storage = new MemReplicaStorage();
    const doc = Doc.empty();
    doc.markPersisted();
    const replica = new Replica(
      doc,
      DOC_ID,
      dek.clone(),
      0n,
      storage as unknown as ReplicaStorage,
    );
    replica.setLastLocalSeq(0);
    driveToIdle(replica);

    replica.handleServerBytes(broadcastBytes(1, blob));

    expect(storage.ops.length).toBe(1);
    expect(storage.ops[0]!.serverSeq).toBe(1);
    // The VV write landed atomically with the row.
    expect(storage.serverKnownVv.length).toBeGreaterThan(0);
    expect(replica.lastContiguousSeq()).toBe(1n);

    // The remote op materialised into the doc, and nothing pends: the
    // remote history is server-known, so a flush ships nothing.
    const items = JSON.parse(replica.itemsInListJson(LIST_MAIN, false)) as Array<{ text: string }>;
    expect(items.map((i) => i.text)).toContain("from-peer");
    replica.flush();
    expect(replica.popOutbox()).toBeUndefined();
  });
});
