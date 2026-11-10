// The mirror's IO (`spec/mac-window-plan.md` "JS side"): the twin of
// `SyncBridge` for a webview that runs over an in-process host instead
// of a WebSocket. Where `SyncBridge` multiplexes synced replicas over one
// socket, this routes *mirror* replicas (`Replica.mirror`) over a
// `MirrorTransport`: three verbs and no envelope, because there is no
// network and no untrusted server between the two docs.
//
//   - `open(docId)` subscribes on the host, which answers with the seed
//     (a plaintext Loro snapshot and the peer id it leased for this
//     mirror); the caller-supplied `createReplica` turns the seed into a
//     mirror replica, which is then routed.
//   - host → mirror: every update the transport delivers is applied to
//     its doc's replica (`applyMirrorUpdate`, tagged remote) and
//     `onServerFrame(docId)` pulses so the host drains AppEvents into
//     its store, exactly as after a server frame.
//   - mirror → host: `pumpOutbox()` drains each replica's mirror outbox
//     (what `flush` exported) and hands every update to `send`, in
//     order. The store's `setOnFlush` hook calls this after each local
//     commit, as it calls `SyncBridge.pumpOutbox` in the browser.
//
// The transport is injected so the same bridge runs under the macOS
// app (a WebKit message handler, bytes as base64) and under Bun (an
// in-memory host). Like `SyncBridge`, this module imports no wasm at
// load time: the replica type is only used as a type.

import type { Replica } from "../wasm/monoplan_core_web.js";
import type { ConnectionEvent } from "./sync-bridge.ts";

/** What the host answers a subscribe with (`Engine.subscribe`): the doc
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
  /** Hand one of the mirror's updates to the host (`Engine.import`).
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
  /** Turn a seed into a mirror replica: `Doc.loadWithPeer(snapshot,
   *  peer)` then `Replica.mirror(doc, docId)`, plus whatever the
   *  host wants on the doc first (the author). Injected so this module
   *  picks no wasm build. */
  createReplica: (seed: MirrorSeed) => Replica;
  /** `online` once the first doc is open; `offline` once the host has
   *  stopped every doc; `drain` after every outbox pump, the pulse the
   *  browser runtime reads as "we just round-tripped". */
  onChange?: (kind: ConnectionEvent) => void;
  /** Fires after every host update applied to a replica, and once
   *  right after a doc opens (the seed is the first frame). Hosts
   *  drain `popAppEvent` on the named replica into their store here. */
  onServerFrame?: (docId: string) => void;
  /** The host stopped serving `docId`; its replica is no longer routed
   *  and will not converge again until a new host seeds a new one. */
  onStopped?: (docId: string) => void;
  /** Problems with no owner: an update for a doc the bridge does not
   *  route, an import the host refused, bytes the replica would not
   *  apply. */
  onError?: (message: string) => void;
}

export class MirrorBridge {
  private readonly replicas = new Map<string, Replica>();
  /** Docs whose subscribe is in flight: updates that arrive before the
   *  replica exists wait here and apply once it does. */
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
    return [...this.replicas.keys()];
  }

  replica(docId: string): Replica | undefined {
    return this.replicas.get(docId);
  }

  /** Subscribe to `docId` on the host and route the resulting replica.
   *  Resolves with the replica once it is seeded; `onServerFrame`
   *  pulses once for it before that. Throws if the doc is already
   *  routed or the bridge is stopped. */
  async open(docId: string): Promise<Replica> {
    if (this.stopped) throw new Error("MirrorBridge: stopped");
    if (this.replicas.has(docId) || this.pending.has(docId)) {
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
    const replica = this.opts.createReplica({ ...seed, docId });
    const queued = this.pending.get(docId) ?? [];
    this.pending.delete(docId);
    if (this.stopped) {
      // Stopped while subscribing: release the lease we just took.
      this.opts.transport.unsubscribe(docId);
      throw new Error("MirrorBridge: stopped");
    }
    this.replicas.set(docId, replica);
    for (const bytes of queued) this.apply(docId, replica, bytes);
    this.opts.onServerFrame?.(docId);
    if (!this.online) {
      this.online = true;
      this.opts.onChange?.("online");
    }
    return replica;
  }

  /** Stop routing `docId` and end its subscription on the host. Any
   *  update still in the replica's outbox is pumped first. Returns
   *  whether the doc was routed. */
  close(docId: string): boolean {
    const replica = this.replicas.get(docId);
    if (!replica) return false;
    this.pumpOne(docId, replica);
    this.replicas.delete(docId);
    this.opts.transport.unsubscribe(docId);
    return true;
  }

  /** Caller signal: a local commit may have left bytes in some
   *  replica's mirror outbox. Drain every replica to the host. */
  pumpOutbox(): void {
    if (this.stopped) return;
    for (const [docId, replica] of this.replicas) this.pumpOne(docId, replica);
    this.opts.onChange?.("drain");
  }

  /** Resolves once every update handed to `send` so far has been
   *  answered by the host (or refused). */
  flushed(): Promise<void> {
    return this.sendChain;
  }

  /** End every subscription. Replicas keep working as plain docs but
   *  neither send nor receive from here on. */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    for (const docId of [...this.replicas.keys()]) {
      this.replicas.delete(docId);
      this.opts.transport.unsubscribe(docId);
    }
    this.pending.clear();
    if (this.online) {
      this.online = false;
      this.opts.onChange?.("offline");
    }
  }

  private pumpOne(docId: string, replica: Replica): void {
    while (true) {
      const bytes = replica.takeMirrorOutbox();
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
    const replica = this.replicas.get(docId);
    if (!replica) {
      const queue = this.pending.get(docId);
      if (queue) {
        queue.push(bytes);
        return;
      }
      this.opts.onError?.(`host update for doc ${docId} dropped: no replica routes it`);
      return;
    }
    this.apply(docId, replica, bytes);
    this.opts.onServerFrame?.(docId);
  }

  private apply(docId: string, replica: Replica, bytes: Uint8Array): void {
    try {
      replica.applyMirrorUpdate(bytes);
    } catch (e) {
      this.opts.onError?.(
        `host update for doc ${docId} not applied: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
  }

  private handleStopped(docId: string): void {
    if (this.stopped) return;
    const had = this.replicas.delete(docId) || this.pending.delete(docId);
    if (!had) return;
    this.opts.onStopped?.(docId);
    if (this.replicas.size === 0 && this.pending.size === 0 && this.online) {
      this.online = false;
      this.opts.onChange?.("offline");
    }
  }
}
