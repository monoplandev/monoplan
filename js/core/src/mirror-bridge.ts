// The mirror's IO (`spec/mac-window-plan.md` "JS side"): the twin of
// `SyncBridge` for a webview that runs over an in-process host instead
// of a WebSocket. Where `SyncBridge` multiplexes synced engines over one
// socket, this routes *mirror* engines (`SyncEngine.mirror`) over a
// `MirrorTransport`: three verbs and no envelope, because there is no
// network and no untrusted server between the two docs.
//
//   - `open(docId)` subscribes on the host, which answers with the seed
//     (a plaintext Loro snapshot and the peer id it leased for this
//     mirror); the caller-supplied `createEngine` turns the seed into a
//     mirror engine, which is then routed.
//   - host → mirror: every update the transport delivers is applied to
//     its doc's engine (`applyMirrorUpdate`, tagged remote) and
//     `onServerFrame(docId)` pulses so the host drains AppEvents into
//     its store, exactly as after a server frame.
//   - mirror → host: `pumpOutbox()` drains each engine's mirror outbox
//     (what `flush` exported) and hands every update to `send`, in
//     order. The store's `setOnFlush` hook calls this after each local
//     commit, as it calls `SyncBridge.pumpOutbox` in the browser.
//
// The transport is injected so the same bridge runs under the macOS
// app (a WebKit message handler, bytes as base64) and under Bun (an
// in-memory host). Like `SyncBridge`, this module imports no wasm at
// load time: the engine type is only used as a type.

import type { SyncEngine } from "../wasm/monoplan_core_web.js";
import type { ConnectionEvent } from "./sync-bridge.ts";

/** What the host answers a subscribe with (`Host.subscribe`): the doc
 *  as it stands, and the peer id the mirror must write under for the
 *  life of the subscription. */
export interface MirrorSeed {
  docId: string;
  /** Plaintext Loro snapshot. Updates continue from exactly here. */
  snapshot: Uint8Array;
  /** The leased Loro peer id (a u64; `BigInt` so no precision is lost). */
  peer: bigint;
}

/** The host as the bridge sees it. One implementation per platform. */
export interface MirrorTransport {
  /** Subscribe to `docId` on the host. Resolves with the seed; updates
   *  for the doc start arriving through `onUpdate` afterwards. */
  subscribe(docId: string): Promise<MirrorSeed>;
  /** End the subscription; the host releases the peer lease. */
  unsubscribe(docId: string): void;
  /** Hand one of the mirror's updates to the host (`Host.import`).
   *  Rejects when the host refused it (stopped, or bytes Loro would
   *  not import). Calls for one doc are made in order and awaited one
   *  at a time. */
  send(docId: string, bytes: Uint8Array): Promise<void>;
  /** Register the host → mirror delivery: an update for `docId`, in
   *  order, after its seed. Called once, at construction. */
  onUpdate(cb: (docId: string, bytes: Uint8Array) => void): void;
  /** Register the end-of-stream signal: the host stopped serving
   *  `docId` (the loop ended, or the doc was closed under the mirror).
   *  No update follows. Called once, at construction. */
  onStopped(cb: (docId: string) => void): void;
}

export interface MirrorBridgeOpts {
  transport: MirrorTransport;
  /** Turn a seed into a mirror engine: `Doc.loadWithPeer(snapshot,
   *  peer)` then `SyncEngine.mirror(doc, docId)`, plus whatever the
   *  host wants on the doc first (the author). Injected so this module
   *  picks no wasm build. */
  createEngine: (seed: MirrorSeed) => SyncEngine;
  /** `online` once the first doc is open; `offline` once the host has
   *  stopped every doc; `drain` after every outbox pump, the pulse the
   *  browser runtime reads as "we just round-tripped". */
  onChange?: (kind: ConnectionEvent) => void;
  /** Fires after every host update applied to an engine, and once
   *  right after a doc opens (the seed is the first frame). Hosts
   *  drain `popAppEvent` on the named engine into their store here. */
  onServerFrame?: (docId: string) => void;
  /** The host stopped serving `docId`; its engine is no longer routed
   *  and will not converge again until a new host seeds a new one. */
  onStopped?: (docId: string) => void;
  /** Problems with no owner: an update for a doc the bridge does not
   *  route, an import the host refused, bytes the engine would not
   *  apply. */
  onError?: (message: string) => void;
}

export class MirrorBridge {
  private readonly engines = new Map<string, SyncEngine>();
  /** Docs whose subscribe is in flight: updates that arrive before the
   *  engine exists wait here and apply once it does. */
  private readonly pending = new Map<string, Uint8Array[]>();
  /** Imports are serialised so the host sees a doc's updates in the
   *  order the mirror produced them. */
  private sendChain: Promise<void> = Promise.resolve();
  private stopped = false;
  private online = false;

  constructor(private readonly opts: MirrorBridgeOpts) {
    opts.transport.onUpdate((docId, bytes) => this.handleUpdate(docId, bytes));
    opts.transport.onStopped((docId) => this.handleStopped(docId));
  }

  /** Doc ids this bridge routes, in open order. */
  docIds(): string[] {
    return [...this.engines.keys()];
  }

  engine(docId: string): SyncEngine | undefined {
    return this.engines.get(docId);
  }

  /** Subscribe to `docId` on the host and route the resulting engine.
   *  Resolves with the engine once it is seeded; `onServerFrame`
   *  pulses once for it before that. Throws if the doc is already
   *  routed or the bridge is stopped. */
  async open(docId: string): Promise<SyncEngine> {
    if (this.stopped) throw new Error("MirrorBridge: stopped");
    if (this.engines.has(docId) || this.pending.has(docId)) {
      throw new Error(`MirrorBridge: doc ${docId} is already open`);
    }
    this.pending.set(docId, []);
    let seed: MirrorSeed;
    try {
      seed = await this.opts.transport.subscribe(docId);
    } catch (e) {
      this.pending.delete(docId);
      throw e;
    }
    const engine = this.opts.createEngine({ ...seed, docId });
    const queued = this.pending.get(docId) ?? [];
    this.pending.delete(docId);
    if (this.stopped) {
      // Stopped while subscribing: release the lease we just took.
      this.opts.transport.unsubscribe(docId);
      throw new Error("MirrorBridge: stopped");
    }
    this.engines.set(docId, engine);
    for (const bytes of queued) this.apply(docId, engine, bytes);
    this.opts.onServerFrame?.(docId);
    if (!this.online) {
      this.online = true;
      this.opts.onChange?.("online");
    }
    return engine;
  }

  /** Stop routing `docId` and end its subscription on the host. Any
   *  update still in the engine's outbox is pumped first. Returns
   *  whether the doc was routed. */
  close(docId: string): boolean {
    const engine = this.engines.get(docId);
    if (!engine) return false;
    this.pumpOne(docId, engine);
    this.engines.delete(docId);
    this.opts.transport.unsubscribe(docId);
    return true;
  }

  /** Caller signal: a local commit may have left bytes in some
   *  engine's mirror outbox. Drain every engine to the host. */
  pumpOutbox(): void {
    if (this.stopped) return;
    for (const [docId, engine] of this.engines) this.pumpOne(docId, engine);
    this.opts.onChange?.("drain");
  }

  /** Resolves once every update handed to `send` so far has been
   *  answered by the host (or refused). */
  flushed(): Promise<void> {
    return this.sendChain;
  }

  /** End every subscription. Engines keep working as plain docs but
   *  neither send nor receive from here on. */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    for (const docId of [...this.engines.keys()]) {
      this.engines.delete(docId);
      this.opts.transport.unsubscribe(docId);
    }
    this.pending.clear();
    if (this.online) {
      this.online = false;
      this.opts.onChange?.("offline");
    }
  }

  private pumpOne(docId: string, engine: SyncEngine): void {
    while (true) {
      const bytes = engine.takeMirrorOutbox();
      if (!bytes) break;
      this.sendChain = this.sendChain
        .then(() => this.opts.transport.send(docId, bytes))
        .catch((e) => {
          this.opts.onError?.(
            `import into the host failed for doc ${docId}: ${e instanceof Error ? e.message : String(e)}`,
          );
        });
    }
  }

  private handleUpdate(docId: string, bytes: Uint8Array): void {
    if (this.stopped) return;
    const engine = this.engines.get(docId);
    if (!engine) {
      const queue = this.pending.get(docId);
      if (queue) {
        queue.push(bytes);
        return;
      }
      this.opts.onError?.(`host update for doc ${docId} dropped: no engine routes it`);
      return;
    }
    this.apply(docId, engine, bytes);
    this.opts.onServerFrame?.(docId);
  }

  private apply(docId: string, engine: SyncEngine, bytes: Uint8Array): void {
    try {
      engine.applyMirrorUpdate(bytes);
    } catch (e) {
      this.opts.onError?.(
        `host update for doc ${docId} not applied: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
  }

  private handleStopped(docId: string): void {
    if (this.stopped) return;
    const had = this.engines.delete(docId) || this.pending.delete(docId);
    if (!had) return;
    this.opts.onStopped?.(docId);
    if (this.engines.size === 0 && this.pending.size === 0 && this.online) {
      this.online = false;
      this.opts.onChange?.("offline");
    }
  }
}
