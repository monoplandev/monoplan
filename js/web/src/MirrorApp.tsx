// Top-level component of the macOS app's window (`spec/mac-window-plan.md`,
// entered from `mirror.tsx`). The browser's `BrowserTabGate` → `App` →
// `BootGate` chain is replaced by one step: ask the in-process host for
// the session and subscribe to each of its docs through the
// `MirrorBridge`, which seeds a mirror engine per doc. There is no
// IndexedDB boot gate (the snapshot is the boot), no single-tab lock
// (the host leases the peer), and no login (the host is signed in).

import { createEffect, createMemo, createSignal, onMount, Show } from "solid-js";
import { Doc, SyncEngine } from "@monoplan/core/wasm";
import { MirrorBridge, type MirrorSeed } from "@monoplan/core/mirror-bridge";
import { useAppI18n } from "./i18n.tsx";
import { loadPrefs } from "./prefs.ts";
import { SessionProvider } from "./SessionContext.tsx";
import { Workspace } from "./Workspace.tsx";
import { detectAppHost, type AppHost } from "./sync/mirrorHost.ts";
import { serverLinkBase, setLinkBase } from "./url.ts";
import {
  createMirrorRuntime,
  sessionFromHost,
  type MirrorBoot,
} from "./sync/mirrorRuntime.ts";

export function MirrorApp(props: {
  /** The host to use instead of the page's WebKit handler (tests). */
  host?: AppHost;
}) {
  const { m, locale, direction } = useAppI18n();
  createEffect(() => {
    document.documentElement.lang = locale();
    document.documentElement.dir = direction();
  });
  const [boot, setBoot] = createSignal<MirrorBoot | null>(null);
  const [host, setHost] = createSignal<AppHost | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  // "Online" is two facts: the host is serving the mirrors (the bridge,
  // from the first seeded doc until the host stops them) and the host
  // itself reaches the server (its sync status, pushed by the app). The
  // footer shows the conjunction, as the tray does.
  const [serving, setServing] = createSignal(false);
  const [hostOnline, setHostOnline] = createSignal(false);
  const online = createMemo(() => serving() && hostOnline());
  const [lastSyncAt, setLastSyncAt] = createSignal<number | null>(null);
  // The bridge is built before the runtime exists (it seeds the engines
  // the runtime is built from); its frame pulse is bound here once the
  // runtime is up. Frames before that have nothing to drain into.
  let drainFor: (docId: string) => void = () => {};

  onMount(async () => {
    const host = props.host ?? detectAppHost();
    if (!host) {
      setError("This page only runs inside the Monoplan app.");
      return;
    }
    host.onSync((status) => {
      setHostOnline(status.online);
      setLastSyncAt(status.lastSyncedAt);
    });
    try {
      const info = await host.session();
      // Shareable links name the server's web client, not this window
      // (`spec/urls.md` "Affordances").
      setLinkBase(serverLinkBase(info.serverUrl));
      const bridge = new MirrorBridge({
        transport: host.transport,
        createEngine: (seed: MirrorSeed) => {
          const doc = Doc.loadWithPeer(seed.snapshot, seed.peer);
          // Items the window creates carry the account id like any
          // authed session's (`createdBy`, spec/data-model.md "Item").
          doc.setAuthor(info.accountId);
          return SyncEngine.mirror(doc, seed.docId);
        },
        onServerFrame: (docId) => drainFor(docId),
        onChange: (kind) => {
          if (kind === "online") setServing(true);
          if (kind === "offline") setServing(false);
        },
        onStopped: (docId) => {
          console.warn(`mirror: the host stopped serving doc ${docId}`);
        },
        onError: (message) => {
          console.error("mirror:", message);
        },
      });
      // Home first, so a failure there is the one reported; a non-Home
      // doc that will not open is left out rather than failing the
      // window.
      const engines = [await bridge.open(info.primaryDocId)];
      for (const d of info.docs) {
        if (d.docId === info.primaryDocId) continue;
        try {
          engines.push(await bridge.open(d.docId));
        } catch (e) {
          console.warn(`mirror: doc ${d.docId} did not open:`, e);
        }
      }
      const prefs = await loadPrefs(info.accountId).catch(() => null);
      setHost(host);
      setBoot({ session: sessionFromHost(info), engines, bridge, prefs });
    } catch (e) {
      console.error("[mirror boot] FAILED:", e);
      setError(e instanceof Error ? e.message : String(e));
    }
  });

  return (
    <Show when={!error()} fallback={<div class="empty">Failed to start: {error()}</div>}>
      <Show when={boot()} fallback={<div class="empty">{m().common.loading}</div>}>
        {(b) => (
          <SessionProvider
            value={{
              session: () => b().session,
              online,
              // The runtime never sets these on a mirror (no WebSocket
              // of its own); the app's status pushes do.
              setOnline: setHostOnline,
              lastSyncAt,
              setLastSyncAt,
              // Signing out is the tray's; the window cannot revoke the
              // device or wipe the profile it mirrors.
              logout: () => {
                console.warn("mirror: sign out from the menu bar");
              },
              swapSession: () => {},
            }}
          >
            <MirrorMain
              boot={b()}
              host={host()!}
              bind={(drain) => {
                drainFor = drain;
              }}
            />
          </SessionProvider>
        )}
      </Show>
    </Show>
  );
}

function MirrorMain(props: {
  boot: MirrorBoot;
  host: AppHost;
  /** Hand the parent the runtime's drain so the bridge's frame pulse
   *  reaches the stores. Called synchronously during setup, before any
   *  frame can arrive after the runtime materialised its stores. */
  bind: (drain: (docId: string) => void) => void;
}) {
  const runtime = createMirrorRuntime(props.boot);
  props.bind(runtime.drain);
  // The app calls `handOff` before closing the window or quitting, then
  // waits for the imports it caused before it drops the subscriptions.
  props.host.onHandOff(runtime.handOff);
  // The app's New Workspace… created a doc on the host: open it here
  // and go to its Inbox, as the browser's dialog does on create.
  props.host.onDocAdded((docId) => {
    runtime
      .openDoc(docId)
      .then((id) => runtime.setView(runtime.docs.listView(id, "inbox")))
      .catch((e: unknown) => console.error("mirror: the new doc did not open:", e));
  });
  return (
    <Workspace
      docs={runtime.docs}
      view={runtime.view}
      setView={runtime.setView}
      createDoc={runtime.createDoc}
      joinDoc={runtime.joinDoc}
      deleteDoc={runtime.deleteDoc}
      presence={runtime.presence}
    />
  );
}
