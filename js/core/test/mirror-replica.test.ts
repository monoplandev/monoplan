// The wasm mirror mode (`spec/mac-window-plan.md` "Wasm side"): a
// `Replica.mirror` over `Doc.loadWithPeer` holds a doc with no DEK,
// no storage and no socket. `flush` exports the doc's ops since the
// previous flush into the mirror outbox; `applyMirrorUpdate` imports
// the host's as remote; the WebSocket and persistence verbs are inert.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, Replica } from "../wasm/monoplan_core_web.js";
import type { ReplicaStorage } from "../wasm/monoplan_core_web.js";
import { MemReplicaStorage } from "./mem-replica-storage.ts";

const DOC_ID = "00000000-0000-0000-0000-000000000000";
const LIST_INBOX = "inbox";

function drainKinds(replica: Replica): string[] {
  const out: string[] = [];
  while (true) {
    const ev = replica.popAppEvent();
    if (!ev) break;
    out.push(ev.kind);
  }
  return out;
}

function texts(json: string): string[] {
  return (JSON.parse(json) as Array<{ text: string }>).map((i) => i.text);
}

/** The host doc under slot 0 and a mirror replica seeded from it under
 *  the leased slot 1, as the app would build them. */
function seedMirror(): { host: Doc; replica: Replica } {
  const host = Doc.createWithPeer(1n);
  host.addItem(LIST_INBOX, "host-first");
  const doc = Doc.loadWithPeer(host.exportSnapshot(), 2n);
  const replica = Replica.mirror(doc, DOC_ID);
  return { host, replica };
}

describe("Doc.loadWithPeer", () => {
  test("loads the host's snapshot under the leased peer, as history", () => {
    const { host } = seedMirror();
    const doc = Doc.loadWithPeer(host.exportSnapshot(), 7n);
    expect(doc.peerId()).toBe(7n);
    expect(texts(doc.itemsInListJson(LIST_INBOX, false))).toEqual(["host-first"]);
    // Seeded ops are not uncaptured local work and not undo steps.
    expect(doc.hasUncapturedOps()).toBe(false);
    expect(doc.canUndo()).toBe(false);
  });

  test("rejects Loro's reserved peer", () => {
    const { host } = seedMirror();
    expect(() => Doc.loadWithPeer(host.exportSnapshot(), 2n ** 64n - 1n)).toThrow();
  });
});

describe("Replica.mirror", () => {
  test("is a mirror with the doc id, and a synced replica is not", () => {
    const { replica } = seedMirror();
    expect(replica.isMirror()).toBe(true);
    expect(replica.docId()).toBe(DOC_ID);
    const synced = new Replica(
      Doc.create(),
      DOC_ID,
      Dek.generate(),
      0n,
      new MemReplicaStorage() as unknown as ReplicaStorage,
    );
    expect(synced.isMirror()).toBe(false);
    expect(synced.takeMirrorOutbox()).toBeUndefined();
    expect(() => synced.applyMirrorUpdate(new Uint8Array([1, 2, 3]))).toThrow(
      /not a mirror/,
    );
  });

  test("flush exports the mirror's ops since the last flush; a second doc imports them to equality", () => {
    const { host, replica } = seedMirror();
    expect(replica.takeMirrorOutbox()).toBeUndefined();
    expect(replica.hasUnsyncedOps()).toBe(false);

    const id = replica.addItem(LIST_INBOX, "from-the-window");
    expect(replica.hasUnsyncedOps()).toBe(true);
    replica.flush();
    expect(replica.hasUnsyncedOps()).toBe(false);
    const first = replica.takeMirrorOutbox();
    expect(first).toBeInstanceOf(Uint8Array);
    expect(replica.takeMirrorOutbox()).toBeUndefined();

    // What the host does with it: import as local, converge.
    host.importOplogUpdates(first!);
    expect(JSON.parse(host.getItemJson(id) ?? "{}").text).toBe("from-the-window");
    expect(host.fingerprint()).toEqual(replica.fingerprint());

    // Nothing new: no bytes. Two commits before a flush: one update.
    replica.flush();
    expect(replica.takeMirrorOutbox()).toBeUndefined();
    replica.editItemText(id, "edited");
    replica.setItemDone(id, true);
    replica.flush();
    const second = replica.takeMirrorOutbox()!;
    expect(replica.takeMirrorOutbox()).toBeUndefined();
    host.importOplogUpdates(second);
    expect(host.fingerprint()).toEqual(replica.fingerprint());

    // Two flushes before a take queue two updates, oldest first.
    replica.addItem(LIST_INBOX, "a");
    replica.flush();
    replica.addItem(LIST_INBOX, "b");
    replica.flush();
    const a = replica.takeMirrorOutbox()!;
    const b = replica.takeMirrorOutbox()!;
    expect(replica.takeMirrorOutbox()).toBeUndefined();
    host.importOplogUpdates(a);
    host.importOplogUpdates(b);
    expect(host.fingerprint()).toEqual(replica.fingerprint());
  });

  test("applyMirrorUpdate emits the AppEvents a remote frame would and is idempotent", () => {
    const { host, replica } = seedMirror();
    const dek = Dek.generate();
    const baseline = host.exportSnapshot();
    host.markPersisted();
    const since = host.oplogVvBytes();
    const hostItem = host.addItem(LIST_INBOX, "from-the-host");
    const update = host.exportUpdatesAfter(since);
    // The same op as an encrypted server frame, applied to a twin of
    // the mirror the way the browser applies it, for the comparison.
    const blob = host.pendingExport(dek)!;
    const twin = Doc.loadWithPeer(baseline, 4n);
    twin.applyRemote(dek, blob);
    const twinReplica = new Replica(
      twin,
      DOC_ID,
      dek.clone(),
      0n,
      new MemReplicaStorage() as unknown as ReplicaStorage,
    );
    const expected = drainKinds(twinReplica);
    expect(expected).toContain("itemAdded");

    drainKinds(replica);
    replica.applyMirrorUpdate(update);
    expect(drainKinds(replica)).toEqual(expected);
    expect(JSON.parse(replica.getItemJson(hostItem) ?? "{}").text).toBe("from-the-host");
    // Not the mirror's own work: nothing to flush, nothing uncaptured.
    expect(replica.hasUnsyncedOps()).toBe(false);
    expect(replica.hasUncapturedOps()).toBe(false);
    replica.flush();
    expect(replica.takeMirrorOutbox()).toBeUndefined();

    // Again: no change, no events.
    replica.applyMirrorUpdate(update);
    expect(drainKinds(replica)).toEqual([]);
    expect(texts(replica.itemsInListJson(LIST_INBOX, false))).toEqual([
      "host-first",
      "from-the-host",
    ]);
  });

  test("host updates and the mirror's own echo never enter the mirror's undo", () => {
    const { host, replica } = seedMirror();
    const since = host.oplogVvBytes();
    const hostItem = host.addItem(LIST_INBOX, "from-the-host");
    replica.applyMirrorUpdate(host.exportUpdatesAfter(since));
    expect(replica.canUndo()).toBe(false);

    const mine = replica.addItem(LIST_INBOX, "mine");
    replica.flush();
    const ops = replica.takeMirrorOutbox()!;
    // The host imports and (a fake host that does not track ranges)
    // echoes the whole delta back.
    const hostSince = host.oplogVvBytes();
    host.importOplogUpdates(ops);
    drainKinds(replica);
    replica.applyMirrorUpdate(host.exportUpdatesAfter(hostSince));
    expect(drainKinds(replica)).toEqual([]);

    expect(replica.canUndo()).toBe(true);
    expect(replica.undo()).toBe(true);
    expect(replica.getItemJson(mine)).toBeUndefined();
    expect(replica.getItemJson(hostItem)).toBeDefined();
    expect(replica.canUndo()).toBe(false);
    // The undo is itself a local op the host must hear about.
    replica.flush();
    expect(replica.takeMirrorOutbox()).toBeDefined();
  });

  test("the WebSocket and persistence verbs are inert", () => {
    const { replica } = seedMirror();
    expect(replica.captureLocalOps()).toBeUndefined();
    expect(replica.snapshotIfWalExceeds(1, 1)).toBe(false);
    expect(replica.forceSnapshot()).toBe(false);
    replica.setLastLocalSeq(5);
    replica.seedWalStats(1, 1);
    replica.seedServerKnownVv(new Uint8Array());
    replica.handleConnected();
    replica.handleTimeout();
    replica.handleServerBytes(new Uint8Array([0xff]));
    replica.handleDisconnected();
    replica.notifyOplogDurable(3n);
    expect(replica.popOutbox()).toBeUndefined();
    expect(replica.popEvent()).toBeUndefined();
    expect(replica.isOnline()).toBe(false);
    expect(replica.isIdle()).toBe(true);
    expect(replica.lastContiguousSeq()).toBe(0n);
    expect(replica.lastDurableSeq()).toBe(0n);
    expect(replica.serverEpoch()).toBeUndefined();
    // The doc surface is the full one.
    expect(JSON.parse(replica.allListsJson())).toBeInstanceOf(Array);
    expect(replica.exportJson().length).toBeGreaterThan(0);
  });
});
