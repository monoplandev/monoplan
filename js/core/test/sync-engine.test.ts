// FFI smoke for the wasm SyncEngine surface.
//
// The deep state-machine coverage lives in `core/src/sync.rs` Rust
// tests; this file only verifies the wasm-bindgen contract holds:
// constructor consumes Doc/Dek, transport callbacks queue bytes,
// events come out as flat objects with the documented shape, doc
// mutations through the engine are observable, and a save/load round
// trip reconstructs an engine without losing state.
//
// "Stub WebSocket" here is the test itself: it pushes connect, drains
// the outbox, and asserts what the engine produced. The real protocol
// round trip is `sync-e2e.test.ts`.

import { describe, expect, test } from "bun:test";
import { decode, encode } from "@msgpack/msgpack";

import {
  Dek,
  Doc,
  helloFrame,
  parseHelloResponse,
  serverFrameDocId,
  SyncEngine,
} from "../wasm/monoplan_core_web.js";
import type { EngineStorage } from "../wasm/monoplan_core_web.js";
import { MemEngineStorage } from "./mem-engine-storage.ts";

const LIST_MAIN = "inbox";
const DOC_ID = "00000000-0000-0000-0000-000000000000";
const OTHER_DOC_ID = "11111111-2222-7333-8444-555555555555";

/** uuid string → the 16 raw bytes rmp_serde expects for a `Uuid`. */
function uuidBytes(id: string): Uint8Array {
  const hexStr = id.replace(/-/g, "");
  const out = new Uint8Array(16);
  for (let i = 0; i < 16; i++) out[i] = parseInt(hexStr.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function memStorage(): EngineStorage {
  return new MemEngineStorage() as unknown as EngineStorage;
}

/** Drain the engine's AppEvent queue into plain objects — the wasm
 *  getters are only readable while the event is alive. */
function drainAppEvents(eng: SyncEngine): Array<{
  kind: string;
  id: string;
  defaultView?: string;
  inboxView?: string;
}> {
  const out = [];
  while (true) {
    const ev = eng.popAppEvent();
    if (!ev) break;
    out.push({
      kind: ev.kind,
      id: ev.id,
      defaultView: ev.defaultView,
      inboxView: ev.inboxView,
    });
  }
  return out;
}

function newEngine(): SyncEngine {
  // Doc.create() builds a fresh doc with no commits (no seeded user
  // lists). Mutations through the engine are what put it in a
  // pending state.
  return new SyncEngine(Doc.create(), DOC_ID, Dek.generate(), 0n, memStorage());
}

describe("SyncEngine construction", () => {
  test("starts offline, non-idle, with a fresh frontier", () => {
    const eng = newEngine();
    expect(eng.isOnline()).toBe(false);
    expect(eng.isIdle()).toBe(false);
    expect(eng.lastContiguousSeq()).toBe(0n);
    expect(eng.docId()).toBe(DOC_ID);
  });
});

describe("connection-level handshake helpers", () => {
  test("helloFrame advertises the client and protocol version 1", () => {
    const hello = decode(helloFrame("monoplan-test", "9.9.9")) as {
      client: string;
      client_version: string;
      supported_protocol_versions: number[];
    };
    expect(hello.client).toBe("monoplan-test");
    expect(hello.client_version).toBe("9.9.9");
    expect(hello.supported_protocol_versions).toEqual([1]);
  });

  test("parseHelloResponse accepts a matching HelloAck only", () => {
    const ok = parseHelloResponse(
      encode({ server_version: "s", protocol_version: 1 }),
    );
    expect(ok.ok).toBe(true);
    expect(ok.serverVersion).toBe("s");
    expect(ok.error).toBeUndefined();

    const mismatch = parseHelloResponse(
      encode({ server_version: "s", protocol_version: 2 }),
    );
    expect(mismatch.ok).toBe(false);
    expect(mismatch.error).toContain("protocol 2");

    const rejected = parseHelloResponse(encode({ reason: "no overlap" }));
    expect(rejected.ok).toBe(false);
    expect(rejected.error).toContain("rejected: no overlap");

    const garbage = parseHelloResponse(new Uint8Array([0x00, 0x01]));
    expect(garbage.ok).toBe(false);
  });

  test("serverFrameDocId reads the routing key off payload frames only", () => {
    const frame = encode({
      type: "OpsAck",
      doc_id: uuidBytes(OTHER_DOC_ID),
      acks: [],
    });
    expect(serverFrameDocId(frame)).toBe(OTHER_DOC_ID);
    expect(
      serverFrameDocId(encode({ server_version: "s", protocol_version: 1 })),
    ).toBeUndefined();
    expect(serverFrameDocId(new Uint8Array([0x00]))).toBeUndefined();
  });
});

describe("transport callbacks", () => {
  test("handleConnected queues OpenDoc for its doc and emits online event", () => {
    const eng = newEngine();
    eng.handleConnected();

    expect(eng.isOnline()).toBe(true);

    const frame = eng.popOutbox();
    expect(frame).toBeInstanceOf(Uint8Array);
    const open = decode(frame!) as { type: string; doc_id: Uint8Array };
    expect(open.type).toBe("OpenDoc");
    expect(open.doc_id).toEqual(uuidBytes(DOC_ID));
    // Outbox is single-pop — second drain is empty.
    expect(eng.popOutbox()).toBeUndefined();

    const evt = eng.popEvent();
    expect(evt).toBeDefined();
    expect(evt!.kind).toBe("connStateChanged");
    expect(evt!.online).toBe(true);
    expect(evt!.seq).toBeUndefined();
    expect(evt!.message).toBeUndefined();
    expect(eng.popEvent()).toBeUndefined();
  });

  test("a frame for another doc is reported, not applied", () => {
    const eng = newEngine();
    eng.handleConnected();
    eng.popOutbox(); // discard OpenDoc
    eng.popEvent(); // discard online event

    eng.handleServerBytes(
      encode({ type: "DocOpened", doc_id: uuidBytes(OTHER_DOC_ID), epoch: 0 }),
    );
    const evt = eng.popEvent();
    expect(evt!.kind).toBe("error");
    expect(evt!.message).toContain(OTHER_DOC_ID);
    // Still waiting on its own DocOpened: no PullOps went out.
    expect(eng.popOutbox()).toBeUndefined();
    expect(eng.isIdle()).toBe(false);
  });

  test("handleDisconnected from Opening clears outbox and emits offline", () => {
    const eng = newEngine();
    eng.handleConnected();
    eng.popOutbox(); // discard OpenDoc
    eng.popEvent(); // discard online event

    eng.handleDisconnected();
    expect(eng.isOnline()).toBe(false);
    expect(eng.popOutbox()).toBeUndefined();

    const evt = eng.popEvent();
    expect(evt!.kind).toBe("connStateChanged");
    expect(evt!.online).toBe(false);
  });

  test("handleServerBytes while disconnected surfaces an error event", () => {
    const eng = newEngine();
    eng.handleServerBytes(new Uint8Array([0x00, 0x01]));
    const evt = eng.popEvent();
    expect(evt!.kind).toBe("error");
    expect(typeof evt!.message).toBe("string");
    expect(evt!.message!.length).toBeGreaterThan(0);
  });

  test("handleTimeout in Opening produces a handshake error", () => {
    const eng = newEngine();
    eng.handleConnected();
    eng.popOutbox();
    eng.popEvent();

    eng.handleTimeout();
    const evt = eng.popEvent();
    expect(evt!.kind).toBe("error");
    expect(evt!.message).toContain("timed out");
  });
});

describe("doc passthrough", () => {
  test("addItem through the engine is visible via itemsInListJson", () => {
    const eng = newEngine();
    const id = eng.addItem(LIST_MAIN, "buy milk");
    const items = JSON.parse(
      eng.itemsInListJson(LIST_MAIN, false),
    ) as Array<{ id: string; text: string }>;
    expect(items).toHaveLength(1);
    expect(items[0].id).toBe(id);
    expect(items[0].text).toBe("buy milk");
  });

  test("allListsJson on a fresh engine is empty; main is not a ListMeta", () => {
    const eng = newEngine();
    const lists = JSON.parse(eng.allListsJson()) as Array<{
      id: string;
      name: string;
    }>;
    // Per spec/data-model.md: `main` is a reserved id, not a
    // MovableList entry, and there are no seeded user lists.
    expect(lists).toHaveLength(0);
    expect(lists.some((l) => l.id === LIST_MAIN)).toBe(false);
  });

  test("hasUncapturedOps reflects uncaptured mutations", () => {
    const eng = newEngine();
    // Fresh doc has no commits → nothing uncaptured.
    expect(eng.hasUncapturedOps()).toBe(false);
    eng.addItem(LIST_MAIN, "later");
    expect(eng.hasUncapturedOps()).toBe(true);
  });

  test("flush before connect is a queued no-op (no outbox bytes)", () => {
    const eng = newEngine();
    eng.addItem(LIST_MAIN, "later");
    eng.flush();
    expect(eng.popOutbox()).toBeUndefined();
  });

  test("setDefaultView surfaces on the snapshot and as an event", () => {
    const eng = newEngine();
    const listId = eng.addList("Work");
    drainAppEvents(eng);

    eng.setDefaultView(listId, "board:backlog,todo,in_progress,review");
    const events = drainAppEvents(eng);
    const changed = events.find((e) => e.kind === "listDefaultViewChanged");
    expect(changed?.id).toBe(listId);
    expect(changed?.defaultView).toBe("board:backlog,todo,in_progress,review");

    const snap = JSON.parse(eng.workspaceSnapshotJson()) as {
      settings: { inboxView?: string };
      lists: Array<{ id: string; defaultView?: string }>;
    };
    expect(snap.lists.find((l) => l.id === listId)?.defaultView).toBe(
      "board:backlog,todo,in_progress,review",
    );

    // Clearing drops the key rather than persisting a "none" value.
    eng.setDefaultView(listId, "");
    expect(drainAppEvents(eng).at(-1)?.defaultView).toBeUndefined();
    const cleared = JSON.parse(eng.workspaceSnapshotJson()) as {
      lists: Array<{ id: string; defaultView?: string }>;
    };
    expect(cleared.lists.find((l) => l.id === listId)?.defaultView).toBeUndefined();
  });

  test("inbox's default view rides on settings, not a ListMeta row", () => {
    const eng = newEngine();
    drainAppEvents(eng);

    // The reserved list has no ListMeta row, so its default lands in the
    // doc-level settings map and reports via settingsChanged.
    eng.setDefaultView(LIST_MAIN, "board");
    const settingsEvent = drainAppEvents(eng).find(
      (e) => e.kind === "settingsChanged",
    );
    expect(settingsEvent?.inboxView).toBe("board");

    const snap = JSON.parse(eng.workspaceSnapshotJson()) as {
      settings: { inboxView?: string };
      lists: Array<{ id: string }>;
    };
    expect(snap.settings.inboxView).toBe("board");
    expect(snap.lists.some((l) => l.id === LIST_MAIN)).toBe(false);
  });

  test("setDefaultView rejects a view it can't encode", () => {
    const eng = newEngine();
    const listId = eng.addList("Work");
    // A typo must surface rather than silently clearing the default.
    expect(() => eng.setDefaultView(listId, "hologram")).toThrow();
  });
});

describe("save / load round trip", () => {
  test("engine.save() → Doc.load() → new SyncEngine preserves fingerprint", () => {
    const a = newEngine();
    a.addItem(LIST_MAIN, "persist me");
    const fingerprintBefore = a.fingerprint();
    const snapshot = a.save();

    // Same DEK so the reconstructed engine could in principle decrypt
    // remote ops; not exercised here, just locking down the API.
    const dek = Dek.generate();
    const restored = new SyncEngine(Doc.load(snapshot), DOC_ID, dek, 0n, memStorage());
    const fingerprintAfter = restored.fingerprint();

    expect(fingerprintAfter).toEqual(fingerprintBefore);
    const items = JSON.parse(
      restored.itemsInListJson(LIST_MAIN, false),
    ) as Array<{ text: string }>;
    expect(items.find((i) => i.text === "persist me")).toBeDefined();
  });
});
