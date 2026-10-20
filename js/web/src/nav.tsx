import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { Popover } from "@kobalte/core/popover";
import { Tooltip } from "@kobalte/core/tooltip";
import { createPopoverTooltipGuard } from "./popoverTooltip.ts";
import { ConfirmDialog } from "./ConfirmDialog.tsx";
import type { AttentionBadge } from "./dayGroups.ts";
import { Dnd, DndSelection, type DndOp } from "./dnd/solid";
import archiveSvg from "./icons/archive.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import caretDownSvg from "./icons/caret-down.svg?raw";
import checkSvg from "./icons/check.svg?raw";
import cloudSvg from "./icons/cloud.svg?raw";
import cloudOffSvg from "./icons/cloud-off.svg?raw";
import crumpledPaperSvg from "./icons/crumpled-paper.svg?raw";
import dotsHorizontalSvg from "./icons/dots-horizontal.svg?raw";
import dotsVerticalSvg from "./icons/dots-vertical.svg?raw";
import starSvg from "./icons/star.svg?raw";
import externalLinkSvg from "./icons/external-link.svg?raw";
import fileSvg from "./icons/file.svg?raw";
import magnifyingGlassSvg from "./icons/magnifying-glass.svg?raw";
import plusSvg from "./icons/plus.svg?raw";
import { formatRelative } from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import { AuthDialog, type Session } from "./Login.tsx";
import { pasteAsPlainText } from "./plainTextPaste.ts";
import type { ViewKey } from "./prefs.ts";
import { listUrl } from "./url.ts";
import type { DocEntry, DocsLayer } from "./sync/docs.ts";
import type { DocApp, ListView } from "./sync/store.ts";

// Whether the anonymous-session sign-in prompt has been dismissed. Auto-
// opening the auth dialog on every load is nagging once the user has
// deliberately closed it, so we persist a single flag and skip the
// auto-open while it's set. Purely local UI state — same localStorage
// rationale as the board prefs in Workspace.tsx. Cleared on logout (see
// App.tsx) so signing out re-prompts. The user can still reopen the
// dialog any time via the cloud-off indicator.
const AUTH_DISMISSED_KEY = "monoplan:auth-prompt-dismissed";
export function loadAuthPromptDismissed(): boolean {
  try {
    return localStorage.getItem(AUTH_DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}
export function clearAuthPromptDismissed(): void {
  try {
    localStorage.removeItem(AUTH_DISMISSED_KEY);
  } catch {
    // Best-effort; a stuck flag only means the prompt stays dismissed.
  }
}
function markAuthPromptDismissed(): void {
  try {
    localStorage.setItem(AUTH_DISMISSED_KEY, "1");
  } catch {
    // Best-effort; failing to persist just means we re-prompt next load.
  }
}

// Whether a doc's nav section (its Inbox + lists) is collapsed. One
// collapsible heading per open doc, the Home ("Personal") one first;
// Home keeps the key it had before other docs existed. Purely local UI
// state, same localStorage rationale as the flag above.
const PERSONAL_COLLAPSED_KEY = "monoplan:nav-personal-collapsed";
const sectionCollapsedKey = (docId: string, home: boolean): string =>
  home ? PERSONAL_COLLAPSED_KEY : `monoplan:nav-collapsed:${docId}`;
function loadSectionCollapsed(docId: string, home: boolean): boolean {
  try {
    return localStorage.getItem(sectionCollapsedKey(docId, home)) === "1";
  } catch {
    return false;
  }
}
function saveSectionCollapsed(docId: string, home: boolean, collapsed: boolean): void {
  try {
    const key = sectionCollapsedKey(docId, home);
    if (collapsed) localStorage.setItem(key, "1");
    else localStorage.removeItem(key);
  } catch {
    // Best-effort; failing to persist just means the group reopens next load.
  }
}

function ConnectionStatusPopover(props: {
  class?: string;
  app: DocApp;
  online: boolean;
  lastSyncAt: number | null;
}) {
  const { m, locale } = useAppI18n();
  // Tooltip + popover share the trigger; the guard keeps the tooltip quiet
  // while the popover is open and across the focus-restore on close.
  const guard = createPopoverTooltipGuard();
  const open = guard.open;
  // Local seconds-resolution clock — only ticks while the popover is
  // visible so we don't spend a 5s interval forever just to drive a
  // string the user can't see. Falls back to a one-shot read when
  // closed (the sub-minute values won't refresh, but the popover
  // re-opens with fresh values anyway).
  const [tickNow, setTickNow] = createSignal(Date.now());
  createEffect(() => {
    if (!open()) return;
    setTickNow(Date.now());
    const id = setInterval(() => setTickNow(Date.now()), 5_000);
    onCleanup(() => clearInterval(id));
  });

  // Engine-derived state. `app.version()` bumps on every dispatched
  // event, so reading it here re-runs these computations exactly when
  // the underlying numbers can change. Cheap reads — engine just
  // forwards into the doc.
  const pending = (): boolean => {
    props.app.version();
    return props.app.engine.hasUnsyncedOps();
  };
  const seqLabel = (): string => {
    props.app.version();
    return String(props.app.engine.lastContiguousSeq());
  };
  // Epoch arrives on HelloAck, which dispatches no event of its own,
  // so also key on the link state: it flips to online right after.
  const epochLabel = (): string | null => {
    props.app.version();
    props.online;
    const epoch = props.app.engine.serverEpoch();
    return epoch === undefined ? null : String(epoch);
  };
  const fingerprintHex = (): string => {
    props.app.version();
    const buf = props.app.engine.fingerprint();
    // Full 64-char hex; the popover row CSS-truncates with
    // text-overflow so the visible width tracks the popover, while
    // copy-paste yields the entire hash.
    let s = "";
    for (let i = 0; i < buf.length; i++) {
      s += buf[i].toString(16).padStart(2, "0");
    }
    return s;
  };
  const itemsCount = (): number =>
    Object.keys(props.app.state.itemsById).length;
  const listsCount = (): number => props.app.state.listsOrder.length;

  // Browser network reachability, distinct from `props.online` (the WS
  // link). Lets us split "no network at all" (Offline) from "network is
  // up but we're not connected to the server" (Disconnected).
  const [navOnline, setNavOnline] = createSignal(navigator.onLine);
  const onNet = () => setNavOnline(navigator.onLine);
  window.addEventListener("online", onNet);
  window.addEventListener("offline", onNet);
  onCleanup(() => {
    window.removeEventListener("online", onNet);
    window.removeEventListener("offline", onNet);
  });

  // Single source of truth for the sync state, shared by the cloud
  // indicator's inline label and the popover's first line.
  const status = (): "offline" | "disconnected" | "syncing" | "synced" => {
    if (!navOnline()) return "offline";
    if (!props.online) return "disconnected";
    return pending() ? "syncing" : "synced";
  };
  const statusLabel = (): string => {
    switch (status()) {
      case "offline":
        return m().nav.offline;
      case "disconnected":
        return m().nav.disconnected;
      case "syncing":
        return m().nav.syncing;
      default:
        return m().nav.synced;
    }
  };

  const sinceLabel = (): string | null => {
    const ts = props.lastSyncAt;
    if (!ts) return null;
    const now = tickNow();
    const diff = now - ts;
    const r = m().relative;
    if (diff < 5_000) return m().nav.lastSynced(r.justNow);
    if (diff < 60_000) {
      return m().nav.lastSynced(r.secondsAgo(Math.floor(diff / 1000)));
    }
    // ≥ 1 min: defer to the shared formatter for minutes/hours/days.
    return m().nav.lastSynced(formatRelative(ts, now, locale()));
  };

  return (
    <Popover {...guard.popover} placement="top-start" gutter={6}>
      <Tooltip {...guard.tooltip} openDelay={200} closeDelay={0} placement="top">
        <Tooltip.Trigger
          as={Popover.Trigger}
          class={props.class ? `connection-indicator ${props.class}` : "connection-indicator"}
          tabIndex={-1}
          aria-label={statusLabel()}
        >
          <Show
            when={props.online}
            fallback={<span class="connection-icon" innerHTML={cloudOffSvg} />}
          >
            <span class="connection-icon" innerHTML={cloudSvg} />
          </Show>
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content class="tooltip-content">
            <span class="status-dot" data-state={status()} aria-hidden="true" />
            {statusLabel()}
            <Tooltip.Arrow />
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip>
      <Popover.Portal>
        <Popover.Content {...guard.content} class="status-popover">
          <div class="status-line">
            <span
              class="status-dot"
              data-state={status()}
              aria-hidden="true"
            />
            <span>{statusLabel()}</span>
          </div>
          <Show when={sinceLabel()}>
            {(label) => <div class="status-line status-muted">{label()}</div>}
          </Show>
          <div class="status-line status-muted">{m().nav.seqLabel(seqLabel())}</div>
          <Show when={epochLabel()}>
            {(n) => (
              <div class="status-line status-muted">{m().nav.epochLabel(n())}</div>
            )}
          </Show>
          <div class="status-fingerprint status-muted status-mono">
            {fingerprintHex()}
          </div>
          <div class="status-line status-muted">
            {m().nav.itemsListsCount(itemsCount(), listsCount())}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
}

/** The account/sync widget: a sign-in prompt while anonymous, the
 *  connection-status popover once authed. Lives in the sidebar footer
 *  beside the app menu (see `Workspace`), rendered exactly once, so its
 *  auto-open-on-mount auth dialog fires only once. */
/** The first-run sign-in prompt on its own: for shells where the
 *  indicator isn't mounted at boot (mobile keeps it in the Find page),
 *  so the prompt still fires on load. Same dismissed-flag rules as
 *  `StatusSlot`'s built-in prompt. */
export function FirstRunAuthPrompt(props: {
  session: Session;
  onSession: (s: Session) => void;
}) {
  const [authOpen, setAuthOpen] = createSignal(
    props.session.anonymous && !loadAuthPromptDismissed(),
  );
  const onOpenChange = (open: boolean) => {
    if (!open) markAuthPromptDismissed();
    setAuthOpen(open);
  };
  return (
    <Show when={props.session.anonymous}>
      <AuthDialog
        open={authOpen()}
        onOpenChange={onOpenChange}
        onSession={(s) => {
          setAuthOpen(false);
          props.onSession(s);
        }}
      />
    </Show>
  );
}

export function StatusSlot(props: {
  /** Extra class on the indicator button (mobile's glass chrome). */
  class?: string;
  app: DocApp;
  online: boolean;
  lastSyncAt: number | null;
  session: Session;
  onSession: (s: Session) => void;
  /** `false` when a `FirstRunAuthPrompt` elsewhere owns the first-run
   *  prompt, so this slot only opens sign-in on tap. Default true. */
  autoPrompt?: boolean;
}) {
  const { m } = useAppI18n();
  // Auto-prompt anonymous users on first mount unless they've dismissed the
  // prompt before (persisted in localStorage). Closing sets that flag so
  // reloads don't re-nag. The whole workspace remounts on session swap
  // (App's keyed <Show>); logout clears the flag so it re-prompts.
  const [authOpen, setAuthOpen] = createSignal(
    props.autoPrompt !== false &&
      props.session.anonymous &&
      !loadAuthPromptDismissed(),
  );
  const onOpenChange = (open: boolean) => {
    if (!open) markAuthPromptDismissed();
    setAuthOpen(open);
  };
  const handleSession = (s: Session) => {
    setAuthOpen(false);
    props.onSession(s);
  };
  return (
    <>
      <Show when={props.session.anonymous}>
        {/* Anonymous sessions show the same cloud-off indicator as an
            offline authed session; clicking it opens the sign-in dialog
            rather than a status popover. */}
        <button
          type="button"
          class={props.class ? `connection-indicator ${props.class}` : "connection-indicator"}
          tabIndex={-1}
          aria-label={m().auth.signIn}
          title={m().auth.signIn}
          onClick={() => setAuthOpen(true)}
        >
          <span innerHTML={cloudOffSvg} />
        </button>
        <AuthDialog
          open={authOpen()}
          onOpenChange={onOpenChange}
          onSession={handleSession}
        />
      </Show>
      <Show when={!props.session.anonymous}>
        <ConnectionStatusPopover
          class={props.class}
          app={props.app}
          online={props.online}
          lastSyncAt={props.lastSyncAt}
        />
      </Show>
    </>
  );
}

export function Nav(props: {
  /** The personal layer: every open doc, Home first (`sync/docs.ts`).
   *  One collapsible section renders per doc; there is no "current
   *  doc", the view names its own. */
  docs: DocsLayer;
  /** Merged across docs (the Bin always spans every doc). */
  binCount: number;
  /** Number of visible Focus entries — the Focus nav entry's count badge, and
   *  what drives the soft "getting big" signal past the threshold. */
  focusCount: number;
  /** The Upcoming entry's badge: Open rows owed or on today. Only the
   *  count is shown here (the tone drives the Find sheet / mobile bar);
   *  hidden at zero. */
  upcomingBadge: AttentionBadge;
  /** Home doc settings flag; when true, render the live-item count
   *  badge beside each non-Inbox list in every section (showing "-" when
   *  the list is empty). Inbox badges are always shown regardless. */
  showListCounts: boolean;
  view: ViewKey;
  setView: (v: ViewKey) => void;
  /** Delete a non-Home doc (`runtime.deleteDoc`), or null when this
   *  session can't (anonymous). Drives the heading menu's Delete item. */
  onDeleteDoc: ((docId: string) => Promise<void>) | null;
  /** Chrome pinned to the bottom of the sidebar (app menu + account/sync
   *  widget) — see `.nav-footer` in styles.css. */
  footer?: JSX.Element;
}) {
  const { m } = useAppI18n();
  const [emptyBinConfirmOpen, setEmptyBinConfirmOpen] = createSignal(false);

  // Match the items list's mobile bump so the drawer's tap targets feel
  // consistent with the main view.
  const navMobileMq = window.matchMedia(
    "(max-width: 768px) and (pointer: coarse)",
  );
  const [navIsMobile, setNavIsMobile] = createSignal(navMobileMq.matches);
  const onNavMqChange = (e: MediaQueryListEvent) => setNavIsMobile(e.matches);
  navMobileMq.addEventListener("change", onNavMqChange);
  onCleanup(() => navMobileMq.removeEventListener("change", onNavMqChange));

  // Keep the active view's nav row in view. The view can change from
  // anywhere (find palette, URL, keyboard, list creation, a click), so
  // react to the view itself rather than to any one entry point; a
  // clicked row is already visible and `block: "nearest"` makes that a
  // no-op. Every entry (Focus, Upcoming, lists, Done, Bin) marks itself
  // with `data-active`, so that's the lookup rather than a per-kind id.
  // The Dnds here aren't scrollers (no fillHeight), so every list row is
  // mounted and `.nav-scroll` is the only scroll container. Deferred a
  // frame (with a couple of retries) so a list created and opened in the
  // same tick has its row in the DOM before we look for it. A section
  // expanding bumps `sectionTick` so the row it reveals is scrolled to.
  let navScrollEl!: HTMLDivElement;
  const [sectionTick, setSectionTick] = createSignal(0);
  const revealActiveNav = () => {
    // Read here so the effect below tracks the view.
    void props.view.kind;
    if (props.view.kind === "list") {
      void props.view.id;
      void props.view.docId;
    }
    let attempts = 0;
    const tryScroll = () => {
      if (!navScrollEl) return;
      const el = navScrollEl.querySelector<HTMLElement>(".nav-item[data-active]");
      if (el) el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      else if (++attempts < 3) requestAnimationFrame(tryScroll);
    };
    requestAnimationFrame(tryScroll);
  };
  createEffect(() => {
    sectionTick();
    revealActiveNav();
  });

  return (
    <nav class="nav">
      <div class="nav-scroll" tabIndex={-1} ref={navScrollEl}>
      <div class="nav-group">
        {/* Focus: a reserved lens (spec/focus.md), not a `ListMeta` row, so
            it's a static entry with a fixed icon — like Done / Bin. Sits at
            the very top: it's the "what am I working on now" answer the
            user reaches for first. */}
        <button
          type="button"
          class="nav-item"
          data-active={props.view.kind === "focus" ? "" : undefined}
          data-drop-focus=""
          tabIndex={-1}
          onClick={() => props.setView({ kind: "focus" })}
        >
          <span class="nav-item-icon" innerHTML={starSvg} />
          {m().nav.focus}
          <Show when={props.focusCount > 0}>
            <span class="nav-item-count">{props.focusCount}</span>
          </Show>
        </button>
        {/* Upcoming: the deadline lens (proto calendar) — a cross-cutting
            view like Done / Bin, so a static entry with a fixed icon. */}
        <button
          type="button"
          class="nav-item"
          data-active={props.view.kind === "upcoming" ? "" : undefined}
          tabIndex={-1}
          onClick={() => props.setView({ kind: "upcoming" })}
        >
          <span class="nav-item-icon" innerHTML={calendarSvg} />
          {m().nav.upcoming}
          <Show when={props.upcomingBadge.count > 0}>
            <span class="nav-item-count">{props.upcomingBadge.count}</span>
          </Show>
        </button>
        <button
          type="button"
          class="nav-item"
          data-active={props.view.kind === "done" ? "" : undefined}
          tabIndex={-1}
          onClick={() => props.setView({ kind: "done" })}
        >
          <span class="nav-item-icon" innerHTML={checkSvg} />
          {m().nav.done}
        </button>
        <Show when={props.binCount > 0}>
          <ContextMenu>
            <ContextMenu.Trigger
              as="button"
              type="button"
              class="nav-item"
              data-active={props.view.kind === "bin" ? "" : undefined}
              data-drop-bin=""
              tabIndex={-1}
              onClick={() => props.setView({ kind: "bin" })}
            >
              <span class="nav-item-icon" innerHTML={crumpledPaperSvg} />
              {m().nav.bin}
              <span class="nav-item-count">{props.binCount}</span>
            </ContextMenu.Trigger>
            <ContextMenu.Portal>
              <ContextMenu.Content class="context-menu-content">
                <ContextMenu.Item
                  class="context-menu-item"
                  onSelect={() => {
                    // Mirror the header button's destructive-action gate —
                    // same in-page confirm, same call.
                    setEmptyBinConfirmOpen(true);
                  }}
                >
                  {m().workspace.emptyBin}
                </ContextMenu.Item>
              </ContextMenu.Content>
            </ContextMenu.Portal>
          </ContextMenu>
        </Show>
      </div>
      {/* One section per open doc, Home ("Personal") first, each with
          its own heading, Inbox, lists and new-list form. Collapse state
          is local to this browser, per doc. */}
      <For each={props.docs.docs()}>
        {(entry) => (
          <DocSection
            entry={entry}
            docs={props.docs}
            view={props.view}
            setView={props.setView}
            showListCounts={props.showListCounts}
            mobile={navIsMobile()}
            onToggled={() => setSectionTick((n) => n + 1)}
            onDeleteDoc={props.onDeleteDoc}
          />
        )}
      </For>
      </div>
      <div class="nav-footer">{props.footer}</div>
      <ConfirmDialog
        open={emptyBinConfirmOpen()}
        onOpenChange={setEmptyBinConfirmOpen}
        message={m().workspace.emptyBinConfirm}
        confirmLabel={m().workspace.emptyBin}
        onConfirm={() => props.docs.emptyBin()}
      />
    </nav>
  );
}

/** Trigger a browser download for an in-memory blob. Anchor + revoke
 *  is the only cross-browser path; FileSystem Access API isn't on
 *  Safari yet. */
function triggerDownload(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

/** "Export JSON" (each workspace heading's "..." menu): pretty-printed
 *  semantic dump (lists + items) of one doc, saved as `<slug>-<date>.json`. Readable in any editor and
 *  round-trips through Import JSON, but lossy: CRDT history, ordering
 *  metadata, and undo-stack info aren't here. (A lossless
 *  plaintext-snapshot export exists in the core — `exportSnapshot` —
 *  but stays unexposed until a matching import lands.) Returns false
 *  on failure so the caller can surface its own alert. */
function exportDocJson(app: DocApp, slug: string): boolean {
  try {
    const json = app.engine.exportJson();
    const blob = new Blob([json], { type: "application/json" });
    const stamp = new Date().toISOString().slice(0, 10);
    triggerDownload(blob, `${slug}-${stamp}.json`);
    return true;
  } catch (err) {
    console.error("export json failed:", err);
    return false;
  }
}

/** File-name slug for a workspace export: the heading label lowercased,
 *  non-alphanumerics collapsed to hyphens, falling back to "monoplan". */
function exportSlug(label: string): string {
  const slug = label
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
  return slug || "monoplan";
}

/** One doc's nav section: a collapsible heading (the doc's name, or
 *  "Personal" for Home), its reserved Inbox, its active lists (one
 *  `Dnd` per section, so a drag never crosses docs), and the new-list
 *  form. Every list row names its doc for the drop targets
 *  (`data-drop-doc-id`) so the Workspace routes a drop to the right
 *  `DocApp`. */
function DocSection(props: {
  entry: DocEntry;
  docs: DocsLayer;
  view: ViewKey;
  setView: (v: ViewKey) => void;
  showListCounts: boolean;
  mobile: boolean;
  onToggled: () => void;
  onDeleteDoc: ((docId: string) => Promise<void>) | null;
}) {
  const { m } = useAppI18n();
  const app = props.entry.app;
  const docId = props.entry.docId;
  // Delete runs through a confirmation: it is server-side, cross-device
  // and not undoable. Home never offers it (`spec/sharing-plan.md`: the
  // primary doc is non-deletable), nor does an anonymous session.
  const canDelete = (): boolean => !props.entry.home && props.onDeleteDoc !== null;
  const [deleteConfirmOpen, setDeleteConfirmOpen] = createSignal(false);
  const deleteDoc = (): void => {
    const run = props.onDeleteDoc;
    if (!run || props.entry.home) return;
    // The collapse flag is per doc id; nothing else will ever clear it.
    saveSectionCollapsed(docId, false, false);
    run(docId).catch((err) => {
      console.error("delete workspace failed:", err);
      alert(m().nav.deleteDocFailed);
    });
  };
  const [adding, setAdding] = createSignal(false);
  const [name, setName] = createSignal("");
  const [collapsed, setCollapsed] = createSignal(loadSectionCollapsed(docId, props.entry.home));
  const setCollapsedPersisted = (next: boolean) => {
    setCollapsed(next);
    saveSectionCollapsed(docId, props.entry.home, next);
    props.onToggled();
  };
  const toggle = () => setCollapsedPersisted(!collapsed());
  const startAdding = () => {
    if (collapsed()) setCollapsedPersisted(false);
    setAdding(true);
  };
  const label = (): string =>
    props.entry.home
      ? m().nav.personal
      : (app.state.settings.docName ?? m().nav.untitledDoc);

  // Active (non-archived) lists in CRDT order. Archived lists render no
  // nav row; they stay reachable through search and restorable via the
  // header's list menu (Unarchive).
  type NavList = { id: string; name: string; icon?: string };
  const lists = createMemo((): NavList[] =>
    app.state.listsOrder
      .map((id) => app.state.listsById[id])
      .filter((l): l is ListView => l !== undefined && l.archivedAt == null)
      .map((l) => ({ id: l.id, name: l.name, icon: l.icon })),
  );
  const [dndLists, setDndLists] = createSignal<NavList[]>([]);
  createEffect(() => setDndLists(lists()));
  // Open-item count per list, read straight off the projection arrays.
  const openCount = (listId: string): number => app.state.listOpen[listId]?.length ?? 0;
  const countLabel = (listId: string): string => {
    const n = openCount(listId);
    return n > 0 ? String(n) : "-";
  };
  const isActive = (listId: string): boolean =>
    props.view.kind === "list" &&
    props.view.id === listId &&
    props.docs.viewDocId(props.view) === docId;
  const open = (listId: string) => props.setView(props.docs.listView(docId, listId));

  const submit = (e: Event) => {
    e.preventDefault();
    const t = name().trim();
    if (!t) return;
    const id = app.addList(t);
    setName("");
    setAdding(false);
    open(id);
  };
  // Archive is non-destructive (globally undoable, trivially reversible
  // via Unarchive), so there is no confirmation dialog. Archiving the
  // list currently on screen would leave a stale view; hop to this doc's
  // Inbox before applying the mutation.
  const archiveList = (id: string) => {
    if (isActive(id)) open("inbox");
    app.setListArchived(id, true);
  };

  const navSelection = new DndSelection();
  const onNavItemClick = (e: MouseEvent, id: string) => {
    const modKey = /Mac|iPhone|iPad|iPod/.test(navigator.platform)
      ? e.metaKey
      : e.ctrlKey;
    if (e.shiftKey || modKey) return;
    open(id);
  };
  // Enter on a keyboard-focused user list opens it. The Dnd listbox owns
  // arrow-key navigation and updates navSelection's top key as the user
  // moves; we just translate that into a setView. stopPropagation keeps the
  // document-level onOpenKey from also firing and opening whatever's
  // selected in the main list.
  const onNavKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Enter") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const target = e.target as Element | null;
    if (target?.closest('input, textarea, [contenteditable="true"]')) return;
    const top = navSelection.getSelectionTop();
    if (top === null) return;
    e.preventDefault();
    e.stopPropagation();
    open(String(top));
  };
  const selectedNavIds = (id: string): string[] =>
    navSelection.isSelected(id) ? navSelection.getSelectedKeys().map(String) : [id];
  const onReorder = (op: DndOp<NavList>) => {
    if (op.type !== "move") return;
    const ids = lists().map((l) => l.id);
    const movedIds = op.keys.map(String).filter((id) => ids.includes(id));
    if (movedIds.length === 0) return;
    const remaining = ids.filter((id) => !movedIds.includes(id));
    const insertAt =
      op.beforeKey === null
        ? remaining.length
        : (() => {
            const idx = remaining.indexOf(String(op.beforeKey));
            return idx >= 0 ? idx : remaining.length;
          })();
    const nextIds = [...remaining];
    nextIds.splice(insertAt, 0, ...movedIds);
    const currentIds = [...ids];
    for (const [index, id] of nextIds.entries()) {
      if (currentIds[index] !== id) {
        const currentIndex = currentIds.indexOf(id);
        if (currentIndex < 0) continue;
        app.moveList(id, index);
        currentIds.splice(currentIndex, 1);
        currentIds.splice(index, 0, id);
      }
    }
  };
  // Rename on a non-Home heading: the label is an EditableNavLabel driven
  // from the heading's context menu, as list rows are.
  let startRename: (() => void) | undefined;

  // Hidden file input the "..." menu's Import JSON item triggers via
  // .click(). Imports land in this doc. Resetting `value` between picks
  // is what lets the user choose the same file twice in a row — without
  // it the change event never fires the second time.
  let importFileInput: HTMLInputElement | undefined;
  const onImportFilePicked = async (
    e: Event & { currentTarget: HTMLInputElement },
  ): Promise<void> => {
    const file = e.currentTarget.files?.[0];
    e.currentTarget.value = "";
    if (!file) return;
    try {
      const text = await file.text();
      const summary = app.importJson(text);
      alert(m().nav.importSucceeded(summary.itemsAdded, summary.listsAdded));
    } catch (err) {
      console.error("import json failed:", err);
      alert(m().nav.importFailed);
    }
  };

  const heading = () => (
    <div
      class="nav-section nav-section-toggle"
      role="button"
      aria-expanded={!collapsed()}
      onClick={toggle}
    >
      <Show
        when={!props.entry.home}
        fallback={<span class="nav-section-label">{label()}</span>}
      >
        <EditableNavLabel
          class="nav-section-label"
          name={label()}
          onSave={(next) => app.setDocName(next)}
          registerStart={(fn) => (startRename = fn)}
        />
      </Show>
      {/* Workspace menu: the first of the three trailing glyphs, opening a
          dropdown with the heading's actions (Rename on non-Home docs,
          Export / Import JSON on every doc). Pointer-down and click both stop
          here so opening the menu neither toggles the group nor starts
          a heading drag; Kobalte opens the menu on pointerdown. */}
      <DropdownMenu placement="bottom-end" gutter={4}>
        <DropdownMenu.Trigger
          class="icon-button nav-section-menu"
          tabIndex={-1}
          aria-label={m().nav.docMenu}
          title={m().nav.docMenu}
          innerHTML={dotsHorizontalSvg}
          onPointerDown={(e: PointerEvent) => e.stopPropagation()}
          onClick={(e: MouseEvent) => e.stopPropagation()}
        />
        <DropdownMenu.Portal>
          <DropdownMenu.Content class="dropdown-menu-content">
            <Show when={!props.entry.home}>
              <DropdownMenu.Item
                class="dropdown-menu-item"
                onSelect={() => {
                  requestAnimationFrame(() => startRename?.());
                }}
              >
                {m().nav.renameDoc}
              </DropdownMenu.Item>
            </Show>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              onSelect={() => {
                if (!exportDocJson(app, exportSlug(label()))) alert(m().nav.exportFailed);
              }}
            >
              {m().nav.exportJson}
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              onSelect={() => {
                // Defer past the menu close + focus-restore so the
                // native file picker isn't fighting Kobalte for focus.
                requestAnimationFrame(() => importFileInput?.click());
              }}
            >
              {m().nav.importJson}
            </DropdownMenu.Item>
            <Show when={canDelete()}>
              <DropdownMenu.Separator class="dropdown-menu-separator" />
              <DropdownMenu.Item
                class="dropdown-menu-item"
                onSelect={() => setDeleteConfirmOpen(true)}
              >
                {m().nav.deleteDoc}
              </DropdownMenu.Item>
            </Show>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu>
      {/* New-list affordance sits inside the heading, between the menu
          and the caret, rather than as a trailing "+ Add list" row:
          hover-only, so the group reads as a plain label at rest. The
          heading is a div (not a button) so this real button can nest
          inside it; stopPropagation keeps the click from also toggling
          the group. Opening the form expands a collapsed group, since
          the input renders inside it. */}
      <button
        type="button"
        class="icon-button nav-section-add"
        tabIndex={-1}
        aria-label={m().nav.newList}
        title={m().nav.newList}
        innerHTML={plusSvg}
        onClick={(e) => {
          e.stopPropagation();
          startAdding();
        }}
      />
      {/* Caret sits last, at the trailing edge; it shares the icon-button
          box and hover state with the "..." and "+" buttons before it and
          has no handler of its own since the whole heading is the toggle.
          Unlike the other two it stays visible while the group is
          collapsed, so a folded workspace still reads as expandable. */}
      <span
        class="icon-button nav-section-caret"
        data-collapsed={collapsed() ? "" : undefined}
        innerHTML={caretDownSvg}
      />
    </div>
  );

  return (
    <div class="nav-group" onKeyDown={onNavKeyDown}>
      {/* Outside the heading so the synthetic click from `.click()`
          can't bubble into its collapse toggle. */}
      <input
        ref={importFileInput}
        type="file"
        accept="application/json,.json"
        style={{ display: "none" }}
        onChange={onImportFilePicked}
      />
      <Show when={canDelete()}>
        <ConfirmDialog
          open={deleteConfirmOpen()}
          onOpenChange={setDeleteConfirmOpen}
          title={m().nav.deleteDoc}
          message={m().nav.deleteDocConfirm(label())}
          confirmLabel={m().common.delete}
          onConfirm={deleteDoc}
        />
      </Show>
      <Show when={!props.entry.home} fallback={heading()}>
        <ContextMenu>
          <ContextMenu.Trigger as="div">{heading()}</ContextMenu.Trigger>
          <ContextMenu.Portal>
            <ContextMenu.Content class="context-menu-content">
              <ContextMenu.Item
                class="context-menu-item"
                onSelect={() => {
                  requestAnimationFrame(() => startRename?.());
                }}
              >
                {m().nav.renameDoc}
              </ContextMenu.Item>
              <ContextMenu.Item
                class="context-menu-item"
                onSelect={() => {
                  void navigator.clipboard.writeText(listUrl("inbox", docId));
                }}
              >
                {m().common.copyLink}
              </ContextMenu.Item>
            </ContextMenu.Content>
          </ContextMenu.Portal>
        </ContextMenu>
      </Show>
      <Show when={!collapsed()}>
      {/* Inbox is reserved: it has no `ListMeta` row and carries the
          localized built-in label, so it's a static entry with no rename
          affordance and no context menu. It sits at the head of the lists
          group — visually one of the lists, but outside the Dnd so it can
          never be reordered, multi-selected, or dragged. */}
      <button
        type="button"
        class="nav-item"
        data-active={isActive("inbox") ? "" : undefined}
        data-drop-list-id="inbox"
        data-drop-doc-id={docId}
        tabIndex={-1}
        onClick={() => open("inbox")}
      >
        <span class="nav-item-icon" innerHTML={archiveSvg} />
        {m().nav.inbox}
        <span class="nav-item-count">{countLabel("inbox")}</span>
      </button>
      <Show when={lists().length > 0}>
        <Dnd
          items={dndLists()}
          setItems={setDndLists}
          getKey={(l) => l.id}
          itemHeight={props.mobile ? 40 : 28}
          multi
          arrowNavigate={false}
          clearOnClickOutside
          selection={navSelection}
          onReorder={onReorder}
        >
          {(l) => {
            // Captured by the Rename ContextMenu.Item below. The
            // EditableNavLabel hands us its startEdit on mount so the
            // context menu can drive rename mode the same way a
            // double-click does.
            let startListRename: (() => void) | undefined;
            const selectedIds = (): string[] => selectedNavIds(l().id);
            const isMultiMenu = (): boolean => selectedIds().length > 1;
            return (
              <ContextMenu
                onOpenChange={(isOpen) => {
                  if (isOpen && !navSelection.isSelected(l().id)) {
                    navSelection.selectOnly(l().id);
                  }
                }}
              >
                <ContextMenu.Trigger
                  as="button"
                  type="button"
                  class="nav-item"
                  data-active={isActive(l().id) ? "" : undefined}
                  data-drop-list-id={l().id}
                  data-drop-doc-id={docId}
                  tabIndex={-1}
                  onClick={(e) => onNavItemClick(e, l().id)}
                >
                  <Show
                    when={l().icon}
                    fallback={<span class="nav-item-icon" innerHTML={fileSvg} />}
                  >
                    {(icon) => (
                      <span class="nav-item-icon nav-item-icon-emoji">{icon()}</span>
                    )}
                  </Show>
                  <EditableNavLabel
                    name={l().name}
                    onSave={(next) => app.renameList(l().id, next)}
                    registerStart={(fn) => (startListRename = fn)}
                  />
                  <Show when={props.showListCounts}>
                    <span class="nav-item-count">{countLabel(l().id)}</span>
                  </Show>
                </ContextMenu.Trigger>
                {/* Rename + Archive are single-list actions. Multi-select
                    previously only hosted the show/hide-counts toggle,
                    which now lives in Settings → General, so a
                    multi-selection has no per-list menu: gate the whole
                    Portal so right-clicking a multi-selection shows
                    nothing rather than an empty menu. */}
                <Show when={!isMultiMenu()}>
                  <ContextMenu.Portal>
                    <ContextMenu.Content class="context-menu-content">
                      <ContextMenu.Item
                        class="context-menu-item"
                        onSelect={() => {
                          // Defer past the menu's close + focus-restore
                          // (Kobalte returns focus to the trigger on
                          // dismiss); rAF ensures our caret-placement
                          // microtask wins the race.
                          requestAnimationFrame(() => startListRename?.());
                        }}
                      >
                        {m().nav.renameList}
                      </ContextMenu.Item>
                      <ContextMenu.Item
                        class="context-menu-item"
                        onSelect={() => {
                          void navigator.clipboard.writeText(listUrl(l().id));
                        }}
                      >
                        {m().common.copyLink}
                      </ContextMenu.Item>
                      <ContextMenu.Item
                        class="context-menu-item"
                        onSelect={() => archiveList(l().id)}
                      >
                        {m().nav.archiveList}
                      </ContextMenu.Item>
                    </ContextMenu.Content>
                  </ContextMenu.Portal>
                </Show>
              </ContextMenu>
            );
          }}
        </Dnd>
      </Show>
      <Show when={adding()}>
        <NewListForm
          name={name()}
          setName={setName}
          onSubmit={submit}
          onDismiss={() => setAdding(false)}
        />
      </Show>
      </Show>
    </div>
  );
}

/** Magnifying-glass button pinned to the right end of the sidebar
 *  footer; opens the Find palette (the same surface as the `/` and ⌘F
 *  shortcuts). Mouse discoverability for the palette — keyboard users
 *  never need it. */
export function NavFindButton(props: { onClick: () => void }) {
  const { m } = useAppI18n();
  return (
    <Tooltip openDelay={200} closeDelay={0} placement="top">
      <Tooltip.Trigger
        as="button"
        type="button"
        class="icon-button nav-find-button"
        tabIndex={-1}
        aria-label={m().shortcuts.find}
        onClick={() => props.onClick()}
        innerHTML={magnifyingGlassSvg}
      />
      <Tooltip.Portal>
        <Tooltip.Content class="tooltip-content">
          {m().shortcuts.find}
          <Tooltip.Arrow />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip>
  );
}

/** The app-menu dropdown (undo/redo, new workspace, settings, auth)
 *  plus the auth dialog the Sign in / Sign up items open. Import /
 *  export live on each workspace heading's "..." menu. Lives in the
 *  sidebar footer (see `Nav`'s `footer` prop). */
export function NavMenu(props: {
  /** Undo / redo go through the layer (the doc that acted last). */
  docs: DocsLayer;
  session: Session;
  logout: () => void;
  onOpenSettings: () => void;
  onOpenShortcuts: () => void;
  onSession: (s: Session) => void;
  /** Open the "New workspace" dialog, or null when this session can't
   *  create docs (anonymous) — the item is hidden then. */
  onNewDoc: (() => void) | null;
}) {
  const { m } = useAppI18n();
  // Auth dialog opened from the menu's Sign in / Sign up items
  // (anonymous sessions only). Separate from StatusSlot's own auto-open
  // dialog; `authMode` seeds the form's login/signup toggle.
  const [authOpen, setAuthOpen] = createSignal(false);
  const [authMode, setAuthMode] = createSignal<"login" | "signup">("login");
  const openAuth = (mode: "login" | "signup") => {
    setAuthMode(mode);
    setAuthOpen(true);
  };
  const handleAuthSession = (s: Session) => {
    setAuthOpen(false);
    props.onSession(s);
  };

  // Reactive read-throughs of the engine's undo state. `app.version`
  // bumps on every dispatched event (local mutation, undo/redo, remote
  // import), which is exactly when undo availability can change.
  const canUndo = (): boolean => {
    props.docs.version();
    return props.docs.canUndo();
  };
  const canRedo = (): boolean => {
    props.docs.version();
    return props.docs.canRedo();
  };

  return (
    <>
      <DropdownMenu placement="top-start" gutter={6}>
        <Tooltip openDelay={200} closeDelay={0} placement="top">
          <Tooltip.Trigger
            as={DropdownMenu.Trigger}
            class="icon-button"
            tabIndex={-1}
            aria-label={m().common.menu}
            innerHTML={dotsVerticalSvg}
          />
          <Tooltip.Portal>
            <Tooltip.Content class="tooltip-content">
              {m().common.menu}
              <Tooltip.Arrow />
            </Tooltip.Content>
          </Tooltip.Portal>
        </Tooltip>
        <DropdownMenu.Portal>
          <DropdownMenu.Content class="dropdown-menu-content">
            <DropdownMenu.Item
              class="dropdown-menu-item"
              disabled={!canUndo()}
              onSelect={() => {
                props.docs.undo();
              }}
            >
              <span>{m().nav.undo}</span>
              <kbd class="menu-shortcut">⌘Z</kbd>
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              disabled={!canRedo()}
              onSelect={() => {
                props.docs.redo();
              }}
            >
              <span>{m().nav.redo}</span>
              <kbd class="menu-shortcut">⌘⇧Z</kbd>
            </DropdownMenu.Item>
            <DropdownMenu.Separator class="dropdown-menu-separator" />
            <Show when={props.onNewDoc}>
              {(onNewDoc) => (
                <DropdownMenu.Item
                  class="dropdown-menu-item"
                  onSelect={() => onNewDoc()()}
                >
                  {m().nav.newDoc}
                </DropdownMenu.Item>
              )}
            </Show>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              onSelect={() => props.onOpenShortcuts()}
            >
              <span>{m().shortcuts.title}</span>
              <kbd class="menu-shortcut">?</kbd>
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              onSelect={() => props.onOpenSettings()}
            >
              {m().nav.settings}
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              as="a"
              href="https://monoplan.app/"
              target="_blank"
              rel="noopener noreferrer"
            >
              {m().nav.website}
              <span innerHTML={externalLinkSvg} />
            </DropdownMenu.Item>
            <Show when={props.session.anonymous}>
              <DropdownMenu.Separator class="dropdown-menu-separator" />
              <DropdownMenu.Item
                class="dropdown-menu-item"
                onSelect={() => openAuth("login")}
              >
                {m().auth.signIn}
              </DropdownMenu.Item>
              <DropdownMenu.Item
                class="dropdown-menu-item"
                onSelect={() => openAuth("signup")}
              >
                {m().auth.signUp}
              </DropdownMenu.Item>
            </Show>
            <Show when={!props.session.anonymous}>
              <DropdownMenu.Item
                class="dropdown-menu-item"
                onSelect={() => props.logout()}
              >
                {m().nav.logOut}
              </DropdownMenu.Item>
            </Show>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu>
      <Show when={props.session.anonymous}>
        <AuthDialog
          open={authOpen()}
          onOpenChange={setAuthOpen}
          onSession={handleAuthSession}
          initialMode={authMode()}
        />
      </Show>
    </>
  );
}

function NewListForm(props: {
  name: string;
  setName: (v: string) => void;
  onSubmit: (e: Event) => void;
  onDismiss: () => void;
}) {
  const { m } = useAppI18n();
  let inputRef!: HTMLInputElement;
  onMount(() => inputRef.focus());
  return (
    <form onSubmit={props.onSubmit}>
      <input
        ref={inputRef}
        class="nav-item nav-item-input"
        type="text"
        placeholder={m().nav.newList}
        value={props.name}
        onInput={(e) => props.setName(e.currentTarget.value)}
        onBlur={() => {
          if (!props.name.trim()) props.onDismiss();
        }}
        // Escape commits a typed name (same as Enter) and dismisses an
        // empty input. Native listener + stopPropagation so the
        // document-level Escape handlers (side panel close, dnd selection
        // clear) don't also fire on the same keystroke.
        on:keydown={(e) => {
          if (e.key !== "Escape") return;
          e.preventDefault();
          e.stopPropagation();
          if (props.name.trim()) props.onSubmit(e);
          else props.onDismiss();
        }}
      />
    </form>
  );
}

export function EditableNavLabel(props: {
  name: string;
  onSave: (name: string) => void;
  class?: string;
  /** Called once on mount with a function the parent can invoke to enter
   *  rename mode (e.g. from a context-menu item). */
  registerStart?: (fn: () => void) => void;
}) {
  let ref!: HTMLSpanElement;
  const [editing, setEditing] = createSignal(false);

  // While not editing, the span's text mirrors the model. While editing
  // we leave the DOM alone so the user's in-flight edits aren't clobbered
  // by reactive updates (including ones from peer renames).
  createEffect(() => {
    if (!editing()) ref.textContent = props.name;
  });

  // Focus + select-all whenever editing flips on, regardless of whether
  // the trigger was a double-click or an external caller (context menu).
  // Microtask defers until contentEditable=true has been applied.
  createEffect(() => {
    if (!editing()) return;
    queueMicrotask(() => {
      ref.focus();
      const sel = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(ref);
      sel?.removeAllRanges();
      sel?.addRange(range);
    });
  });

  const startEdit = (e?: Event) => {
    e?.preventDefault();
    e?.stopPropagation();
    if (editing()) return;
    setEditing(true);
  };

  onMount(() => {
    props.registerStart?.(() => startEdit());
  });

  const save = () => {
    if (!editing()) return;
    const next = (ref.textContent ?? "").trim();
    setEditing(false);
    if (next !== props.name) props.onSave(next);
    else ref.textContent = props.name;
  };

  const cancel = () => {
    if (!editing()) return;
    setEditing(false);
    ref.textContent = props.name;
  };

  return (
    <span
      ref={ref}
      class={props.class ?? "nav-item-label"}
      contentEditable={editing()}
      onDblClick={startEdit}
      on:keydown={(e) => {
        // Native (non-delegated) so the bubble order is span → nav dnd
        // listbox; Solid's delegated `onKeyDown` would fire at document
        // level *after* the listbox's bubble handler, so Cmd+A would hit
        // the dnd's select-all before we could stop it. Same pattern as
        // the row-text editor below.
        if (!editing()) return;
        if (e.key === "Enter") {
          e.preventDefault();
          e.stopPropagation();
          save();
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          cancel();
          return;
        }
        // Don't let the dnd intercept keys the contenteditable owns
        // (Cmd+A select-all, arrow caret movement, etc).
        e.stopPropagation();
      }}
      on:paste={(e) => {
        if (!editing()) return;
        // Strip formatting: paste plain text only, matching the row-text
        // editor — the label saves `textContent`, so pasted HTML would
        // just look styled until the rename commits.
        pasteAsPlainText(e);
      }}
      onBlur={save}
      onClick={(e) => {
        if (editing()) e.stopPropagation();
      }}
      onPointerDown={(e) => {
        if (editing()) e.stopPropagation();
      }}
    />
  );
}
