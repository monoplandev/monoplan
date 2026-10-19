// WebSocket pump + lifecycle around a set of sans-IO `SyncEngine`s —
// one per open doc, multiplexed over one socket. Owned by the JS host
// (not the wasm core) so each platform — web and bun-based tests — can
// drive the same engines with the same reconnect / auth policy by
// injecting only the bits that differ:
//
//   - `socketFactory` builds a fresh `WebSocket` per attempt (cookie
//     auth on the browser, bearer header in Bun, etc.).
//   - `probeAuth` disambiguates a pre-open close: a returned `false`
//     means the server has confirmed our credentials are bad (HTTP
//     401), and the bridge stops permanently; anything else (success,
//     network error, non-auth HTTP error) reconnects.
//   - `codec` is the wasm handshake surface (`helloFrame`,
//     `parseHelloResponse`, `serverFrameDocId`). Injected rather than
//     imported so this module stays wasm-free at load time and the
//     host picks the right wasm build (bundler vs nodejs).
//
// Connection vs doc: the `Hello` / `HelloAck` version handshake is per
// socket and runs here, once, before any engine goes online. Each
// engine then binds its own doc (`OpenDoc`) and runs its own state
// machine; inbound frames are routed to the engine whose `docId()`
// matches the frame's `doc_id`. Engines never see each other's frames
// and never see the handshake.
//
// The engines themselves stay sans-IO per `spec/architecture.md`; this
// file is what makes the IO testable from Bun instead of only from
// a browser.
//
// Reconnect uses exponential backoff with jitter, capped at `maxMs`.
// The attempt counter resets on a successful handshake. Hosts can
// short-circuit the backoff timer with `reconnectNow()` — that's the
// hook platform layers wire to visibility/online events.

import type {
  EngineEvent,
  HelloOutcome,
  SyncEngine,
} from "../wasm/monoplan_core_web.js";

export type ConnectionEvent = "online" | "offline" | "drain";

export interface ReconnectBackoff {
  /** Delay before the first retry. Subsequent retries double until
   *  capped at `maxMs`. Default 500ms. */
  baseMs?: number;
  /** Cap on the exponentially-grown delay. Default 30_000ms. */
  maxMs?: number;
  /** Jitter as a fraction of the computed delay; 0.2 = ±20%.
   *  Default 0.2. Set to 0 for deterministic tests. */
  jitter?: number;
}

/** The connection-level wasm helpers the bridge needs. Pass the
 *  functions exported by the wasm module the host loaded. */
export interface HandshakeCodec {
  helloFrame(clientName: string, clientVersion: string): Uint8Array;
  parseHelloResponse(bytes: Uint8Array): HelloOutcome;
  serverFrameDocId(bytes: Uint8Array): string | undefined;
}

export interface SyncBridgeOpts {
  /** One engine per doc this device has open on the connection. All
   *  are brought online together after the handshake; a frame for a
   *  doc none of them syncs is dropped and reported via `onError`. */
  engines: SyncEngine[];
  codec: HandshakeCodec;
  /** Identity advertised in `Hello`. */
  clientName: string;
  clientVersion: string;
  /** Build a fresh socket. Called on every (re)connect, so callers
   *  can pick up rotated auth (cookie, header, ticket) without
   *  rebuilding the bridge. */
  socketFactory: () => WebSocket;
  /** Disambiguate a pre-open close: did the server reject auth, or
   *  was this a plain network drop? Resolve `false` only if the
   *  server confirms our credentials are bad (typically HTTP 401 on
   *  a known-authed endpoint). Any other outcome → resolve `true`
   *  and the bridge reconnects. Without a probe, every close
   *  reconnects — appropriate for tests where the only reason a
   *  pre-open close happens is that the server was killed. */
  probeAuth?: () => Promise<boolean>;
  /** Connection-state flips. `online` fires once the handshake has
   *  succeeded and every engine has been brought online; `drain`
   *  fires after every outbox flush (server-triggered or host-
   *  triggered via `pumpOutbox`) — the natural pulse for "we just
   *  round-tripped with the server". */
  onChange?: (kind: ConnectionEvent) => void;
  /** Fires after every server frame the bridge feeds to an engine
   *  (post-pump), with that engine's doc id. Hosts drain
   *  `popAppEvent` on the named engine and dispatch to their UI store
   *  here. Also fires once per engine right after the handshake. */
  onServerFrame?: (docId: string) => void;
  /** `probeAuth` resolved `false`. The bridge has stopped and will
   *  not reconnect; the host should tear down the session. */
  onAuthFailed?: () => void;
  /** Surface protocol-level engine events (`popEvent`) — errors,
   *  protocol-version warnings, etc. — tagged with the doc. If unset,
   *  events are still drained (so the engine queues can't grow
   *  unbounded) but discarded. */
  onEngineEvent?: (ev: EngineEvent, docId: string) => void;
  /** Connection-level problems the engines never see: a rejected or
   *  malformed handshake, a timed-out handshake, a frame for a doc no
   *  engine syncs. Informational — the bridge has already decided
   *  what to do (drop the frame, or close and reconnect). */
  onError?: (message: string) => void;
  /** Exponential-backoff parameters. Defaults: base 500ms, cap 30s,
   *  ±20% jitter. */
  reconnectBackoff?: ReconnectBackoff;
  /** How long to wait for the server's `HelloAck` before closing the
   *  socket and reconnecting. Default 10s. */
  handshakeTimeoutMs?: number;
}

const DEFAULT_BASE_MS = 500;
const DEFAULT_MAX_MS = 30_000;
const DEFAULT_JITTER = 0.2;
const DEFAULT_HANDSHAKE_TIMEOUT_MS = 10_000;
/** Cadence at which the bridge calls `engine.handleTimeout()` on every
 *  engine to drive the `OpenDoc` watchdog. 1s polling keeps wake-up
 *  cost negligible. */
const TICK_INTERVAL_MS = 1_000;

export class SyncBridge {
  private ws: WebSocket | null = null;
  /** `hello` from socket open until `HelloAck`; `open` after. */
  private phase: "hello" | "open" = "hello";
  private timer: ReturnType<typeof setTimeout> | null = null;
  private handshakeTimer: ReturnType<typeof setTimeout> | null = null;
  private tickTimer: ReturnType<typeof setInterval> | null = null;
  private stopped = false;
  private attempt = 0;
  private readonly engines: Map<string, SyncEngine>;
  private readonly baseMs: number;
  private readonly maxMs: number;
  private readonly jitter: number;
  private readonly handshakeTimeoutMs: number;

  constructor(private readonly opts: SyncBridgeOpts) {
    const b = opts.reconnectBackoff;
    this.baseMs = b?.baseMs ?? DEFAULT_BASE_MS;
    this.maxMs = b?.maxMs ?? DEFAULT_MAX_MS;
    this.jitter = b?.jitter ?? DEFAULT_JITTER;
    this.handshakeTimeoutMs = opts.handshakeTimeoutMs ?? DEFAULT_HANDSHAKE_TIMEOUT_MS;
    this.engines = new Map();
    for (const engine of opts.engines) {
      const id = engine.docId();
      if (this.engines.has(id)) {
        throw new Error(`SyncBridge: two engines for doc ${id}`);
      }
      this.engines.set(id, engine);
    }
  }

  /** Doc ids this bridge routes, in construction order. */
  docIds(): string[] {
    return [...this.engines.keys()];
  }

  start(): void {
    this.connect();
    if (!this.tickTimer) {
      // Periodic tick drives each engine's OpenDoc watchdog. Cheap —
      // outside Opening, `handleTimeout` is a no-op.
      this.tickTimer = setInterval(() => {
        for (const engine of this.engines.values()) engine.handleTimeout();
        this.pumpOutbox();
        this.drainEngineEvents();
      }, TICK_INTERVAL_MS);
    }
  }

  stop(): void {
    this.stopped = true;
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.clearHandshakeTimer();
    if (this.tickTimer) {
      clearInterval(this.tickTimer);
      this.tickTimer = null;
    }
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = null;
    }
    // Best-effort engine notification; if already Disconnected this is a no-op.
    for (const engine of this.engines.values()) engine.handleDisconnected();
  }

  /** Cancel any pending backoff timer and reconnect immediately.
   *  Hosts wire this to platform wake-ups (browser `online` /
   *  `visibilitychange`, native push). Cheap no-op if already
   *  connecting or stopped; the attempt counter is unchanged so
   *  repeated failed wake-ups still back off — only a successful
   *  handshake resets it. */
  reconnectNow(): void {
    if (this.stopped) return;
    if (this.ws) return;
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.connect();
  }

  /** Caller signal: local mutations may have produced new outbox
   *  bytes on some engine. Drain every engine and flush. */
  pumpOutbox(): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    for (const engine of this.engines.values()) {
      while (true) {
        const bytes = engine.popOutbox();
        if (!bytes) break;
        this.ws.send(bytes);
      }
    }
    this.opts.onChange?.("drain");
  }

  private connect(): void {
    if (this.stopped) return;
    const ws = this.opts.socketFactory();
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    this.phase = "hello";
    let opened = false;

    ws.onopen = () => {
      opened = true;
      // Version handshake first; engines come online on `HelloAck`.
      ws.send(this.opts.codec.helloFrame(this.opts.clientName, this.opts.clientVersion));
      this.handshakeTimer = setTimeout(() => {
        this.handshakeTimer = null;
        this.opts.onError?.(`handshake timed out after ${this.handshakeTimeoutMs}ms`);
        // `onclose` drives the reconnect.
        ws.close();
      }, this.handshakeTimeoutMs);
    };
    ws.onmessage = (ev) => {
      const data =
        ev.data instanceof ArrayBuffer
          ? new Uint8Array(ev.data)
          : ArrayBuffer.isView(ev.data)
            ? new Uint8Array(ev.data.buffer, ev.data.byteOffset, ev.data.byteLength)
            : null;
      if (!data) return; // text frames ignored
      if (this.phase === "hello") {
        this.handleHelloResponse(ws, data);
        return;
      }
      const docId = this.opts.codec.serverFrameDocId(data);
      const engine = docId === undefined ? undefined : this.engines.get(docId);
      if (!engine) {
        // Not ours: either undecodable or for a doc this device never
        // opened on this connection. The server only sends frames for
        // open docs, so this is a routing bug on one side or the other
        // — report, drop, keep the session.
        this.opts.onError?.(
          docId === undefined
            ? "undecodable server frame dropped"
            : `server frame for doc ${docId} dropped: no engine routes it`,
        );
        return;
      }
      engine.handleServerBytes(data);
      this.pumpOutbox();
      this.drainEngineEvents();
      this.opts.onServerFrame?.(docId!);
    };
    ws.onclose = () => {
      this.clearHandshakeTimer();
      const wasOpen = this.phase === "open";
      this.phase = "hello";
      for (const engine of this.engines.values()) engine.handleDisconnected();
      if (wasOpen) this.opts.onChange?.("offline");
      this.ws = null;
      if (this.stopped) return;
      if (!opened) {
        // Pre-open close: upgrade itself may have been rejected (e.g.
        // revoked device token). Probe to disambiguate from a plain
        // network drop before scheduling the next attempt.
        void this.probeAndReconnect();
      } else {
        this.scheduleReconnect();
      }
    };
    ws.onerror = () => {
      // onclose runs after onerror; let onclose drive reconnect.
    };
  }

  private handleHelloResponse(ws: WebSocket, data: Uint8Array): void {
    this.clearHandshakeTimer();
    const outcome = this.opts.codec.parseHelloResponse(data);
    if (!outcome.ok) {
      this.opts.onError?.(outcome.error ?? "handshake failed");
      // The server closes after a rejection anyway; close from our
      // side too so `onclose` schedules the backoff retry.
      ws.close();
      return;
    }
    this.phase = "open";
    this.attempt = 0;
    for (const engine of this.engines.values()) engine.handleConnected();
    this.pumpOutbox();
    this.drainEngineEvents();
    for (const docId of this.engines.keys()) this.opts.onServerFrame?.(docId);
    this.opts.onChange?.("online");
  }

  private clearHandshakeTimer(): void {
    if (this.handshakeTimer) {
      clearTimeout(this.handshakeTimer);
      this.handshakeTimer = null;
    }
  }

  private async probeAndReconnect(): Promise<void> {
    const probe = this.opts.probeAuth;
    if (probe) {
      const ok = await probe();
      if (!ok) {
        this.stopped = true;
        this.opts.onAuthFailed?.();
        return;
      }
    }
    // While the probe was in flight, the host may have called
    // `reconnectNow()` (or `stop()`); don't double-schedule.
    if (this.stopped || this.ws || this.timer) return;
    this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    if (this.stopped) return;
    const exp = Math.min(this.maxMs, this.baseMs * 2 ** this.attempt);
    const j = this.jitter > 0 ? 1 + (Math.random() * 2 - 1) * this.jitter : 1;
    const delay = Math.max(0, Math.round(exp * j));
    this.attempt++;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.connect();
    }, delay);
  }

  private drainEngineEvents(): void {
    const cb = this.opts.onEngineEvent;
    for (const [docId, engine] of this.engines) {
      while (true) {
        const ev = engine.popEvent();
        if (!ev) break;
        cb?.(ev, docId);
      }
    }
  }
}
