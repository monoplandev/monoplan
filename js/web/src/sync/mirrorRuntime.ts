// The app window's runtime (`spec/mac-window-plan.md` "JS side"): the
// twin of `createWorkspaceRuntime` for docs that are mirrors of the
// in-process host's. Same store (`createSyncedApp`), same docs layer,
// same view state; what differs is the IO around each doc, and there is
// much less of it:
//
//   - no IndexedDB op log, no capture, no snapshot folds: the host is
//     the durable copy, and every flush hands the mirror's ops to it
//     through the `MirrorBridge` (the store's `setOnFlush` hook, where
//     the browser pumps the WebSocket bridge);
//   - no WebSocket, no DEK, no device row, no login: the session came
//     from the host, and the DEK-touching flows (create / join / delete
//     a doc, invites, password) are unavailable to the page; the app's
//     own menus and the CLI do them through the host, and a doc the
//     host opens after boot arrives here through `openDoc`;
//   - host updates arrive through the bridge's `onServerFrame`, which
//     the app routes to `drain(docId)` here.
//
// Like the browser runtime this registers cleanups against the calling
// component's reactive owner, so call it synchronously from a component
// body.

import { onCleanup } from "solid-js";
import type { SyncEngine } from "@monoplan/core/wasm";
import type { MirrorBridge } from "@monoplan/core/mirror-bridge";
import type { Session } from "../Login.tsx";
import type { Prefs } from "../prefs.ts";
import { createDocsLayer, type DocsLayer } from "./docs.ts";
import { createSyncedApp, type DocApp } from "./store.ts";
import type { HostSessionInfo } from "./mirrorHost.ts";
import type { PresentAccount } from "./runtime.ts";
import { createViewState } from "./viewState.ts";

/** The session as the window sees it: the host's identity, no key. */
export function sessionFromHost(info: HostSessionInfo): Session {
  return {
    anonymous: false,
    mirror: true,
    accountId: info.accountId,
    primaryDocId: info.primaryDocId,
    email: info.email,
    deviceId: info.deviceId,
    dek: null,
    docKeys: new Map(),
    freshSignup: false,
  };
}

/** Everything the app resolved before the runtime can start: the
 *  session as the host reports it, one seeded mirror engine per open
 *  doc (Home first), the bridge that routes them, and the prefs row. */
export type MirrorBoot = {
  session: Session;
  engines: SyncEngine[];
  bridge: MirrorBridge;
  prefs: Prefs | null;
};

export function createMirrorRuntime(boot: MirrorBoot) {
  const docApps = new Map<string, DocApp>();
  // A store over one seeded mirror engine. Host updates that landed
  // between the seed and now (while the other docs were opening)
  // queued AppEvents on the doc; the store's initial materialisation
  // already reflects them, so they must not be dispatched on top of it
  // afterwards.
  const mount = (engine: SyncEngine): DocApp => {
    while (engine.popAppEvent()) {
      // discard
    }
    const app = createSyncedApp(engine);
    app.setOnFlush(() => boot.bridge.pumpOutbox());
    docApps.set(app.docId, app);
    return app;
  };
  for (const engine of boot.engines) mount(engine);
  const app = docApps.get(boot.session.primaryDocId);
  if (!app) throw new Error("mirror boot has no Home doc");
  const docs: DocsLayer = createDocsLayer(app, [...docApps.values()]);
  const { view, setView } = createViewState(docs, boot.prefs, boot.session.accountId);

  if (import.meta.env.DEV && typeof window !== "undefined") {
    const w = window as unknown as {
      __app?: DocApp;
      __apps?: Map<string, DocApp>;
      __docs?: DocsLayer;
    };
    w.__app = app;
    w.__apps = docApps;
    w.__docs = docs;
    onCleanup(() => {
      delete w.__app;
      delete w.__apps;
      delete w.__docs;
    });
  }

  /** Pump every doc to the host: a flush of whatever is committed but
   *  not yet exported (a notes burst still on its idle timer), then the
   *  outbox. Resolves once the host has answered every import this
   *  caused, which is what the app waits for before it drops the
   *  subscriptions on close or quit. */
  const handOff = (): Promise<void> => {
    for (const a of docApps.values()) a.flushNotes();
    for (const engine of boot.engines) engine.flush();
    boot.bridge.pumpOutbox();
    return boot.bridge.flushed();
  };

  // The window going away or to the background is the moment the
  // browser folds its WAL; here it is the moment to make sure the host
  // holds everything the window committed.
  if (typeof document !== "undefined") {
    const onVisibility = (): void => {
      if (document.visibilityState === "hidden") void handOff();
    };
    const onPageHide = (): void => {
      void handOff();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", onPageHide);
    onCleanup(() => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", onPageHide);
    });
  }
  onCleanup(() => {
    void handOff();
    boot.bridge.stop();
  });

  /** A doc the host opened after boot (the app's New Workspace…, which
   *  creates it host-side): subscribe over the bridge, mount a store
   *  and add it to the layer, the window's half of the browser
   *  runtime's `openNewDoc`. A doc already open is left alone. Resolves
   *  to the doc id once it is in `docs`. */
  const openDoc = async (docId: string): Promise<string> => {
    if (docApps.has(docId)) return docId;
    const engine = await boot.bridge.open(docId);
    boot.engines.push(engine);
    docs.addDoc(mount(engine));
    return docId;
  };

  /** A doc the host stopped serving (deleted here or elsewhere, or the
   *  server closed it): leave it if the view is in it, end its route
   *  and drop its store, the window's half of the browser runtime's
   *  `forgetOpenDoc`. Idempotent; Home is never dropped. */
  const closeDoc = (docId: string): void => {
    if (docId === boot.session.primaryDocId) return;
    const a = docApps.get(docId);
    if (!a) return;
    if (docs.viewDocId(view()) === docId) setView({ kind: "list", id: "inbox" });
    boot.bridge.close(docId);
    const i = boot.engines.indexOf(a.engine);
    if (i >= 0) boot.engines.splice(i, 1);
    docs.removeDoc(docId);
    docApps.delete(docId);
  };

  const unavailable =
    (what: string) =>
    async (): Promise<never> => {
      throw new Error(`${what} is not available in the app window yet; use the menu bar or the browser`);
    };

  return {
    app,
    docApps,
    docs,
    view,
    setView,
    createDoc: unavailable("creating a workspace") as (name: string) => Promise<string>,
    joinDoc: unavailable("joining a workspace") as (
      docId: string,
      dek: import("@monoplan/core/wasm").Dek,
    ) => Promise<string>,
    /** The app deletes through the host (`MirrorMain`); this is the
     *  fallback when no app is wired. */
    deleteDoc: unavailable("deleting a workspace") as (docId: string) => Promise<void>,
    openDoc,
    closeDoc,
    /** Presence is a server fact the host has not relayed yet. */
    presence: (_docId: string): PresentAccount[] => [],
    /** Route the bridge's `onServerFrame` here: drain the named doc's
     *  queued AppEvents into its store. */
    drain(docId: string): void {
      docApps.get(docId)?.drainEvents();
    },
    /** Hand everything committed so far to the host now. */
    handOff,
  };
}
