// Top-level app. First visit auto-creates a local-only anonymous
// session (DEK generated client-side, no server account) and drops
// straight into the workspace — no auth gate. Reads and mutations
// always flow through the SyncEngine over an IndexedDB op log, anonymous
// or authed. Signing up or logging in via Settings swaps the anonymous session for
// an authenticated one (deleting the local doc) and turns on server
// sync, so peer ops apply live. Anonymous sessions run the same engine
// but never ship ops to a server.

import { createEffect, createSignal, onMount, Show } from "solid-js";
import { Dek, Doc, EncryptedBlob } from "@monoplan/core/wasm";
import { IdbStorage, readOrMintPeerSlot } from "@monoplan/core";
import { loadPrefs } from "./prefs.ts";
import { refreshLocationBias } from "./locationBias.ts";
import { api } from "./api.ts";
import { dekVault } from "./sync/dekVault.ts";
import { useAppI18n } from "./i18n.tsx";
import { type Session } from "./Login.tsx";
import { Workspace } from "./Workspace.tsx";
import { createWorkspaceRuntime, type BootInfo } from "./sync/runtime.ts";
import { SessionProvider, useSession } from "./SessionContext.tsx";
import { clearAuthPromptDismissed } from "./nav.tsx";

export function App(props: {
  /** True when this tab holds the `monoplan-single-tab` Web Lock — the
   *  claim on peer slot 0. Without it, boot minted a random peer. */
  singleTabLockHeld: boolean;
}) {
  const { m, locale, direction } = useAppI18n();
  createEffect(() => {
    document.documentElement.lang = locale();
    document.documentElement.dir = direction();
  });
  // `null` = vault probe still in flight; once it resolves we either
  // restore the persisted session or auto-mint a fresh anonymous one —
  // `session()` is never null after that point.
  const [session, setSession] = createSignal<Session | null>(null);
  const [online, setOnline] = createSignal(false);
  // Wall-clock timestamp of the last successful server frame (recv or
  // outbox flush). Reset on logout/session-swap so the new account
  // doesn't inherit the previous device's last-synced time.
  const [lastSyncAt, setLastSyncAt] = createSignal<number | null>(null);
  const [boot, setBoot] = createSignal<BootInfo | null>(null);
  const [bootError, setBootError] = createSignal<string | null>(null);

  // Probe the vault on mount. If a wrapped DEK is present and we can
  // unwrap it, restore that session — for authenticated records, the
  // device cookie should still be valid (the WS pump will surface the
  // failure if it isn't); for anonymous records, the local IndexedDB
  // op log is the source of truth. If there's no record at all, mint a
  // fresh anonymous session so the user lands directly in the app.
  //
  // The place-lookup bias refresh rides the same mount: it is independent
  // of the session, never throws, and is fire-and-forget so a slow or
  // prompted geolocation call cannot hold up boot (`locationBias.ts`).
  onMount(async () => {
    void refreshLocationBias();
    try {
      const v = await dekVault.load();
      if (v) {
        setSession({
          anonymous: v.anonymous,
          email: v.email,
          accountId: v.accountId,
          primaryDocId: v.primaryDocId,
          deviceId: v.deviceId,
          dek: v.dek,
          freshSignup: false,
        });
        return;
      }
    } catch (e) {
      console.warn("vault load failed:", e);
    }
    setSession(await createAnonymousSession());
  });

  // Logout: tear down server-side state and replace the session with
  // a fresh anonymous one. Anonymous sessions also flow through here
  // (e.g. "discard local data") — `api.logout` no-ops cleanly when
  // there's no device cookie.
  const logout = async () => {
    const current = session();
    if (current && !current.anonymous) {
      try {
        await api.logout();
      } catch (e) {
        // Best-effort: even if the server call fails (offline, expired
        // cookie), drop local state so the next login is clean.
        console.warn("logout call failed:", e);
      }
    }
    await dekVault.clear();
    // Signing out is a deliberate act — re-prompt the fresh anonymous
    // session to sign back in, even if the prompt was dismissed before.
    clearAuthPromptDismissed();
    setBoot(null);
    setBootError(null);
    setOnline(false);
    setLastSyncAt(null);
    setSession(await createAnonymousSession());
  };

  const onAuthenticated = (s: Session) => {
    // Local-only anonymous data is left to drift in IndexedDB under the
    // old anon accountId. It'll never be addressed again — option C
    // says clobber, not migrate. A future cleanup pass can reap it.
    setBoot(null);
    setBootError(null);
    setOnline(false);
    setLastSyncAt(null);
    setSession(s);
  };

  return (
    <Show
      when={session() !== null}
      fallback={<div class="empty">{m().common.loading}</div>}
    >
      <Show keyed when={session()}>
        {(s) => (
          <SessionProvider
            value={{
              session: () => s,
              online,
              setOnline,
              lastSyncAt,
              setLastSyncAt,
              logout,
              swapSession: onAuthenticated,
            }}
          >
            <BootGate
              session={s}
              singleTabLockHeld={props.singleTabLockHeld}
              boot={boot()}
              bootError={bootError()}
              setBoot={setBoot}
              setBootError={setBootError}
            />
          </SessionProvider>
        )}
      </Show>
    </Show>
  );
}

async function createAnonymousSession(): Promise<Session> {
  const accountId = `anon-${crypto.randomUUID()}`;
  // Anonymous sessions never reach the server, so there's no
  // server-assigned doc id to use. Mint one locally; matches the
  // shape authenticated sessions get from signup/login responses.
  const primaryDocId = crypto.randomUUID();
  const dek = Dek.generate();
  const session: Session = {
    anonymous: true,
    accountId,
    primaryDocId,
    email: null,
    deviceId: null,
    dek,
    // Seed the doc on first run via Doc.create(). On reload we read
    // the IndexedDB op log instead.
    freshSignup: true,
  };
  try {
    await dekVault.save({
      anonymous: true,
      accountId,
      primaryDocId,
      email: null,
      deviceId: null,
      dek: dek.clone(),
    });
  } catch (e) {
    console.warn("dekVault.save failed for anonymous session:", e);
  }
  return session;
}

function BootGate(props: {
  session: Session;
  singleTabLockHeld: boolean;
  boot: BootInfo | null;
  bootError: string | null;
  setBoot: (b: BootInfo | null) => void;
  setBootError: (m: string | null) => void;
}) {
  const { m } = useAppI18n();
  // Rebuild the doc from the engine WAL (`spec/local-storage.md`
  // §"Web boot"), mirroring the CLI's `boot_doc`: load the snapshot (a bare
  // Loro snapshot) and replay every WAL row after it via the deferred replay
  // API, finalize indexes once, then `markPersisted()` so the WAL capture
  // cursor covers the replayed ops (what still needs *uploading* is
  // derived separately from the persisted `serverKnownVv`). Fresh
  // signups — and brand-new anonymous docs with an empty store — start
  // from `Doc.create()` so the seeded built-ins land; authed devices
  // with an empty store start empty and let sync deliver a snapshot.
  onMount(async () => {
    try {
      // Prefs are independent of the op-log replay — fire in parallel so
      // they don't add a serial roundtrip to first paint. A miss (first
      // run for this account on this device) resolves null and `MainApp`
      // falls back to defaults.
      const prefsPromise = loadPrefs(props.session.accountId).catch(() => null);
      const dek = props.session.dek;
      // The slot-0 peer claim rides the storage open — holding the
      // single-tab Web Lock (checked by `BrowserTabGate` before this
      // mounts) IS the claim, so all that's left is reading the minted
      // peer id. No lock → random peer, so a gate-disabled second tab
      // can never share a peer with the holder.
      const [storage, peerId] = await Promise.all([
        IdbStorage.open(props.session.primaryDocId),
        props.singleTabLockHeld ? readOrMintPeerSlot(0) : Promise.resolve(null),
      ]);
      const rows = storage.bootRows();

      // The peer must be bound at construction — before `Doc.create`'s
      // builtin seed commits and before the replay import (Loro resumes
      // the slot peer's counter from the imported history).
      const create = () =>
        peerId !== null ? Doc.createWithPeer(peerId) : Doc.create();
      const empty = () =>
        peerId !== null ? Doc.emptyWithPeer(peerId) : Doc.empty();

      let doc: Doc;
      let seeded = false;
      if (props.session.freshSignup) {
        doc = create();
        seeded = true;
      } else if (rows.snapshot || rows.replay.length > 0) {
        doc = empty();
        if (rows.snapshot) {
          doc.replayOplogUpdate(
            dek.open(new EncryptedBlob(rows.snapshot.nonce, rows.snapshot.ciphertext)),
          );
        }
        for (const r of rows.replay) {
          doc.replayOplogUpdate(dek.open(new EncryptedBlob(r.nonce, r.ciphertext)));
        }
        doc.finishOplogReplay();
        doc.markPersisted();
      } else if (props.session.anonymous) {
        // Brand-new (or wiped) local-only doc — seed the built-ins.
        doc = create();
        seeded = true;
      } else {
        // Authed device, empty local store — sync will send a snapshot.
        doc = empty();
      }

      props.setBoot({
        doc,
        // Resume cursor comes from the engine's own persisted state
        // (`IdbStorage`/`writeAckedSeq`), not the device row.
        lastAcked: BigInt(rows.lastAckedSeq),
        storage,
        lastLocalSeq: rows.lastLocalSeq,
        serverKnownVv: rows.serverKnownVv,
        inFlightPush: rows.inFlightPush,
        walRows: rows.replay.length,
        walBytes: rows.walBytes,
        seeded,
        prefs: await prefsPromise,
      });
    } catch (e) {
      // Hard fail & surfaces idb boot errors
      console.error("[boot] FAILED:", e);
      props.setBootError(e instanceof Error ? e.message : String(e));
    }
  });

  return (
    <Show
      when={!props.bootError}
      fallback={<div class="empty">Failed to start: {props.bootError}</div>}
    >
      <Show when={props.boot} fallback={<div class="empty">{m().common.loading}</div>}>
        {(b) => <MainApp session={props.session} boot={b()} />}
      </Show>
    </Show>
  );
}

function MainApp(props: { session: Session; boot: BootInfo }) {
  const { setOnline, setLastSyncAt, logout } = useSession();
  const { app, view, setView } = createWorkspaceRuntime({
    session: props.session,
    boot: props.boot,
    setOnline,
    setLastSyncAt,
    logout,
  });

  return <Workspace app={app} view={view} setView={setView} />;
}
