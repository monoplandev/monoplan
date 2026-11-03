// `MirrorBridge` over the in-memory host: open seeds an engine under
// the leased peer, the store-side flush pump reaches the host and
// converges it, host changes reach the engine with an `onServerFrame`
// pulse, updates arriving mid-subscribe are not lost, two docs route
// independently, and the host stopping ends routing.

import { describe, expect, test } from "bun:test";

import { Doc, SyncEngine } from "../wasm/monoplan_core_web.js";
import { MirrorBridge, type MirrorSeed } from "../src/mirror-bridge.ts";
import { FakeMirrorHost } from "./fake-mirror-host.ts";

const HOME = "00000000-0000-0000-0000-000000000001";
const OTHER = "00000000-0000-0000-0000-000000000002";
const LIST_INBOX = "inbox";

const createEngine = (seed: MirrorSeed): SyncEngine =>
  SyncEngine.mirror(Doc.loadWithPeer(seed.snapshot, seed.peer), seed.docId);

function texts(engine: { itemsInListJson(l: string, b: boolean): string }): string[] {
  return (JSON.parse(engine.itemsInListJson(LIST_INBOX, false)) as Array<{ text: string }>).map(
    (i) => i.text,
  );
}

function hostDoc(peer: bigint, first: string): Doc {
  const doc = Doc.createWithPeer(peer);
  doc.addItem(LIST_INBOX, first);
  return doc;
}

describe("MirrorBridge", () => {
  test("open seeds under the leased peer and both directions converge", async () => {
    const host = new FakeMirrorHost({ [HOME]: hostDoc(1n, "host-first") });
    const frames: string[] = [];
    const changes: string[] = [];
    const errors: string[] = [];
    const bridge = new MirrorBridge({
      transport: host,
      createEngine,
      onServerFrame: (id) => frames.push(id),
      onChange: (k) => changes.push(k),
      onError: (m) => errors.push(m),
    });
    const engine = await bridge.open(HOME);
    expect(engine.isMirror()).toBe(true);
    expect(bridge.docIds()).toEqual([HOME]);
    expect(frames).toEqual([HOME]);
    expect(changes).toEqual(["online"]);
    expect(texts(engine)).toEqual(["host-first"]);
    // Under the leased peer, not the host's.
    expect(host.subscribers.get(HOME)?.[0]?.peer).toBe(100n);
    expect(host.docs.get(HOME)!.peerId()).toBe(1n);

    // Mirror to host: what the store does on a commit.
    const mine = engine.addItem(LIST_INBOX, "from-the-window");
    engine.flush();
    bridge.pumpOutbox();
    await bridge.flushed();
    expect(host.imported).toHaveLength(1);
    const hd = host.docs.get(HOME)!;
    expect(JSON.parse(hd.getItemJson(mine) ?? "{}").text).toBe("from-the-window");
    expect(changes).toEqual(["online", "drain"]);
    // The fake echoed the delta back: harmless, and it pulsed.
    expect(frames).toEqual([HOME, HOME]);
    expect(hd.fingerprint()).toEqual(engine.fingerprint());

    // Host to mirror.
    host.commit(HOME, (d) => {
      d.addItem(LIST_INBOX, "from-the-cli");
    });
    expect(frames).toEqual([HOME, HOME, HOME]);
    expect(texts(engine)).toEqual(["host-first", "from-the-window", "from-the-cli"]);
    expect(hd.fingerprint()).toEqual(engine.fingerprint());
    // Host ops are not the mirror's to undo.
    expect(engine.undo()).toBe(true);
    expect(engine.getItemJson(mine)).toBeUndefined();
    expect(texts(engine)).toEqual(["host-first", "from-the-cli"]);
    engine.flush();
    bridge.pumpOutbox();
    await bridge.flushed();
    expect(hd.fingerprint()).toEqual(engine.fingerprint());
    expect(errors).toEqual([]);

    bridge.stop();
    expect(host.unsubscribed).toEqual([HOME]);
    expect(changes.at(-1)).toBe("offline");
    // After stop nothing moves either way.
    engine.addItem(LIST_INBOX, "late");
    engine.flush();
    bridge.pumpOutbox();
    await bridge.flushed();
    expect(host.imported).toHaveLength(2);
  });

  test("updates that land while the subscribe is in flight apply once the engine exists", async () => {
    const doc = hostDoc(1n, "host-first");
    const host = new FakeMirrorHost({ [HOME]: doc });
    // A transport whose subscribe resolves late, after a host commit.
    const gate: { release: (() => void) | null } = { release: null };
    const slow = {
      ...hostAsTransport(host),
      subscribe: async (docId: string) => {
        const seed = await host.subscribe(docId);
        await new Promise<void>((r) => {
          gate.release = r;
        });
        return seed;
      },
    };
    const frames: string[] = [];
    const bridge = new MirrorBridge({
      transport: slow,
      createEngine,
      onServerFrame: (id) => frames.push(id),
    });
    const opening = bridge.open(HOME);
    await Promise.resolve();
    // The host moved on after the snapshot was taken, before the
    // webview had an engine for it.
    host.commit(HOME, (d) => {
      d.addItem(LIST_INBOX, "between");
    });
    expect(frames).toEqual([]);
    gate.release!();
    const engine = await opening;
    expect(texts(engine)).toEqual(["host-first", "between"]);
    expect(frames).toEqual([HOME]);
  });

  test("two docs route independently and close releases one", async () => {
    const host = new FakeMirrorHost({
      [HOME]: hostDoc(1n, "home"),
      [OTHER]: hostDoc(2n, "other"),
    });
    const frames: string[] = [];
    const bridge = new MirrorBridge({
      transport: host,
      createEngine,
      onServerFrame: (id) => frames.push(id),
    });
    const home = await bridge.open(HOME);
    const other = await bridge.open(OTHER);
    expect(bridge.docIds()).toEqual([HOME, OTHER]);
    await expect(bridge.open(HOME)).rejects.toThrow(/already open/);

    host.commit(OTHER, (d) => {
      d.addItem(LIST_INBOX, "other-2");
    });
    expect(texts(other)).toEqual(["other", "other-2"]);
    expect(texts(home)).toEqual(["home"]);
    expect(frames).toEqual([HOME, OTHER, OTHER]);

    other.addItem(LIST_INBOX, "other-3");
    home.addItem(LIST_INBOX, "home-2");
    other.flush();
    home.flush();
    bridge.pumpOutbox();
    await bridge.flushed();
    expect(host.imported.map((i) => i.docId)).toEqual([HOME, OTHER]);
    expect(texts(host.docs.get(OTHER)!)).toEqual(["other", "other-2", "other-3"]);
    expect(texts(host.docs.get(HOME)!)).toEqual(["home", "home-2"]);

    // Close pumps what is left, then unsubscribes that doc only.
    other.addItem(LIST_INBOX, "other-last");
    other.flush();
    expect(bridge.close(OTHER)).toBe(true);
    expect(bridge.close(OTHER)).toBe(false);
    await bridge.flushed();
    expect(texts(host.docs.get(OTHER)!)).toContain("other-last");
    expect(host.unsubscribed).toEqual([OTHER]);
    expect(bridge.docIds()).toEqual([HOME]);
    host.commit(HOME, (d) => {
      d.addItem(LIST_INBOX, "home-3");
    });
    expect(texts(home)).toContain("home-3");
  });

  test("a stopped host ends routing and reports it; refused imports are errors", async () => {
    const host = new FakeMirrorHost({ [HOME]: hostDoc(1n, "home") });
    const stopped: string[] = [];
    const changes: string[] = [];
    const errors: string[] = [];
    const bridge = new MirrorBridge({
      transport: host,
      createEngine,
      onStopped: (id) => stopped.push(id),
      onChange: (k) => changes.push(k),
      onError: (m) => errors.push(m),
    });
    const engine = await bridge.open(HOME);

    host.refuseImports = "the host has stopped";
    engine.addItem(LIST_INBOX, "doomed");
    engine.flush();
    bridge.pumpOutbox();
    await bridge.flushed();
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/the host has stopped/);

    host.stop(HOME);
    expect(stopped).toEqual([HOME]);
    expect(bridge.docIds()).toEqual([]);
    expect(changes).toEqual(["online", "drain", "offline"]);
    // A stopped doc hears nothing more from the host; a late update for
    // an unrouted doc is reported, not applied.
    host.commit(HOME, (d) => {
      d.addItem(LIST_INBOX, "after");
    });
    expect(errors).toHaveLength(1);
    host.deliver(HOME, new Uint8Array([1, 2, 3]));
    expect(errors).toHaveLength(2);
    expect(errors[1]).toMatch(/no engine routes it/);
    expect(texts(engine)).toEqual(["home", "doomed"]);
    // Nothing to tear down twice.
    bridge.stop();
    expect(host.unsubscribed).toEqual([]);
  });
});

/** The host's transport methods bound, for spreading into a wrapper. */
function hostAsTransport(host: FakeMirrorHost) {
  return {
    subscribe: (id: string) => host.subscribe(id),
    unsubscribe: (id: string) => host.unsubscribe(id),
    send: (id: string, b: Uint8Array) => host.send(id, b),
    onUpdate: (cb: (id: string, b: Uint8Array) => void) => host.onUpdate(cb),
    onStopped: (cb: (id: string) => void) => host.onStopped(cb),
  };
}
