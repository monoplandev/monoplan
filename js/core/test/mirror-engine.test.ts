// The wasm mirror mode (`spec/mac-window-plan.md` "Wasm side"): a
// `SyncEngine.mirror` over `Doc.loadWithPeer` holds a doc with no DEK,
// no storage and no socket. `flush` exports the doc's ops since the
// previous flush into the mirror outbox; `applyMirrorUpdate` imports
// the host's as remote; the WebSocket and persistence verbs are inert.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, SyncEngine } from "../wasm/monoplan_core_web.js";
import type { EngineStorage } from "../wasm/monoplan_core_web.js";
import { MemEngineStorage } from "./mem-engine-storage.ts";

const DOC_ID = "00000000-0000-0000-0000-000000000000";
const LIST_INBOX = "inbox";

function drainKinds(engine: SyncEngine): string[] {
  const out: string[] = [];
  while (true) {
    const ev = engine.popAppEvent();
    if (!ev) break;
    out.push(ev.kind);
  }
  return out;
}

function texts(json: string): string[] {
  return (JSON.parse(json) as Array<{ text: string }>).map((i) => i.text);
}

/** The host doc under slot 0 and a mirror engine seeded from it under
 *  the leased slot 1, as the app would build them. */
function seedMirror(): { host: Doc; engine: SyncEngine } {
  const host = Doc.createWithPeer(1n);
  host.addItem(LIST_INBOX, "host-first");
  const doc = Doc.loadWithPeer(host.exportSnapshot(), 2n);
  const engine = SyncEngine.mirror(doc, DOC_ID);
  return { host, engine };
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

describe("SyncEngine.mirror", () => {
  test("is a mirror with the doc id, and a synced engine is not", () => {
    const { engine } = seedMirror();
    expect(engine.isMirror()).toBe(true);
    expect(engine.docId()).toBe(DOC_ID);
    const synced = new SyncEngine(
      Doc.create(),
      DOC_ID,
      Dek.generate(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    expect(synced.isMirror()).toBe(false);
    expect(synced.takeMirrorOutbox()).toBeUndefined();
    expect(() => synced.applyMirrorUpdate(new Uint8Array([1, 2, 3]))).toThrow(
      /not a mirror/,
    );
  });

  test("flush exports the mirror's ops since the last flush; a second doc imports them to equality", () => {
    const { host, engine } = seedMirror();
    expect(engine.takeMirrorOutbox()).toBeUndefined();
    expect(engine.hasUnsyncedOps()).toBe(false);

    const id = engine.addItem(LIST_INBOX, "from-the-window");
    expect(engine.hasUnsyncedOps()).toBe(true);
    engine.flush();
    expect(engine.hasUnsyncedOps()).toBe(false);
    const first = engine.takeMirrorOutbox();
    expect(first).toBeInstanceOf(Uint8Array);
    expect(engine.takeMirrorOutbox()).toBeUndefined();

    // What the host does with it: import as local, converge.
    host.importOplogUpdates(first!);
    expect(JSON.parse(host.getItemJson(id) ?? "{}").text).toBe("from-the-window");
    expect(host.fingerprint()).toEqual(engine.fingerprint());

    // Nothing new: no bytes. Two commits before a flush: one update.
    engine.flush();
    expect(engine.takeMirrorOutbox()).toBeUndefined();
    engine.editItemText(id, "edited");
    engine.setItemDone(id, true);
    engine.flush();
    const second = engine.takeMirrorOutbox()!;
    expect(engine.takeMirrorOutbox()).toBeUndefined();
    host.importOplogUpdates(second);
    expect(host.fingerprint()).toEqual(engine.fingerprint());

    // Two flushes before a take queue two updates, oldest first.
    engine.addItem(LIST_INBOX, "a");
    engine.flush();
    engine.addItem(LIST_INBOX, "b");
    engine.flush();
    const a = engine.takeMirrorOutbox()!;
    const b = engine.takeMirrorOutbox()!;
    expect(engine.takeMirrorOutbox()).toBeUndefined();
    host.importOplogUpdates(a);
    host.importOplogUpdates(b);
    expect(host.fingerprint()).toEqual(engine.fingerprint());
  });

  test("applyMirrorUpdate emits the AppEvents a remote frame would and is idempotent", () => {
    const { host, engine } = seedMirror();
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
    const twinEngine = new SyncEngine(
      twin,
      DOC_ID,
      dek.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const expected = drainKinds(twinEngine);
    expect(expected).toContain("itemAdded");

    drainKinds(engine);
    engine.applyMirrorUpdate(update);
    expect(drainKinds(engine)).toEqual(expected);
    expect(JSON.parse(engine.getItemJson(hostItem) ?? "{}").text).toBe("from-the-host");
    // Not the mirror's own work: nothing to flush, nothing uncaptured.
    expect(engine.hasUnsyncedOps()).toBe(false);
    expect(engine.hasUncapturedOps()).toBe(false);
    engine.flush();
    expect(engine.takeMirrorOutbox()).toBeUndefined();

    // Again: no change, no events.
    engine.applyMirrorUpdate(update);
    expect(drainKinds(engine)).toEqual([]);
    expect(texts(engine.itemsInListJson(LIST_INBOX, false))).toEqual([
      "host-first",
      "from-the-host",
    ]);
  });

  test("host updates and the mirror's own echo never enter the mirror's undo", () => {
    const { host, engine } = seedMirror();
    const since = host.oplogVvBytes();
    const hostItem = host.addItem(LIST_INBOX, "from-the-host");
    engine.applyMirrorUpdate(host.exportUpdatesAfter(since));
    expect(engine.canUndo()).toBe(false);

    const mine = engine.addItem(LIST_INBOX, "mine");
    engine.flush();
    const ops = engine.takeMirrorOutbox()!;
    // The host imports and (a fake host that does not track ranges)
    // echoes the whole delta back.
    const hostSince = host.oplogVvBytes();
    host.importOplogUpdates(ops);
    drainKinds(engine);
    engine.applyMirrorUpdate(host.exportUpdatesAfter(hostSince));
    expect(drainKinds(engine)).toEqual([]);

    expect(engine.canUndo()).toBe(true);
    expect(engine.undo()).toBe(true);
    expect(engine.getItemJson(mine)).toBeUndefined();
    expect(engine.getItemJson(hostItem)).toBeDefined();
    expect(engine.canUndo()).toBe(false);
    // The undo is itself a local op the host must hear about.
    engine.flush();
    expect(engine.takeMirrorOutbox()).toBeDefined();
  });

  test("the WebSocket and persistence verbs are inert", () => {
    const { engine } = seedMirror();
    expect(engine.captureLocalOps()).toBeUndefined();
    expect(engine.snapshotIfWalExceeds(1, 1)).toBe(false);
    expect(engine.forceSnapshot()).toBe(false);
    engine.setLastLocalSeq(5);
    engine.seedWalStats(1, 1);
    engine.seedServerKnownVv(new Uint8Array());
    engine.handleConnected();
    engine.handleTimeout();
    engine.handleServerBytes(new Uint8Array([0xff]));
    engine.handleDisconnected();
    engine.notifyOplogDurable(3n);
    expect(engine.popOutbox()).toBeUndefined();
    expect(engine.popEvent()).toBeUndefined();
    expect(engine.isOnline()).toBe(false);
    expect(engine.isIdle()).toBe(true);
    expect(engine.lastContiguousSeq()).toBe(0n);
    expect(engine.lastDurableSeq()).toBe(0n);
    expect(engine.serverEpoch()).toBeUndefined();
    // The doc surface is the full one.
    expect(JSON.parse(engine.allListsJson())).toBeInstanceOf(Array);
    expect(engine.exportJson().length).toBeGreaterThan(0);
  });
});
