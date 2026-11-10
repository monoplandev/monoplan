// Top-level component of the macOS app's window (`spec/mac-window-plan.md`,
// entered from `mirror.tsx`). The browser's `BrowserTabGate` → `App` →
// `BootGate` chain is replaced by one step: ask the in-process host for
// the session and subscribe to each of its docs through the
// `MirrorBridge`, which seeds a mirror `Replica` per doc. There is no
// IndexedDB boot gate (the snapshot is the boot), no single-tab lock
// (the engine leases the peer), and no login (the engine is signed in).

import { createEffect, createMemo, createSignal, onMount, Show } from "solid-js";
import { Doc, Replica } from "@monoplan/core/wasm";
import { MirrorBridge, type MirrorSeed } from "@monoplan/core/mirror-bridge";
import { useAppI18n } from "./i18n.tsx";
import { deletePrefs, loadPrefs, savePrefs, type Prefs } from "./prefs.ts";
import { SessionProvider } from "./SessionContext.tsx";
import { Workspace } from "./Workspace.tsx";
import {
  detectAppHost,
  type AppHost,
  type EngineSessionInfo,
  type EngineState,
} from "./sync/mirrorHost.ts";
import { serverLinkBase, setLinkBase } from "./url.ts";
import {
  createMirrorRuntime,
  localAccountId,
  sessionFromHost,
  type MirrorBoot,
} from "./sync/mirrorRuntime.ts";

export function MirrorApp(props: {
  /** The app host to use instead of the page's WebKit handler (tests). */
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
  // No engine to mirror yet (starting, or stopping for a sign-out): the
  // waiting screen instead of the workspace. The app reloads the page
  // once an engine serves, so this only ever moves between reasons.
  const [gate, setGate] = createSignal<EngineState | null>(null);
  // "Online" is two facts: the engine is serving the mirrors (the bridge,
  // from the first seeded doc until the engine stops them) and the engine
  // itself reaches the server (its sync status, pushed by the app). The
  // footer shows the conjunction, as the tray does.
  const [serving, setServing] = createSignal(false);
  const [engineOnline, setEngineOnline] = createSignal(false);
  const online = createMemo(() => serving() && engineOnline());
  const [lastSyncAt, setLastSyncAt] = createSignal<number | null>(null);
  // The bridge is built before the runtime exists (it seeds the replicas
  // the runtime is built from); its frame pulse is bound here once the
  // runtime is up. Frames before that have nothing to drain into.
  let drainFor: (docId: string) => void = () => {};
  // Likewise the engine stopping a doc (deleted, or closed by the server)
  // drops it from the runtime once there is one.
  let closeFor: (docId: string) => void = () => {};

  onMount(async () => {
    const host = props.host ?? detectAppHost();
    if (!host) {
      setError(m().mirror.notInApp);
      return;
    }
    host.onSync((status) => {
      setEngineOnline(status.online);
      setLastSyncAt(status.lastSyncedAt);
    });
    try {
      const reply = await host.session();
      if (reply.status !== "ok") {
        setHost(host);
        setGate(reply);
        host.onEngineState(setGate);
        return;
      }
      const info = reply;
      // Shareable links name the server's web client, not this window
      // (`spec/urls.md` "Affordances"); a local-only workspace has no
      // server and "Copy link" is disabled on an anonymous session.
      if (!info.localOnly) setLinkBase(serverLinkBase(info.serverUrl));
      const bridge = new MirrorBridge({
        transport: host.transport,
        createReplica: (seed: MirrorSeed) => {
          const doc = Doc.loadWithPeer(seed.snapshot, seed.peer);
          // Items the window creates carry the account id like any
          // authed session's (`createdBy`, spec/data-model.md "Item");
          // a local-only workspace has none, like the browser's
          // anonymous session.
          if (!info.localOnly) doc.setAuthor(info.accountId);
          return Replica.mirror(doc, seed.docId);
        },
        onServerFrame: (docId) => drainFor(docId),
        onChange: (kind) => {
          if (kind === "online") setServing(true);
          if (kind === "offline") setServing(false);
        },
        onStopped: (docId) => {
          console.warn(`mirror: the engine stopped serving doc ${docId}`);
          closeFor(docId);
        },
        onError: (message) => {
          console.error("mirror:", message);
        },
      });
      // Home first, so a failure there is the one reported; a non-Home
      // doc that will not open is left out rather than failing the
      // window.
      const replicas = [await bridge.open(info.primaryDocId)];
      for (const d of info.docs) {
        if (d.docId === info.primaryDocId) continue;
        try {
          replicas.push(await bridge.open(d.docId));
        } catch (e) {
          console.warn(`mirror: doc ${d.docId} did not open:`, e);
        }
      }
      const session = sessionFromHost(info);
      const prefs = await loadBootPrefs(session.accountId, info).catch(() => null);
      setHost(host);
      setBoot({ session, replicas, bridge, prefs });
    } catch (e) {
      console.error("[mirror boot] FAILED:", e);
      setError(e instanceof Error ? e.message : String(e));
    }
  });

  return (
    <Show
      when={!error()}
      fallback={
        <div class="empty">
          {m().mirror.failedToStart}: {error()}
        </div>
      }
    >
      <Show when={!gate()} fallback={<EngineGate state={gate()!} />}>
        <Show when={boot()} fallback={<div class="empty">{m().common.loading}</div>}>
          {(b) => (
          <SessionProvider
            value={{
              session: () => b().session,
              online,
              // The runtime never sets these on a mirror (no WebSocket
              // of its own); the app's status pushes do.
              setOnline: setEngineOnline,
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
              bind={(drain, close) => {
                drainFor = drain;
                closeFor = close;
              }}
            />
          </SessionProvider>
          )}
        </Show>
      </Show>
    </Show>
  );
}

/** The prefs row for this boot. Signup adopts the local-only workspace
 *  (same doc id), so a row the anonymous session saved under its
 *  derived account id follows the account the first time the page
 *  boots signed in, as the browser's `onAuthenticated` does. */
async function loadBootPrefs(accountId: string, info: EngineSessionInfo): Promise<Prefs | null> {
  const prefs = await loadPrefs(accountId);
  if (prefs || info.localOnly) return prefs;
  const local = localAccountId(info.primaryDocId);
  const adopted = await loadPrefs(local);
  if (!adopted) return null;
  await savePrefs(accountId, adopted);
  await deletePrefs(local);
  return adopted;
}

/** The window without an engine yet: waiting on it, with the app's
 *  reason. Centred in the window, under the title-bar strip the app
 *  draws its traffic lights over. */
function EngineGate(props: { state: EngineState }) {
  const { m } = useAppI18n();
  return (
    <div class="mirror-gate">
      <p class="mirror-gate-title">{m().mirror.waiting}</p>
      <Show when={props.state.status === "pending" && props.state}>
        {(s) => <p class="mirror-gate-hint">{s().reason}</p>}
      </Show>
    </div>
  );
}

function MirrorMain(props: {
  boot: MirrorBoot;
  host: AppHost;
  /** Hand the parent the runtime's drain so the bridge's frame pulse
   *  reaches the stores, and its close so a doc the engine stops serving
   *  leaves the page. Called synchronously during setup, before any
   *  frame can arrive after the runtime materialised its stores. */
  bind: (drain: (docId: string) => void, close: (docId: string) => void) => void;
}) {
  const runtime = createMirrorRuntime(props.boot);
  props.bind(runtime.drain, runtime.closeDoc);
  // Delete goes through the app: a native confirm, then the engine
  // deletes on the server and forgets its copy; the page drops the doc
  // once the app says it is gone (the engine's `stopped` for it may have
  // done so already).
  const deleteDoc = async (docId: string, name: string): Promise<void> => {
    if (await props.host.deleteDoc(docId, name)) runtime.closeDoc(docId);
  };
  // The app calls `handOff` before closing the window or quitting, then
  // waits for the imports it caused before it drops the subscriptions.
  props.host.onHandOff(runtime.handOff);
  // The app's New Workspace… created a doc on the engine: open it here
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
      deleteDoc={deleteDoc}
      presence={runtime.presence}
    />
  );
}
