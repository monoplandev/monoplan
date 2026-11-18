// The stable-peer wasm boundary (`spec/peer-id-plan.md`): the
// `*WithPeer` constructors bind the doc's stored peer before anything
// else touches the doc — before `create`'s builtin seed commits and
// before the replay import — so every op a device mints in the doc
// carries that peer, and a reload resumes the peer's counter instead of restarting
// it (a restart would mint duplicate `(peer, counter)` ids that a
// replica silently drops). Single-ownership of the peer is the JS
// host's job (the single-tab Web Lock); these tests cover the doc
// behavior under correct ownership.

import { describe, expect, test } from "bun:test";

import { Dek, Doc } from "../wasm/monoplan_core_web.js";

const LIST_MAIN = "inbox";
const PEER = 7n;

/** One "session": everything this doc committed, as plaintext update
 *  bytes a later boot replays. */
function exportSession(doc: Doc, dek: Dek): Uint8Array {
  const blob = doc.pendingExport(dek);
  if (!blob) throw new Error("expected pending ops");
  return dek.open(blob);
}

describe("peer id boundary", () => {
  test("createWithPeer binds the peer; loro's reserved peer is rejected", () => {
    const doc = Doc.createWithPeer(PEER);
    expect(doc.peerId()).toBe(PEER);
    expect(Doc.emptyWithPeer(9n).peerId()).toBe(9n);
    // Random-peer constructors are unaffected.
    expect(Doc.create().peerId()).not.toBe(PEER);
    expect(() => Doc.createWithPeer(0xffffffffffffffffn)).toThrow();
    expect(() => Doc.emptyWithPeer(0xffffffffffffffffn)).toThrow();
  });

  test("reload under the same peer resumes the counter — no colliding op ids", () => {
    const dek = Dek.generate();

    // Session 1: fresh signup — seed commits + one user item, all
    // minted under the doc's stored peer.
    const s1 = Doc.createWithPeer(PEER);
    s1.addItem(LIST_MAIN, "from session 1");
    const wal1 = exportSession(s1, dek);

    // Session 2: reload — peer bound before the replay import, then a
    // new commit that must continue the counter where session 1 left
    // off.
    const s2 = Doc.emptyWithPeer(PEER);
    s2.replayOplogUpdate(wal1);
    s2.finishOplogReplay();
    s2.markPersisted();
    expect(s2.peerId()).toBe(PEER);
    s2.addItem(LIST_MAIN, "from session 2");
    const wal2 = exportSession(s2, dek);

    // A replica that imports both sessions sees both items. If session
    // 2 had restarted the counter, its ops would collide with session
    // 1's ids and be dropped on import.
    const replica = Doc.empty();
    replica.replayOplogUpdate(wal1);
    replica.replayOplogUpdate(wal2);
    replica.finishOplogReplay();
    const texts = (
      JSON.parse(replica.itemsInListJson(LIST_MAIN, false)) as { text: string }[]
    ).map((i) => i.text);
    expect(texts).toContain("from session 1");
    expect(texts).toContain("from session 2");
  });
});
