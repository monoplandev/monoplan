// The imperative runtime behind the workspace: it owns one `SyncEngine`
// + synced store + op-log persistence per booted doc, the device-row
// writes, and the single WebSocket sync bridge that multiplexes every
// doc's engine for one booted session. `MainApp` calls this
// once during setup and renders the returned `app` / `view` — all the
// lifecycle (effects, `onCleanup`, the visibility handler) registers
// against the calling component's reactive owner, so this must be
// invoked synchronously from a component body, not an async callback.

import {
  createEffect,
  createSignal,
  getOwner,
  on,
  onCleanup,
  runWithOwner,
} from "solid-js";
import { Dek, Doc, SyncEngine, wrapDekWithDek } from "@monoplan/core/wasm";
import { IdbStorage, type InFlightPushJs, putDevice } from "@monoplan/core";
import { api } from "../api.ts";
import { savePrefs, type Prefs, type ViewKey } from "../prefs.ts";
import { type Session } from "../Login.tsx";
import { dekVault } from "./dekVault.ts";
import { createDocsLayer, type DocsLayer } from "./docs.ts";
import { createSyncedApp, type DocApp } from "./store.ts";
import { createSyncBridge, SyncBridge } from "./sync.ts";

// Onboarding items for a brand-new account's "Welcome" list — learn by
// doing: the item text names the gesture, and completing it (ticking the
// box) is itself the lesson. The first item's descriptive line lives in
// its notes rather than as its own checkbox. Kept literal (user-list
// content is user-authored plain text, not localized).
const WELCOME_ITEMS = [
  "Welcome to Monoplan",
  "Press 'space' to create a new item",
  "Tick the box (or press 'x') to complete an item",
  "Press 's' to star an item",
  "Press '?' to see all keyboard shortcuts",
] as const;
const WELCOME_NOTE =
  "Monoplan helps you capture and organise your ideas, tasks, and projects.";

/** Seed a fresh account's starter list and onboarding items; returns the
 *  new list's id so the caller can open it. The list is freshly created
 *  (empty), so inserting the batch at index 0 keeps the authored order —
 *  and sidesteps the wasm32 `usize` (32-bit) overflow a "very large append
 *  index" would hit. */
function seedWelcome(app: DocApp): string {
  const welcomeId = app.addList("Welcome");
  app.setListIcon(welcomeId, "👋");
  const ids = app.addItemsAt(welcomeId, [...WELCOME_ITEMS], 0);
  if (ids[0]) app.setItemNotes(ids[0], WELCOME_NOTE);
  return welcomeId;
}

/** One doc rebuilt from its op log by `BootGate`: the doc, the durable
 *  cursors, and the opened store. */
export type DocBoot = {
  docId: string;
  /** The doc's DEK as held by the session (not consumed; the engine
   *  gets a clone). */
  dek: Dek;
  doc: Doc;
  lastAcked: bigint;
  /** Engine op-log store (the engine stores of IndexedDB `monoplan-web`).
   *  Mandatory — the
   *  engine has no storage-less mode; if IDB can't be opened the boot
   *  fails hard (see `BootGate`) rather than running without local
   *  persistence. */
  storage: IdbStorage;
  /** Highest `localSeq` the store has assigned — seeds the engine. */
  lastLocalSeq: number;
  /** Encoded `serverKnownVv` (empty = never synced) — seeds the
   *  engine's outbound-delta base. */
  serverKnownVv: Uint8Array;
  /** Durable in-flight push left by a previous session, if any —
   *  re-sent (same pushId) after the initial pull. */
  inFlightPush: InFlightPushJs | null;
  /** Surviving WAL rows / payload bytes past the last snapshot —
   *  seeds the engine's fold-threshold statistics. */
  walRows: number;
  walBytes: number;
  /** True when the doc was freshly created (`Doc.create()`) and its
   *  seeded built-ins still need an initial `captureLocalOps`. */
  seeded: boolean;
};

/** Everything `BootGate` rebuilds before the runtime can start: every
 *  doc the session can open (Home first) and the account's prefs. */
export type BootInfo = {
  docs: DocBoot[];
  /** The Loro peer id every doc was bound to (slot 0 while this tab
   *  holds the single-tab lock), or null for a random peer. A doc
   *  created mid-session is bound to the same one. */
  peerId: bigint | null;
  /** Whatever was in the `prefs` store for this account at boot. Null
   *  if the row was missing (first run for this account on this
   *  device). */
  prefs: Prefs | null;
};

/** The per-doc half of the runtime: one engine, one synced store, and
 *  the op-log persistence rhythm around them. Every doc gets one; the
 *  bridge multiplexes their engines over a single socket. */
type DocRuntime = {
  app: DocApp;
  engine: SyncEngine;
  /** Write the just-committed mutation as a durable op row. Must run
   *  before `engine.flush()`. */
  capture: () => void;
  /** The "we just round-tripped with the server for this doc" pulse:
   *  drain events, fold if the WAL crossed the threshold, re-arm the
   *  idle fold, ratchet the durable cursor. */
  onServerFrame: () => void;
  /** The after-local-flush pulse (everything but the drain). */
  onLocalFlush: () => void;
  scheduleDurable: () => void;
  foldNow: () => void;
  /** Resolves once queued IDB writes for this doc have landed. */
  flushed: () => Promise<void>;
  /** Stop the persistence rhythm (the idle fold timer). Also runs on the
   *  owner's cleanup; called early when a doc is removed mid-session. */
  dispose: () => void;
};

/** Build one doc's engine + store + persistence rhythm. `pumpOutbox`
 *  is resolved lazily because the bridge that owns it is constructed
 *  after every doc runtime exists. Registers `onCleanup` against the
 *  calling owner. */
function createDocRuntime(
  boot: DocBoot,
  pumpOutbox: () => void,
): DocRuntime {
  const storage = boot.storage;
  // `SyncEngine` consumes its Dek argument, and the session-level Dek
  // must stay valid across MainApp remounts (anonymous → authed swap
  // remounts this whole tree on the Session key). So clone for the
  // engine; the original on the session is untouched. The `IdbStorage`
  // mirror is handed in as the engine's `EngineStorage` — capture / ack
  // / remote-apply now flow through it.
  const engine = new SyncEngine(
    boot.doc,
    boot.docId,
    boot.dek.clone(),
    boot.lastAcked,
    storage,
  );
  // Seed the engine's persisted cursors from what the store loaded:
  // the localSeq counter, the WAL fold statistics, the server-known
  // VV (the outbound-delta base), and any in-flight push a previous
  // session left durable (re-sent with the same pushId after the
  // initial pull; the server deduplicates).
  engine.setLastLocalSeq(boot.lastLocalSeq);
  engine.seedWalStats(boot.walRows, boot.walBytes);
  engine.seedServerKnownVv(boot.serverKnownVv);
  const inFlight = boot.inFlightPush;
  if (inFlight) {
    engine.seedInFlightPush(
      inFlight.pushId,
      inFlight.ciphertext,
      inFlight.nonce,
      inFlight.fromVv,
      inFlight.toVv,
    );
  }
  const app = createSyncedApp(engine);

  // ---------- Engine op-log persistence (IdbStorage) ----------
  //
  // The engine's `LocalStorage` trait does the heavy lifting: local
  // commits become durable op rows via `captureLocalOps`, remote ops are
  // mirrored inside `handleServerBytes`, and the outbox drives the push.
  // The web host's job is just the CLI's `persist_engine_state` rhythm,
  // adapted for IDB's async durability:
  //
  //   - `capture()` (before every `flush()`) writes the just-committed
  //     mutation to a durable op row *before* the outbox-driven push
  //     reads it. This must precede `engine.flush()`.
  //   - `compact()` folds a fully-synced log into a fresh snapshot.
  //   - `scheduleDurable()` samples the contiguous seq, waits for the
  //     IDB write to actually land, then tells the engine the bytes are
  //     durable (which queues the `Ack`) and pumps it onto the wire.
  const capture = (): void => {
    try {
      engine.captureLocalOps();
    } catch (e) {
      console.error("captureLocalOps failed:", e);
    }
  };
  // Snapshot export is O(doc) on the main thread (tens of ms at ~10k
  // lifetime items under wasm), and this runs on the per-ack pulse —
  // i.e. one RTT after *every* local mutation. Unthresholded it turned
  // each keystroke into a whole-doc export; see spec/list-perf-plan.md.
  // So the hot pulse only folds once the WAL crosses ≥250 rows (or the
  // byte safety net), and a quiet-period timer (below) folds whatever's
  // left so short sessions don't boot into a long WAL replay. Folding
  // is independent of sync state (spec/vv-wal-separation.md): unsent
  // local ops survive inside the full-history snapshot, so anonymous
  // and authed sessions compact identically.
  const COMPACT_MAX_WAL_ROWS = 250;
  const COMPACT_MAX_WAL_BYTES = 8 * 1024 * 1024;
  const COMPACT_IDLE_MS = 20_000;
  const compactIfWalExceeds = (): void => {
    try {
      engine.snapshotIfWalExceeds(COMPACT_MAX_WAL_ROWS, COMPACT_MAX_WAL_BYTES);
    } catch (e) {
      console.error("snapshotIfWalExceeds failed:", e);
    }
  };
  const foldNow = (): void => {
    try {
      engine.forceSnapshot();
    } catch (e) {
      console.error("forceSnapshot failed:", e);
    }
  };
  // Debounced idle fold-down: re-armed on every server round-trip, so
  // it fires once things go quiet and folds any remaining WAL rows.
  let compactIdleTimer: ReturnType<typeof setTimeout> | undefined;
  const scheduleIdleCompact = (): void => {
    clearTimeout(compactIdleTimer);
    compactIdleTimer = setTimeout(foldNow, COMPACT_IDLE_MS);
  };
  const dispose = (): void => clearTimeout(compactIdleTimer);
  onCleanup(dispose);

  // Sample the contiguous seq now, wait for queued IDB writes to land,
  // then mark durable + ship the resulting Ack. Sampling synchronously
  // binds the notify to bytes that were actually being persisted.
  const scheduleDurable = (): void => {
    const seq = engine.lastContiguousSeq();
    storage
      .whenFlushed()
      .then(() => {
        engine.notifyOplogDurable(seq);
        pumpOutbox();
      })
      .catch((e) => {
        console.error("durable flush failed:", e);
      });
  };

  // Capture local commits before the push reads the outbox — both
  // anonymous and authed need durable op rows.
  app.setBeforeFlush(capture);

  const onLocalFlush = (): void => {
    compactIfWalExceeds();
    scheduleIdleCompact();
    scheduleDurable();
  };
  const onServerFrame = (): void => {
    app.drainEvents();
    // The engine already mirrored remote ops into the WAL inside
    // `handleServerBytes`. Fold if the WAL crossed the threshold
    // (this pulse fires one RTT after every mutation), then
    // ratchet the durable cursor once the writes land.
    onLocalFlush();
  };

  return {
    app,
    engine,
    capture,
    onServerFrame,
    onLocalFlush,
    scheduleDurable,
    foldNow,
    flushed: () => storage.whenFlushed(),
    dispose,
  };
}

/** Hook a doc runtime's after-flush pulse: push the outbox over the
 *  bridge (authed) and run the local persistence rhythm. */
function wireFlush(r: DocRuntime, bridge: SyncBridge | null): void {
  r.app.setOnFlush(() => {
    bridge?.pumpOutbox();
    r.onLocalFlush();
  });
}

export function createWorkspaceRuntime(props: {
  session: Session;
  boot: BootInfo;
  setOnline: (b: boolean) => void;
  setLastSyncAt: (ts: number | null) => void;
  logout: () => void;
}) {
  let bridge: SyncBridge | null = null;
  const pumpOutbox = (): void => bridge?.pumpOutbox();
  // One runtime per booted doc, keyed by doc id. The Home doc's `app`
  // is what the workspace renders today; the others sync headlessly
  // until the cross-doc lenses land (`spec/sharing-plan.md` Phase 5).
  const runtimes = new Map<string, DocRuntime>();
  for (const b of props.boot.docs) {
    runtimes.set(b.docId, createDocRuntime(b, pumpOutbox));
  }
  const home = runtimes.get(props.session.primaryDocId);
  if (!home) throw new Error("boot has no Home doc");
  const app = home.app;
  const docApps = new Map<string, DocApp>(
    [...runtimes].map(([id, r]) => [id, r.app]),
  );
  // The personal layer: doc map, reverse index, routed mutations and
  // the merged lenses (`docs.ts`). The UI renders from this.
  const docs: DocsLayer = createDocsLayer(
    app,
    [...runtimes.values()].map((r) => r.app),
  );
  // Reactive owner of this runtime, so a doc created later from an
  // event handler can register its cleanups against the same lifetime.
  const owner = getOwner();

  if (import.meta.env.DEV) {
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

  // Workspace view lives at this level so the prefs-save effect
  // below can persist it independently of the device-frontier write.
  // Seed from the prefs row; a `kind:"list"` pointing at a
  // since-deleted list falls back to Inbox silently. `focus`/`done`/`bin`
  // are global views and always resolvable.
  // A list view names its doc (`ViewKey.docId`, absent ≡ Home); the
  // canonical form drops a Home doc id so equality, prefs and URLs
  // agree. Every write goes through `setView`, which normalises.
  const normalizeView = (v: ViewKey): ViewKey =>
    v.kind === "list" ? docs.listView(docs.viewDocId(v), v.id) : v;
  const initialView: ViewKey = (() => {
    const v = props.boot.prefs?.currentView;
    if (!v) return { kind: "list", id: "inbox" };
    if (v.kind === "list") {
      const target = docs.appForView(v);
      if (v.id !== "inbox" && !target.state.listsById[v.id]) {
        return { kind: "list", id: "inbox" };
      }
      return normalizeView(v);
    }
    return v;
  })();
  const [view, setViewRaw] = createSignal<ViewKey>(initialView);
  const setView = (v: ViewKey): void => {
    setViewRaw(normalizeView(v));
  };

  // Persist the device row — identity + the "last synced" stamp
  // (observability only). The resume cursor is no longer stored here:
  // the engine owns it and persists it itself via
  // `IdbStorage.writeAckedSeq` (clamped to the durable contiguous
  // frontier). Anonymous sessions skip — no server identity to record.
  const persistDeviceNow = async (): Promise<void> => {
    if (props.session.anonymous) return;
    await putDevice(props.session.accountId, {
      accountId: props.session.accountId,
      primaryDocId: props.session.primaryDocId,
      email: props.session.email!,
      serverUrl: window.location.origin,
      deviceId: props.session.deviceId!,
      lastSyncAt: Date.now(),
    });
  };

  // Anonymous sessions are local-only by definition — no account on the
  // server to authenticate to. Skip the WebSocket pump entirely; local
  // commits still persist through each runtime's `capture()`.
  if (!props.session.anonymous) {
    bridge = createSyncBridge({
      // Every booted doc's engine; the bridge routes inbound frames by
      // `doc_id` and stamps outbound ones.
      engines: [...runtimes.values()].map((r) => r.engine),
      onChange: (kind) => {
        if (kind === "online") {
          props.setOnline(true);
          props.setLastSyncAt(Date.now());
        }
        if (kind === "offline") props.setOnline(false);
        // `drain` fires after every recv-frame pump and every outbox
        // flush, so it's the natural pulse for "we just round-tripped
        // with the server" — even when no app events were produced.
        if (kind === "drain") props.setLastSyncAt(Date.now());
      },
      onServerFrame: (docId) => {
        runtimes.get(docId)?.onServerFrame();
        saveDeviceSoon();
      },
      onAuthFailed: () => void props.logout(),
    });
    const b = bridge;
    for (const r of runtimes.values()) wireFlush(r, b);
    bridge.start();
    onCleanup(() => b.stop());
  } else {
    // Anonymous sessions have no server round-trips to pulse off, but
    // the WAL still needs bounding — fold on the same thresholds after
    // every local flush.
    for (const r of runtimes.values()) wireFlush(r, null);
  }

  // ---------- Device config: light writes on each frontier change ----------
  //
  // Refreshes the "last synced" stamp (observability) on a coarse
  // debounce. The resume cursor is persisted separately by the engine
  // (`IdbStorage.writeAckedSeq`), so this no longer carries it.
  let deviceTimer: ReturnType<typeof setTimeout> | null = null;
  const saveDeviceSoon = (): void => {
    if (props.session.anonymous) return;
    if (deviceTimer) clearTimeout(deviceTimer);
    deviceTimer = setTimeout(() => {
      deviceTimer = null;
      void persistDeviceNow().catch((e) => {
        console.error("device save failed:", e);
      });
    }, 500);
  };
  const watchVersion = (r: DocRuntime): void => {
    createEffect(() => {
      r.app.version();
      saveDeviceSoon();
    });
  };
  for (const r of runtimes.values()) watchVersion(r);

  // Create a doc the account owns and open it in this session
  // (`spec/sharing-plan.md` Phase 5): a fresh DEK wrapped under the Home
  // DEK is registered with the server, vaulted, and the doc is seeded
  // like Home (`Doc.create`), booted into its own runtime, routed over
  // the live bridge and named. Rejected on anonymous sessions, which
  // have no server to own the doc. The returned promise resolves to the
  // new doc id once it is in `docs`.
  const createDoc = async (name: string): Promise<string> => {
    if (props.session.anonymous) {
      throw new Error("sign in to create a workspace");
    }
    const dek = Dek.generate();
    const wrapped = wrapDekWithDek(props.session.dek, dek);
    const { doc_id: docId } = await api.createDoc({
      wrapped_dek: wrapped.ciphertext,
      wrapped_dek_nonce: wrapped.nonce,
    });
    props.session.docKeys.set(docId, dek);
    try {
      await dekVault.putDocKey(docId, dek);
    } catch (e) {
      // The doc still opens this session; the next boot's `GET /api/docs`
      // refresh re-learns it (`App.tsx` `refreshDocKeys`).
      console.warn("createDoc: vaulting the new doc failed:", e);
    }
    const storage = await IdbStorage.open(docId);
    const peerId = props.boot.peerId;
    const doc = peerId !== null ? Doc.createWithPeer(peerId) : Doc.create();
    const boot: DocBoot = {
      docId,
      dek,
      doc,
      lastAcked: 0n,
      storage,
      lastLocalSeq: 0,
      serverKnownVv: new Uint8Array(),
      inFlightPush: null,
      walRows: 0,
      walBytes: 0,
      seeded: true,
    };
    const r = runWithOwner(owner, () => createDocRuntime(boot, pumpOutbox));
    if (!r) throw new Error("runtime owner is gone");
    runtimes.set(docId, r);
    docApps.set(docId, r.app);
    wireFlush(r, bridge);
    runWithOwner(owner, () => watchVersion(r));
    // Name first so the seed and the name travel as one WAL row / one
    // push, then persist and route.
    if (name.trim()) r.app.setDocName(name);
    r.capture();
    r.scheduleDurable();
    bridge?.addEngine(r.engine);
    docs.addDoc(r.app);
    return docId;
  };

  // Delete a doc the account owns and forget it on this device. The
  // server goes first (owner-only; it refuses the primary doc, as does
  // the guard here — Home is non-deletable, `spec/sharing-plan.md`); a
  // refusal leaves everything in place. Then, in an order where a
  // failure part-way leaves nothing half-alive: the view leaves the doc,
  // the engine stops routing, the layer and runtime maps forget it, and
  // the key (session + vault) and the local op log go last. Other
  // devices learn on their next boot (`refreshDocKeys` in App.tsx); a
  // device with the doc open right now is not told — `DocClosed` is
  // future work.
  const deleteDoc = async (docId: string): Promise<void> => {
    if (props.session.anonymous) {
      throw new Error("sign in to delete a workspace");
    }
    if (docId === props.session.primaryDocId) {
      throw new Error("the Home workspace cannot be deleted");
    }
    const r = runtimes.get(docId);
    if (!r) throw new Error(`deleteDoc: no open doc ${docId}`);
    await api.deleteDoc(docId);
    if (docs.viewDocId(view()) === docId) setView({ kind: "list", id: "inbox" });
    bridge?.removeEngine(docId);
    docs.removeDoc(docId);
    runtimes.delete(docId);
    docApps.delete(docId);
    r.dispose();
    props.session.docKeys.delete(docId);
    try {
      await dekVault.removeDocKey(docId);
    } catch (e) {
      // Next boot's `GET /api/docs` refresh drops it from the vault.
      console.warn("deleteDoc: removing the vault entry failed:", e);
    }
    try {
      await r.flushed();
      await IdbStorage.wipe(docId);
    } catch (e) {
      console.warn("deleteDoc: wiping the local op log failed:", e);
    }
  };

  // ---------- Prefs: separate store, write-through ----------
  //
  // View state ("which list / Done / Bin am I on?") lives in its own
  // IDB store keyed per account. Decoupled from `device` so a burst
  // of typing doesn't rewrite the view, and switching lists doesn't
  // rewrite the sync frontier. Anonymous sessions persist too — the
  // device row gate above is about identity, not local-only UI state.
  //
  // No debounce: view changes are user-initiated (click / keyboard
  // nav) and rare on the timescale of an IDB write. Coalescing them
  // would only buy us dropped saves when the tab closes mid-window.
  createEffect(
    on(
      view,
      () => {
        void savePrefs(props.session.accountId, {
          currentView: view(),
        }).catch((e) => {
          console.error("prefs save failed:", e);
        });
      },
      { defer: true },
    ),
  );

  // Freshly-created doc (signup, or a brand-new anonymous doc): seed a
  // friendly starter list so the workspace opens with more than an empty
  // Inbox, then persist as the first op row so a reload before any user
  // mutation still has something to replay. Seeding goes through the
  // normal mutation path, so it commits and (for authed sessions) pushes
  // to the server via the outbox; other devices receive it through sync.
  // Genesis in the core stays a clean empty baseline (see App.tsx boot);
  // the account-creating client is the single seeder, so no duplicate
  // arises. Guard on emptiness so a re-entrant boot can't double-seed.
  for (const b of props.boot.docs) {
    if (!b.seeded) continue;
    const r = runtimes.get(b.docId)!;
    if (r === home && Object.keys(app.state.listsById).length === 0) {
      const welcomeId = seedWelcome(app);
      // Open the Welcome list on the very first launch so the onboarding
      // items are what a new user sees (Inbox stays the pure capture
      // surface). Only overrides the default Inbox landing — never a real
      // saved view — and the deferred prefs-save persists it from here on.
      if (!props.boot.prefs?.currentView) {
        setView({ kind: "list", id: welcomeId });
      }
    }
    r.capture();
    r.scheduleDurable();
  }

  const onVisibility = () => {
    if (document.visibilityState === "hidden") {
      if (deviceTimer) clearTimeout(deviceTimer);
      // Persist any uncommitted local op, then fold the whole WAL so
      // the next boot has a fresh base — safe regardless of sync state
      // (unsent ops survive folding; the push derives them from
      // `serverKnownVv`). Best-effort, every doc.
      for (const r of runtimes.values()) {
        r.capture();
        r.foldNow();
      }
      void persistDeviceNow().catch(() => {});
    }
  };
  document.addEventListener("visibilitychange", onVisibility);
  onCleanup(() => {
    document.removeEventListener("visibilitychange", onVisibility);
    if (deviceTimer) clearTimeout(deviceTimer);
  });

  return { app, docApps, docs, view, setView, createDoc, deleteDoc };
}
