// The macOS app's host as the webview reaches it (`spec/mac-window-plan.md`
// "JS side" / "Swift side"): a `MirrorTransport` over a WebKit message
// handler, plus the session the host holds (account, device, doc list),
// which replaces `Login.tsx` in the window. Bytes cross as base64.
//
// Outbound (JS → Swift): `window.webkit.messageHandlers.monoplan
// .postMessage(request)`, registered on the Swift side with a reply
// handler (`WKScriptMessageHandlerWithReply`), so `postMessage` resolves
// with the reply or rejects with the host's error message:
//
//   { kind: "session" }                       → HostSessionInfo
//   { kind: "subscribe", docId }              → { snapshot: b64, peer: "u64 decimal" }
//   { kind: "unsubscribe", docId }            → null
//   { kind: "import", docId, bytes: b64 }     → null   (`Host.import`)
//
// Inbound (Swift → JS, by `evaluateJavaScript`), on `window.__monoplanHost`:
//
//   .update(docId, b64)   one host → mirror update (`MirrorSink.update`)
//   .stopped(docId)       no update follows (`MirrorSink.stopped`)
//
// The doc id is the `workspace` selector the host resolves (`-w` accepts
// a doc uuid, `spec/cli.md`). The peer id travels as a decimal string
// because a u64 does not fit a JSON number.

import type { MirrorSeed, MirrorTransport } from "@monoplan/core/mirror-bridge";

/** Name of the WebKit message handler the app registers. */
export const HOST_HANDLER = "monoplan";
/** Name of the global the app calls back into. */
export const HOST_GLOBAL = "__monoplanHost";

export interface HostDocInfo {
  docId: string;
  /** The doc's name as the host last saw it, if it has one. */
  name: string | null;
}

/** What the host knows about the signed-in profile. */
export interface HostSessionInfo {
  accountId: string;
  email: string;
  deviceId: string;
  primaryDocId: string;
  /** The server the host syncs with: where `api.ts` should point. */
  serverUrl: string;
  /** Every doc the profile has open, Home included. */
  docs: HostDocInfo[];
}

export type HostRequest =
  | { kind: "session" }
  | { kind: "subscribe"; docId: string }
  | { kind: "unsubscribe"; docId: string }
  | { kind: "import"; docId: string; bytes: string };

/** The handler's reply to `subscribe`. */
export interface SubscribeReply {
  snapshot: string;
  peer: string;
}

/** The one method of a `WKScriptMessageHandlerWithReply` as seen from JS. */
export interface HostMessageHandler {
  postMessage(request: HostRequest): Promise<unknown>;
}

/** What the app calls on `window.__monoplanHost`. */
export interface HostInbound {
  update(docId: string, bytes: string): void;
  stopped(docId: string): void;
}

export interface AppHost {
  session(): Promise<HostSessionInfo>;
  transport: MirrorTransport;
}

/** The app host behind this page, or null outside the app: the
 *  presence of the message handler is the whole test. */
export function detectAppHost(): AppHost | null {
  const w = window as unknown as {
    webkit?: { messageHandlers?: Record<string, HostMessageHandler | undefined> };
  };
  const handler = w.webkit?.messageHandlers?.[HOST_HANDLER];
  if (!handler) return null;
  return createAppHost(handler, window);
}

/** Build the host over a message handler, installing the inbound global
 *  on `target`. Exposed for tests, which pass a fake handler and a plain
 *  object. */
export function createAppHost(handler: HostMessageHandler, target: object): AppHost {
  let onUpdate: ((docId: string, bytes: Uint8Array) => void) | null = null;
  let onStopped: ((docId: string) => void) | null = null;
  const inbound: HostInbound = {
    update(docId, bytes) {
      onUpdate?.(docId, decodeBytes(bytes));
    },
    stopped(docId) {
      onStopped?.(docId);
    },
  };
  (target as Record<string, unknown>)[HOST_GLOBAL] = inbound;

  const post = (request: HostRequest): Promise<unknown> => handler.postMessage(request);

  const transport: MirrorTransport = {
    async subscribe(docId: string): Promise<MirrorSeed> {
      const reply = (await post({ kind: "subscribe", docId })) as SubscribeReply;
      return {
        docId,
        snapshot: decodeBytes(reply.snapshot),
        peer: BigInt(reply.peer),
      };
    },
    unsubscribe(docId: string): void {
      void post({ kind: "unsubscribe", docId }).catch((e) => {
        console.warn("mirror: unsubscribe failed:", e);
      });
    },
    async send(docId: string, bytes: Uint8Array): Promise<void> {
      await post({ kind: "import", docId, bytes: encodeBytes(bytes) });
    },
    onUpdate(cb) {
      onUpdate = cb;
    },
    onStopped(cb) {
      onStopped = cb;
    },
  };

  return {
    transport,
    session: async () => (await post({ kind: "session" })) as HostSessionInfo,
  };
}

// A snapshot can run to megabytes; `String.fromCharCode` over a whole
// buffer blows the argument limit and a per-byte string append is
// quadratic in some engines, so encode in chunks.
const CHUNK = 0x8000;

export function encodeBytes(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}

export function decodeBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
