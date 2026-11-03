// Top-level component of the macOS app's window (`spec/mac-window-plan.md`,
// entered from `mirror.tsx`). The browser's `BrowserTabGate` → `App` →
// `BootGate` chain is replaced by one step: ask the in-process host for
// the session and subscribe to each of its docs through the
// `MirrorBridge`, which seeds a mirror engine per doc. There is no
// IndexedDB boot gate (the snapshot is the boot), no single-tab lock
// (the host leases the peer), and no login (the host is signed in).

import { createEffect, createSignal, onMount, Show } from "solid-js";
import { Doc, SyncEngine } from "@monoplan/core/wasm";
import { MirrorBridge, type MirrorSeed } from "@monoplan/core/mirror-bridge";
import { useAppI18n } from "./i18n.tsx";
import { loadPrefs } from "./prefs.ts";
import { SessionProvider } from "./SessionContext.tsx";
import { Workspace } from "./Workspace.tsx";
import { detectAppHost, type AppHost } from "./sync/mirrorHost.ts";
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
  const [error, setError] = createSignal<string | null>(null);
  // "Online" here is the host: true from the first seeded doc until the
  // host stops serving them. Whether the host itself reaches the server
  // is a later addition (its sync status over the same handler).
  const [online, setOnline] = createSignal(false);
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
    try {
      const info = await host.session();
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
          if (kind === "online") setOnline(true);
          if (kind === "offline") setOnline(false);
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
              setOnline,
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
  /** Hand the parent the runtime's drain so the bridge's frame pulse
   *  reaches the stores. Called synchronously during setup, before any
   *  frame can arrive after the runtime materialised its stores. */
  bind: (drain: (docId: string) => void) => void;
}) {
  const runtime = createMirrorRuntime(props.boot);
  props.bind(runtime.drain);
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
