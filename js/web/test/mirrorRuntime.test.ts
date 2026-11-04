// The app window's runtime over the in-memory host: the store renders
// the host's docs, a store mutation reaches the host on the flush pump
// without any capture, a host change reaches the store through the
// bridge's frame pulse, the window's undo leaves host ops alone, and
// the WebKit transport frames bytes and peers the way the Swift side
// will. Run from `js/web` (the Solid server build).

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";

import { Doc, SyncEngine } from "@monoplan/core/wasm";
import { MirrorBridge, type MirrorSeed } from "@monoplan/core/mirror-bridge";
import { FakeMirrorHost } from "../../core/test/fake-mirror-host.ts";
import {
  createAppHost,
  decodeBytes,
  encodeBytes,
  HOST_GLOBAL,
  type HostInbound,
  type HostRequest,
} from "../src/sync/mirrorHost.ts";
import { createMirrorRuntime, sessionFromHost } from "../src/sync/mirrorRuntime.ts";

const HOME = "00000000-0000-0000-0000-000000000001";
const OTHER = "00000000-0000-0000-0000-000000000002";
const LIST_INBOX = "inbox";

const createEngine = (seed: MirrorSeed): SyncEngine =>
  SyncEngine.mirror(Doc.loadWithPeer(seed.snapshot, seed.peer), seed.docId);

function hostDoc(peer: bigint, first: string): Doc {
  const doc = Doc.createWithPeer(peer);
  doc.addItem(LIST_INBOX, first);
  return doc;
}

function hostTexts(doc: Doc): string[] {
  return (JSON.parse(doc.itemsInListJson(LIST_INBOX, false)) as Array<{ text: string }>).map(
    (i) => i.text,
  );
}

const info = {
  accountId: "acct-1",
  email: "a@example.com",
  deviceId: "dev-1",
  primaryDocId: HOME,
  serverUrl: "https://alpha.monoplan.app",
  docs: [
    { docId: HOME, name: null },
    { docId: OTHER, name: "Other" },
  ],
};

/** What `MirrorApp` does before `createMirrorRuntime`: open every doc
 *  through the bridge, Home first, binding the frame pulse later. */
async function bootRuntime(host: FakeMirrorHost) {
  let drainFor: (docId: string) => void = () => {};
  const bridge = new MirrorBridge({
    transport: host,
    createEngine,
    onServerFrame: (id) => drainFor(id),
  });
  const engines = [await bridge.open(HOME), await bridge.open(OTHER)];
  const runtime = createRoot(() =>
    createMirrorRuntime({
      session: sessionFromHost(info),
      engines,
      bridge,
      prefs: null,
    }),
  );
  drainFor = runtime.drain;
  return { bridge, runtime };
}

describe("createMirrorRuntime", () => {
  test("renders the host's docs and converges both ways through the store", async () => {
    const host = new FakeMirrorHost({
      [HOME]: hostDoc(1n, "home-first"),
      [OTHER]: hostDoc(2n, "other-first"),
    });
    const { bridge, runtime } = await bootRuntime(host);
    const { app, docs } = runtime;
    expect(app.docId).toBe(HOME);
    expect(docs.docs().map((d) => d.docId)).toEqual([HOME, OTHER]);
    expect(app.state.listOpen[LIST_INBOX]).toHaveLength(1);
    const first = app.state.listOpen[LIST_INBOX]![0]!;
    expect(docs.appOf(OTHER)!.state.listOpen[LIST_INBOX]).toHaveLength(1);
    expect(runtime.view()).toEqual({ kind: "list", id: "inbox" });

    // Store mutation → flush → the bridge pump → the host, with no
    // capture hook wired (there is no WAL in the window).
    const mine = app.addItem(LIST_INBOX, "from-the-window");
    expect(app.state.itemsById[mine]?.text).toBe("from-the-window");
    await bridge.flushed();
    const home = host.docs.get(HOME)!;
    expect(hostTexts(home)).toEqual(["home-first", "from-the-window"]);
    expect(home.fingerprint()).toEqual(app.engine.fingerprint());
    // The fake echoed the update; the store is unchanged by it.
    expect(app.state.listOpen[LIST_INBOX]).toEqual([first, mine]);
    expect(app.engine.hasUnsyncedOps()).toBe(false);

    // Host change → the bridge's frame pulse → the store.
    let cliItem = "";
    host.commit(HOME, (d) => {
      cliItem = d.addItem(LIST_INBOX, "from-the-cli");
    });
    expect(app.state.itemsById[cliItem]?.text).toBe("from-the-cli");
    expect(app.state.listOpen[LIST_INBOX]).toEqual([first, mine, cliItem]);
    // Routed by doc: the other doc did not move.
    expect(docs.appOf(OTHER)!.state.listOpen[LIST_INBOX]).toHaveLength(1);

    // The window's undo is its own: it reverses the window's add only,
    // and that reversal reaches the host.
    expect(app.undo()).toBe(true);
    expect(app.state.itemsById[mine]).toBeUndefined();
    expect(app.state.itemsById[cliItem]?.text).toBe("from-the-cli");
    await bridge.flushed();
    expect(hostTexts(home)).toEqual(["home-first", "from-the-cli"]);
    expect(home.fingerprint()).toEqual(app.engine.fingerprint());

    // The other doc's store routes to the other host doc.
    const other = docs.appOf(OTHER)!;
    other.addItem(LIST_INBOX, "other-window");
    await bridge.flushed();
    expect(hostTexts(host.docs.get(OTHER)!)).toEqual(["other-first", "other-window"]);
    expect(hostTexts(home)).toEqual(["home-first", "from-the-cli"]);

    // The DEK-touching flows are unavailable, not silent.
    await expect(runtime.createDoc("x")).rejects.toThrow(/not available/);
    await expect(runtime.deleteDoc(OTHER)).rejects.toThrow(/not available/);
    expect(runtime.presence(HOME)).toEqual([]);
  });

  test("a doc the host adds after boot opens live and routes both ways", async () => {
    // The host holds a doc the page did not boot with (New Workspace…
    // created it after the session reply); `docAdded` from the app
    // opens it in place.
    const host = new FakeMirrorHost({
      [HOME]: hostDoc(1n, "home-first"),
      [OTHER]: hostDoc(2n, "other-first"),
    });
    let drainFor: (docId: string) => void = () => {};
    const bridge = new MirrorBridge({
      transport: host,
      createEngine,
      onServerFrame: (id) => drainFor(id),
    });
    const engines = [await bridge.open(HOME)];
    const runtime = createRoot(() =>
      createMirrorRuntime({
        session: sessionFromHost({ ...info, docs: [{ docId: HOME, name: null }] }),
        engines,
        bridge,
        prefs: null,
      }),
    );
    drainFor = runtime.drain;
    expect(runtime.docs.docs().map((d) => d.docId)).toEqual([HOME]);

    expect(await runtime.openDoc(OTHER)).toBe(OTHER);
    expect(runtime.docs.docs().map((d) => d.docId)).toEqual([HOME, OTHER]);
    const other = runtime.docs.appOf(OTHER)!;
    expect(other.state.listOpen[LIST_INBOX]).toHaveLength(1);
    // Window → host on the shared pump, and host → store on the pulse.
    other.addItem(LIST_INBOX, "other-window");
    await bridge.flushed();
    expect(hostTexts(host.docs.get(OTHER)!)).toEqual(["other-first", "other-window"]);
    let cliItem = "";
    host.commit(OTHER, (d) => {
      cliItem = d.addItem(LIST_INBOX, "other-cli");
    });
    expect(other.state.itemsById[cliItem]?.text).toBe("other-cli");
    // Already open: nothing happens, and the host sees no new subscriber.
    const subscribed = host.subscribers.get(OTHER)!.length;
    expect(await runtime.openDoc(OTHER)).toBe(OTHER);
    expect(host.subscribers.get(OTHER)!.length).toBe(subscribed);
    // Hand-off covers the late doc too.
    expect(engines).toHaveLength(2);
  });

  test("notes typed on the idle timer reach the host on hand-off", async () => {
    const host = new FakeMirrorHost({
      [HOME]: hostDoc(1n, "home-first"),
      [OTHER]: hostDoc(2n, "other-first"),
    });
    const { bridge, runtime } = await bootRuntime(host);
    const app = runtime.app;
    const id = app.addItem(LIST_INBOX, "note me");
    await bridge.flushed();
    app.subscribeNotes(id);
    app.applyNotesDelta(id, [{ insert: "hey" }]);
    expect(app.state.itemsById[id]?.notes).toBe("hey");
    // Committed locally, still on the notes idle timer: not at the host.
    await bridge.flushed();
    const home = host.docs.get(HOME)!;
    expect(JSON.parse(home.getItemJson(id) ?? "{}").notes ?? "").toBe("");
    // The window going away flushes it through.
    runtime.handOff();
    await bridge.flushed();
    expect(JSON.parse(home.getItemJson(id) ?? "{}").notes).toBe("hey");
    app.unsubscribeNotes(id);
  });

  test("a host update queued before the store existed is not dispatched twice", async () => {
    const host = new FakeMirrorHost({
      [HOME]: hostDoc(1n, "home-first"),
      [OTHER]: hostDoc(2n, "other-first"),
    });
    let drainFor: (docId: string) => void = () => {};
    const bridge = new MirrorBridge({
      transport: host,
      createEngine,
      onServerFrame: (id) => drainFor(id),
    });
    const homeEngine = await bridge.open(HOME);
    // Lands on the engine while the second doc is still opening.
    let between = "";
    host.commit(HOME, (d) => {
      between = d.addItem(LIST_INBOX, "between");
    });
    const otherEngine = await bridge.open(OTHER);
    const runtime = createRoot(() =>
      createMirrorRuntime({
        session: sessionFromHost(info),
        engines: [homeEngine, otherEngine],
        bridge,
        prefs: null,
      }),
    );
    drainFor = runtime.drain;
    expect(runtime.app.state.listOpen[LIST_INBOX]).toHaveLength(2);
    expect(runtime.app.state.itemsById[between]?.text).toBe("between");
    // A later frame dispatches normally and does not resurrect the
    // earlier one as a duplicate.
    host.commit(HOME, (d) => {
      d.addItem(LIST_INBOX, "after");
    });
    expect(runtime.app.state.listOpen[LIST_INBOX]).toHaveLength(3);
  });
});

describe("the WebKit host transport", () => {
  test("frames requests and replies as the Swift side will", async () => {
    const doc = hostDoc(1n, "home-first");
    const requests: HostRequest[] = [];
    const imported: Uint8Array[] = [];
    const handler = {
      async postMessage(request: HostRequest): Promise<unknown> {
        requests.push(request);
        switch (request.kind) {
          case "session":
            return info;
          case "subscribe":
            return { snapshot: encodeBytes(doc.exportSnapshot()), peer: "18446744073709551614" };
          case "import":
            imported.push(decodeBytes(request.bytes));
            return null;
          case "unsubscribe":
            return null;
        }
      },
    };
    const target: Record<string, unknown> = {};
    const appHost = createAppHost(handler, target);
    expect(await appHost.session()).toEqual(info);

    const seed = await appHost.transport.subscribe(HOME);
    expect(seed.docId).toBe(HOME);
    // A u64 beyond Number's precision survives as a BigInt.
    expect(seed.peer).toBe(18446744073709551614n);
    const mirror = Doc.loadWithPeer(seed.snapshot, 5n);
    expect(hostTexts(mirror)).toEqual(["home-first"]);

    const since = mirror.oplogVvBytes();
    mirror.addItem(LIST_INBOX, "x");
    const bytes = mirror.exportUpdatesAfter(since);
    await appHost.transport.send(HOME, bytes);
    expect(imported).toHaveLength(1);
    expect(imported[0]).toEqual(bytes);
    doc.importOplogUpdates(imported[0]!);
    expect(hostTexts(doc)).toEqual(["home-first", "x"]);

    // Inbound: the global the app calls, bytes as base64.
    const updates: Array<{ docId: string; bytes: Uint8Array }> = [];
    const stopped: string[] = [];
    appHost.transport.onUpdate((docId, b) => updates.push({ docId, bytes: b }));
    appHost.transport.onStopped((docId) => stopped.push(docId));
    const inbound = target[HOST_GLOBAL] as HostInbound;
    inbound.update(HOME, encodeBytes(new Uint8Array([1, 2, 3])));
    inbound.stopped(HOME);
    expect(updates).toEqual([{ docId: HOME, bytes: new Uint8Array([1, 2, 3]) }]);
    expect(stopped).toEqual([HOME]);

    // The host's sync status: a push before anyone listens is replayed
    // to the first listener (the app sends it as soon as the page is
    // up, before the runtime exists), later ones arrive live.
    inbound.sync({ online: false, lastSyncedAt: null, pending: false });
    const statuses: Array<{ online: boolean; lastSyncedAt: number | null; pending: boolean }> = [];
    appHost.onSync((st) => statuses.push(st));
    inbound.sync({ online: true, lastSyncedAt: 1700000000000, pending: true });
    expect(statuses).toEqual([
      { online: false, lastSyncedAt: null, pending: false },
      { online: true, lastSyncedAt: 1700000000000, pending: true },
    ]);

    // Hand-off: resolved at once until the runtime binds its own, then
    // that, awaited.
    await inbound.handOff();
    let handedOff = 0;
    appHost.onHandOff(async () => {
      handedOff++;
    });
    await inbound.handOff();
    expect(handedOff).toBe(1);

    // A doc the host added after boot (New Workspace…): dropped until
    // the runtime listens, then delivered by id.
    inbound.docAdded(OTHER);
    const added: string[] = [];
    appHost.onDocAdded((id) => added.push(id));
    inbound.docAdded(OTHER);
    expect(added).toEqual([OTHER]);

    appHost.transport.unsubscribe(HOME);
    await Promise.resolve();
    expect(requests.map((r) => r.kind)).toEqual(["session", "subscribe", "import", "unsubscribe"]);
  });

  test("base64 round-trips large buffers", () => {
    const big = new Uint8Array(200_000);
    for (let i = 0; i < big.length; i++) big[i] = (i * 31) & 0xff;
    expect(decodeBytes(encodeBytes(big))).toEqual(big);
    expect(decodeBytes(encodeBytes(new Uint8Array()))).toEqual(new Uint8Array());
  });
});
