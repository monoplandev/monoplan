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
import { api } from "./api.ts";
import { dekVault } from "./sync/dekVault.ts";
import { unwrapNewMemberships } from "./sync/docKeys.ts";
import { useAppI18n } from "./i18n.tsx";
import { type Session } from "./Login.tsx";
import { Workspace } from "./Workspace.tsx";
import { createWorkspaceRuntime, type BootInfo, type DocBoot } from "./sync/runtime.ts";
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
  onMount(async () => {
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
          docKeys: v.docKeys,
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

/** How long boot waits on `GET /api/docs` before going ahead with the
 *  vaulted docs alone. A doc learned after this is picked up next boot. */
const DOC_REFRESH_TIMEOUT_MS = 5000;

/** What `GET /api/docs` taught us at boot: memberships this device
 *  lacked, now unwrapped and vaulted, and vaulted docs the server no
 *  longer lists (deleted on another device; later, left or revoked). */
type DocRefresh = { fresh: Map<string, Dek>; stale: string[] };

/** Learn about docs the account joined since this device last logged in:
 *  `GET /api/docs`, unwrap each membership the session lacks under the
 *  Home DEK, vault it (`putDocKey`), and add it to `session.docKeys` so
 *  it boots with the rest. Also names the vaulted non-primary docs the
 *  server no longer lists, which the caller drops before boot (their
 *  `OpenDoc` would be refused, and a refusal ends the whole session).
 *  Best-effort: offline, a server error, or the timeout leaves the
 *  vaulted set as it is. */
async function refreshDocKeys(session: Session): Promise<DocRefresh> {
  const none: DocRefresh = { fresh: new Map<string, Dek>(), stale: [] };
  if (session.anonymous) return none;
  const timeout = new Promise<null>((resolve) =>
    setTimeout(() => resolve(null), DOC_REFRESH_TIMEOUT_MS),
  );
  let resp: Awaited<ReturnType<typeof api.listDocs>> | null;
  try {
    resp = await Promise.race([api.listDocs(), timeout]);
  } catch (e) {
    console.warn("docs refresh skipped:", e);
    return none;
  }
  if (resp === null) {
    console.warn("docs refresh timed out; booting vaulted docs only");
    return none;
  }
  const fresh = unwrapNewMemberships(
    session.dek,
    session.primaryDocId,
    resp.memberships,
    session.docKeys,
  );
  for (const [docId, dek] of fresh) {
    session.docKeys.set(docId, dek);
    try {
      await dekVault.putDocKey(docId, dek);
    } catch (e) {
      console.warn("docs refresh: vaulting new doc failed:", e);
    }
  }
  const listed = new Set(resp.memberships.map((m) => m.doc_id));
  const stale = [...session.docKeys.keys()].filter(
    (id) => id !== session.primaryDocId && !listed.has(id),
  );
  return { fresh, stale };
}

/** Forget a doc the server no longer lists for this account: its key
 *  (session + vault) and its local op log. Best-effort; a failure leaves
 *  the doc to be dropped again next boot. */
async function forgetDoc(session: Session, docId: string): Promise<void> {
  session.docKeys.delete(docId);
  try {
    await dekVault.removeDocKey(docId);
  } catch (e) {
    console.warn("docs refresh: removing a stale vault entry failed:", e);
  }
  try {
    await IdbStorage.wipe(docId);
  } catch (e) {
    console.warn("docs refresh: wiping a stale doc failed:", e);
  }
}

async function createAnonymousSession(): Promise<Session> {
  const accountId = `anon-${crypto.randomUUID()}`;
  // Anonymous sessions never reach the server, so there's no
  // server-assigned doc id to use. Mint one locally; matches the
  // shape authenticated sessions get from signup/login responses.
  const primaryDocId = crypto.randomUUID();
  const dek = Dek.generate();
  const docKeys = new Map([[primaryDocId, dek]]);
  const session: Session = {
    anonymous: true,
    accountId,
    primaryDocId,
    email: null,
    deviceId: null,
    dek,
    docKeys,
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
      dek,
      docKeys,
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
  // Rebuild every doc the session can open from its engine WAL
  // (`spec/local-storage.md` §"Web boot"), mirroring the CLI's `boot_doc`:
  // load the snapshot (a bare Loro snapshot) and replay every WAL row after
  // it via the deferred replay API, finalize indexes once, then
  // `markPersisted()` so the WAL capture cursor covers the replayed ops
  // (what still needs *uploading* is derived separately from the persisted
  // `serverKnownVv`). Fresh signups — and brand-new anonymous docs with an
  // empty store — start from `Doc.create()` so the seeded built-ins land;
  // authed devices with an empty store (and every non-Home doc first seen
  // on this device) start empty and let sync deliver a snapshot. Clients
  // load every membership at boot (`spec/sharing-plan.md` "Decisions"), one
  // `Doc` + store each; the UI binds to the Home doc.
  onMount(async () => {
    try {
      // Prefs are independent of the op-log replay — fire in parallel so
      // they don't add a serial roundtrip to first paint. A miss (first
      // run for this account on this device) resolves null and `MainApp`
      // falls back to defaults.
      const prefsPromise = loadPrefs(props.session.accountId).catch(() => null);
      // The slot-0 peer claim rides the storage open — holding the
      // single-tab Web Lock (checked by `BrowserTabGate` before this
      // mounts) IS the claim, so all that's left is reading the minted
      // peer id. No lock → random peer, so a gate-disabled second tab
      // can never share a peer with the holder. One peer serves every
      // doc: op ids are per doc, so independent docs never collide.
      const peerId = props.singleTabLockHeld ? await readOrMintPeerSlot(0) : null;

      // The peer must be bound at construction — before `Doc.create`'s
      // builtin seed commits and before the replay import (Loro resumes
      // the slot peer's counter from the imported history).
      const create = () =>
        peerId !== null ? Doc.createWithPeer(peerId) : Doc.create();
      const empty = () =>
        peerId !== null ? Doc.emptyWithPeer(peerId) : Doc.empty();

      const bootDoc = async (docId: string, dek: Dek): Promise<DocBoot> => {
        const isHome = docId === props.session.primaryDocId;
        const storage = await IdbStorage.open(docId);
        const rows = storage.bootRows();
        let doc: Doc;
        let seeded = false;
        if (isHome && props.session.freshSignup) {
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
        } else if (isHome && props.session.anonymous) {
          // Brand-new (or wiped) local-only doc — seed the built-ins.
          doc = create();
          seeded = true;
        } else {
          // Authed device, empty local store — sync will send a snapshot.
          doc = empty();
        }
        return {
          docId,
          dek,
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
        };
      };

      // Ask the server for memberships this device has not vaulted (a
      // doc created or joined elsewhere since login). Fired now and
      // awaited after the vaulted docs have replayed, so the common
      // path pays no extra latency; see `refreshDocKeys`.
      const refresh = refreshDocKeys(props.session);

      // Home first, so a failure there is the one reported; replay is
      // main-thread wasm work either way, so there is nothing to gain
      // from interleaving the docs.
      let docs: DocBoot[] = [
        await bootDoc(props.session.primaryDocId, props.session.dek),
      ];
      for (const [docId, dek] of props.session.docKeys) {
        if (docId === props.session.primaryDocId) continue;
        docs.push(await bootDoc(docId, dek));
      }
      const { fresh, stale } = await refresh;
      for (const [docId, dek] of fresh) {
        docs.push(await bootDoc(docId, dek));
      }
      // A vaulted doc the server no longer lists was deleted elsewhere:
      // drop it rather than boot it (its replay above was wasted work on
      // a rare path, which beats delaying the common path on the fetch).
      if (stale.length > 0) {
        docs = docs.filter((d) => !stale.includes(d.docId));
        for (const docId of stale) await forgetDoc(props.session, docId);
      }

      props.setBoot({ docs, peerId, prefs: await prefsPromise });
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
  const { docs, view, setView, createDoc, joinDoc, deleteDoc } = createWorkspaceRuntime({
    session: props.session,
    boot: props.boot,
    setOnline,
    setLastSyncAt,
    logout,
  });

  return (
    <Workspace
      docs={docs}
      view={view}
      setView={setView}
      createDoc={createDoc}
      joinDoc={joinDoc}
      deleteDoc={deleteDoc}
    />
  );
}
