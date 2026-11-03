// In-memory stand-in for the macOS app's host (`spec/mac-window-plan.md`
// "Host side"), for driving `MirrorBridge` and mirror engines under
// Bun. One wasm `Doc` per doc id plays the authoritative doc under peer
// slot 0; the fake does what `Session::subscribe` and the loop's
// `notify_if_changed` do, minus the WAL and the server:
//
//   - `subscribe` exports a plaintext snapshot and leases the next peer
//     slot (1, 2, ...), remembering the frontier it was sent;
//   - `send` imports the mirror's update as the host would
//     (`Doc::import_mirror_update`) and fans out;
//   - `commit` is a host-local change (a CLI command, a remote frame)
//     followed by the same fan-out;
//   - fan-out delivers `export(updates(last_sent))` to every subscriber
//     of that doc whose `last_sent` falls short, then advances it. The
//     real host also merges an importer's `last_sent` with what it just
//     imported so a mirror's own ops are not echoed back; this fake
//     deliberately echoes, to exercise that the echo is harmless.
//
// Delivery is synchronous from inside `send` / `commit`, which is
// stricter than the real host (whose sink runs on the loop's thread and
// never re-enters the webview synchronously) and so catches any
// re-entrancy the bridge might not tolerate.

import type { Doc } from "../wasm/monoplan_core_web.js";
import type { MirrorSeed, MirrorTransport } from "../src/mirror-bridge.ts";

interface Subscriber {
  docId: string;
  peer: bigint;
  lastSent: Uint8Array;
}

export class FakeMirrorHost implements MirrorTransport {
  readonly docs = new Map<string, Doc>();
  readonly subscribers = new Map<string, Subscriber[]>();
  /** Every update handed to `send`, in order, for assertions. */
  readonly imported: Array<{ docId: string; bytes: Uint8Array }> = [];
  /** Subscriptions ended by the bridge, in order. */
  readonly unsubscribed: string[] = [];
  /** Leased peers start well clear of the slots the test's host docs
   *  are minted under (1, 2, ...): two writers on one peer would mint
   *  duplicate op ids, which is the bug the lease exists to prevent. */
  private nextSlot = 100n;
  private updateCb: ((docId: string, bytes: Uint8Array) => void) | null = null;
  private stoppedCb: ((docId: string) => void) | null = null;
  /** When set, `send` rejects with it (a stopped host). */
  refuseImports: string | null = null;

  constructor(docs: Record<string, Doc> = {}) {
    for (const [id, doc] of Object.entries(docs)) this.docs.set(id, doc);
  }

  subscribe(docId: string): Promise<MirrorSeed> {
    const doc = this.docs.get(docId);
    if (!doc) return Promise.reject(new Error(`no such workspace: ${docId}`));
    const peer = this.nextSlot++;
    const sub: Subscriber = { docId, peer, lastSent: doc.oplogVvBytes() };
    const list = this.subscribers.get(docId) ?? [];
    list.push(sub);
    this.subscribers.set(docId, list);
    return Promise.resolve({ docId, snapshot: doc.exportSnapshot(), peer });
  }

  unsubscribe(docId: string): void {
    this.unsubscribed.push(docId);
    this.subscribers.delete(docId);
  }

  send(docId: string, bytes: Uint8Array): Promise<void> {
    if (this.refuseImports) return Promise.reject(new Error(this.refuseImports));
    const doc = this.docs.get(docId);
    if (!doc) return Promise.reject(new Error(`no such workspace: ${docId}`));
    this.imported.push({ docId, bytes: bytes.slice() });
    doc.importOplogUpdates(bytes);
    this.fanOut(docId);
    return Promise.resolve();
  }

  onUpdate(cb: (docId: string, bytes: Uint8Array) => void): void {
    this.updateCb = cb;
  }

  onStopped(cb: (docId: string) => void): void {
    this.stoppedCb = cb;
  }

  /** A host-local change: mutate the doc, then fan out like the loop
   *  does after a command or a remote frame. */
  commit(docId: string, mutate: (doc: Doc) => void): void {
    const doc = this.docs.get(docId);
    if (!doc) throw new Error(`no such workspace: ${docId}`);
    mutate(doc);
    this.fanOut(docId);
  }

  /** Push arbitrary bytes at the bridge as if the host had sent them:
   *  for the paths a well-behaved host never takes. */
  deliver(docId: string, bytes: Uint8Array): void {
    this.updateCb?.(docId, bytes);
  }

  /** The loop ended, or the doc closed under its mirrors. */
  stop(docId: string): void {
    this.subscribers.delete(docId);
    this.stoppedCb?.(docId);
  }

  private fanOut(docId: string): void {
    const doc = this.docs.get(docId);
    const subs = this.subscribers.get(docId);
    if (!doc || !subs) return;
    for (const sub of subs) {
      const bytes = doc.exportUpdatesAfter(sub.lastSent);
      if (bytes.length === 0) continue;
      sub.lastSent = doc.oplogVvBytes();
      this.updateCb?.(docId, bytes);
    }
  }
}
