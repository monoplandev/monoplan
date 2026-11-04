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
//   { kind: "window", action }                → null   (a title-bar gesture,
//                                                       see `postWindowGesture`)
//   { kind: "undoState", canUndo, canRedo }   → null   (enables Edit › Undo / Redo)
//
// Inbound (Swift → JS, by `evaluateJavaScript`), on `window.__monoplanHost`:
//
//   .update(docId, b64)   one host → mirror update (`MirrorSink.update`)
//   .stopped(docId)       no update follows (`MirrorSink.stopped`)
//   .sync(status)         the host's sync status, for the footer: sent
//                         once the page is up and on every change
//   .handOff()            flush everything committed to the host now;
//                         resolves once the host has answered the
//                         imports it caused. The app awaits it before
//                         it closes the window or quits.
//   .settings(obj)        the app's device settings changed
//                         (`hostSettings.ts`; the first values arrive
//                         by a document-start script, not here)
//   .undo() / .redo()     Edit › Undo / Redo: the window's one undo
//                         surface (`setUndoRedoHandler`)
//   .find()               the window's search bubble: open the Find
//                         palette (`setFindHandler`)
//
// The doc id is the `workspace` selector the host resolves (`-w` accepts
// a doc uuid, `spec/cli.md`). The peer id travels as a decimal string
// because a u64 does not fit a JSON number. HTTP (`api.ts`) is not
// here: the app serves the page from its own scheme and proxies `/api`
// to the host's server with the device token, so the client's
// same-origin fetches work unchanged and the credential never enters
// the page.

import type { MirrorSeed, MirrorTransport } from "@monoplan/core/mirror-bridge";
import { applyHostSettings, type HostSettings } from "../hostSettings.ts";

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
  | { kind: "import"; docId: string; bytes: string }
  | { kind: "window"; action: WindowGesture }
  | { kind: "undoState"; canUndo: boolean; canRedo: boolean };

/** What the sidebar's title strip (`.nav-titlebar`) forwards: the
 *  window has no title bar of its own (the traffic lights sit over the
 *  strip), and the webview takes every mouse event, so a drag or a
 *  double-click on the strip has to be handed back to the app. */
export type WindowGesture = "drag" | "doubleClick";

/** The handler's reply to `subscribe`. */
export interface SubscribeReply {
  snapshot: string;
  peer: string;
}

/** The one method of a `WKScriptMessageHandlerWithReply` as seen from JS. */
export interface HostMessageHandler {
  postMessage(request: HostRequest): Promise<unknown>;
}

/** The host's view of the server link, as `ipc::SyncStatus` has it. */
export interface HostSyncStatus {
  online: boolean;
  /** Unix millis of the last completed exchange, on any run of the
   *  host, or null if it never synced. */
  lastSyncedAt: number | null;
  /** Local work the server has not confirmed (an engine still pulling,
   *  a push in flight, ops not yet pushed): the footer's "syncing". */
  pending: boolean;
}

/** What the app calls on `window.__monoplanHost`. */
export interface HostInbound {
  update(docId: string, bytes: string): void;
  stopped(docId: string): void;
  sync(status: HostSyncStatus): void;
  handOff(): Promise<void>;
  settings(settings: HostSettings): void;
  undo(): void;
  redo(): void;
  find(): void;
  /** The window entered or left full screen: `[data-fullscreen]` on
   *  the root drops the title-bar strips (styles.css), since there are
   *  no traffic lights to clear and nothing to drag. */
  fullScreen(active: boolean): void;
  /** The host opened a doc after this page booted (the app's New
   *  Workspace…): subscribe to it and show it. */
  docAdded(docId: string): void;
}

export interface AppHost {
  session(): Promise<HostSessionInfo>;
  transport: MirrorTransport;
  /** The host's sync status, whenever the app reports it. The last one
   *  is replayed to a late subscriber, so the footer never misses the
   *  status the app pushed while the page was still booting. */
  onSync(cb: (status: HostSyncStatus) => void): void;
  /** What `handOff()` runs; the runtime binds its own once it exists. */
  onHandOff(cb: () => Promise<void>): void;
  /** A doc the host opened after boot (`docAdded`); the runtime opens
   *  it over the bridge. */
  onDocAdded(cb: (docId: string) => void): void;
}

/** The app host behind this page, or null outside the app: the
 *  presence of the message handler is the whole test. */
export function detectAppHost(): AppHost | null {
  const handler = findHandler();
  if (!handler) return null;
  return createAppHost(handler, window);
}

function findHandler(): HostMessageHandler | null {
  const w = window as unknown as {
    webkit?: { messageHandlers?: Record<string, HostMessageHandler | undefined> };
  };
  return w.webkit?.messageHandlers?.[HOST_HANDLER] ?? null;
}

/** Whether this page is the app's window (`mirror.tsx` marks it), so
 *  the title-bar strips render only there. */
export function isMacShell(): boolean {
  return document.documentElement.dataset.shell === "mac";
}

/** The mouse-down handler of a title-bar strip: a press drags, the
 *  second press of a double-click (by click count: the app's drag takes
 *  the mouse, so the page never sees a `dblclick`) does what the title
 *  bar's double-click would. */
export function onTitlebarMouseDown(e: MouseEvent): void {
  if (e.button !== 0) return;
  e.preventDefault();
  postWindowGesture(e.detail >= 2 ? "doubleClick" : "drag");
}

/** Hand a title-bar gesture to the app; nothing outside it. Fire and
 *  forget: the app acts on the mouse event it is already holding. */
export function postWindowGesture(action: WindowGesture): void {
  post({ kind: "window", action });
}

/** What Edit › Undo / Redo in the app's menu bar run: the window's one
 *  undo surface (the sidebar menu hides its rows there). `Workspace`
 *  binds it; null unbinds. */
let undoRedoHandler: ((kind: "undo" | "redo") => void) | null = null;
export function setUndoRedoHandler(cb: ((kind: "undo" | "redo") => void) | null): void {
  undoRedoHandler = cb;
}

/** What the app's search bubble runs: open the Find palette.
 *  `Workspace` binds it; null unbinds. */
let openFindHandler: (() => void) | null = null;
export function setFindHandler(cb: (() => void) | null): void {
  openFindHandler = cb;
}

/** Tell the app whether its Undo / Redo items should be enabled. */
export function postUndoState(canUndo: boolean, canRedo: boolean): void {
  post({ kind: "undoState", canUndo, canRedo });
}

function post(request: HostRequest): void {
  const handler = findHandler();
  if (!handler) return;
  void handler.postMessage(request).catch((e) => {
    console.warn(`mirror: ${request.kind} failed:`, e);
  });
}

/** Build the host over a message handler, installing the inbound global
 *  on `target`. Exposed for tests, which pass a fake handler and a plain
 *  object. */
export function createAppHost(handler: HostMessageHandler, target: object): AppHost {
  let onUpdate: ((docId: string, bytes: Uint8Array) => void) | null = null;
  let onStopped: ((docId: string) => void) | null = null;
  let onSync: ((status: HostSyncStatus) => void) | null = null;
  let lastSync: HostSyncStatus | null = null;
  let onHandOff: (() => Promise<void>) | null = null;
  let onDocAdded: ((docId: string) => void) | null = null;
  const inbound: HostInbound = {
    update(docId, bytes) {
      onUpdate?.(docId, decodeBytes(bytes));
    },
    stopped(docId) {
      onStopped?.(docId);
    },
    sync(status) {
      lastSync = status;
      onSync?.(status);
    },
    handOff() {
      return onHandOff?.() ?? Promise.resolve();
    },
    settings(settings) {
      applyHostSettings(settings);
    },
    undo() {
      undoRedoHandler?.("undo");
    },
    redo() {
      undoRedoHandler?.("redo");
    },
    find() {
      openFindHandler?.();
    },
    fullScreen(active) {
      if (active) document.documentElement.dataset.fullscreen = "";
      else delete document.documentElement.dataset.fullscreen;
    },
    docAdded(docId) {
      onDocAdded?.(docId);
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
    onSync(cb) {
      onSync = cb;
      if (lastSync) cb(lastSync);
    },
    onHandOff(cb) {
      onHandOff = cb;
    },
    onDocAdded(cb) {
      onDocAdded = cb;
    },
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
