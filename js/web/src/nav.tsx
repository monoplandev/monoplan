import {
  createEffect,
  createSignal,
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
import { Dnd, DndSelection, type DndOp } from "./dnd/solid";
import archiveSvg from "./icons/archive.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import caretDownSvg from "./icons/caret-down.svg?raw";
import checkSvg from "./icons/check.svg?raw";
import cloudSvg from "./icons/cloud.svg?raw";
import cloudOffSvg from "./icons/cloud-off.svg?raw";
import crumpledPaperSvg from "./icons/crumpled-paper.svg?raw";
import dotsVerticalSvg from "./icons/dots-vertical.svg?raw";
import drawingPinSvg from "./icons/drawing-pin.svg?raw";
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
import type { DocApp } from "./sync/store.ts";

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

// Whether the "Personal" workspace group (Inbox + lists) is collapsed in
// the nav. Groundwork for shared workspaces: each workspace will get its
// own collapsible heading, and the personal one is the first. Purely
// local UI state, same localStorage rationale as the flag above.
const PERSONAL_COLLAPSED_KEY = "monoplan:nav-personal-collapsed";
function loadPersonalCollapsed(): boolean {
  try {
    return localStorage.getItem(PERSONAL_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}
function savePersonalCollapsed(collapsed: boolean): void {
  try {
    if (collapsed) localStorage.setItem(PERSONAL_COLLAPSED_KEY, "1");
    else localStorage.removeItem(PERSONAL_COLLAPSED_KEY);
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
export function StatusSlot(props: {
  /** Extra class on the indicator button (mobile's glass chrome). */
  class?: string;
  app: DocApp;
  online: boolean;
  lastSyncAt: number | null;
  session: Session;
  onSession: (s: Session) => void;
}) {
  const { m } = useAppI18n();
  // Auto-prompt anonymous users on first mount unless they've dismissed the
  // prompt before (persisted in localStorage). Closing sets that flag so
  // reloads don't re-nag. The whole workspace remounts on session swap
  // (App's keyed <Show>); logout clears the flag so it re-prompts.
  const [authOpen, setAuthOpen] = createSignal(
    props.session.anonymous && !loadAuthPromptDismissed(),
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
  app: DocApp;
  /** Active (non-archived) lists — the draggable main section. Archived
   *  lists render no nav section for now; they stay reachable through
   *  search and restorable via the header's list menu (Unarchive). */
  lists: { id: string; name: string; icon?: string }[];
  binCount: number;
  /** Number of visible Focus refs — the Focus nav entry's count badge, and
   *  what drives the soft "getting big" signal past the threshold. */
  focusCount: number;
  /** Open-item count (Backlog + Live) per list id. Inbox's row always
   *  renders a badge (showing "-" when zero); non-Inbox rows render theirs
   *  only when `showListCounts` is true, again with "-" for zero. */
  openCountsByList: Record<string, number>;
  /** Doc-level settings flag; when true, render the live-item count
   *  badge beside each non-Inbox list in the nav (showing "-" when the
   *  list is empty). Inbox's badge is always shown regardless. */
  showListCounts: boolean;
  view: ViewKey;
  setView: (v: ViewKey) => void;
  /** Chrome pinned to the bottom of the sidebar (app menu + account/sync
   *  widget). Rendered inside the nav so it hides and floats with it —
   *  see `.nav-footer` in styles.css. */
  footer?: JSX.Element;
}) {
  const { m } = useAppI18n();
  const [adding, setAdding] = createSignal(false);
  const [emptyBinConfirmOpen, setEmptyBinConfirmOpen] = createSignal(false);
  // Archive is non-destructive (globally undoable, trivially reversible
  // via Unarchive), so there is no confirmation dialog. Archiving the
  // list currently on screen would leave a stale view; hop to Inbox
  // before applying the mutation.
  const archiveList = (id: string) => {
    if (props.view.kind === "list" && props.view.id === id) {
      props.setView({ kind: "list", id: "inbox" });
    }
    props.app.setListArchived(id, true);
  };
  const [name, setName] = createSignal("");
  const submit = (e: Event) => {
    e.preventDefault();
    const t = name().trim();
    if (!t) return;
    const id = props.app.addList(t);
    setName("");
    setAdding(false);
    props.setView({ kind: "list", id });
  };
  // `main` is a reserved id with no MovableList entry — clients
  // render it as a static nav button, so the dnd source is just
  // `props.lists` directly.
  type NavList = { id: string; name: string; icon?: string };
  const [dndLists, setDndLists] = createSignal<NavList[]>([]);
  createEffect(() => setDndLists(props.lists));

  // Match the items list's mobile bump so the drawer's tap targets feel
  // consistent with the main view.
  const navMobileMq = window.matchMedia(
    "(max-width: 768px) and (pointer: coarse)",
  );
  const [navIsMobile, setNavIsMobile] = createSignal(navMobileMq.matches);
  const onNavMqChange = (e: MediaQueryListEvent) => setNavIsMobile(e.matches);
  navMobileMq.addEventListener("change", onNavMqChange);
  onCleanup(() => navMobileMq.removeEventListener("change", onNavMqChange));

  const navSelection = new DndSelection();
  const onNavItemClick = (e: MouseEvent, id: string) => {
    const modKey = /Mac|iPhone|iPad|iPod/.test(navigator.platform)
      ? e.metaKey
      : e.ctrlKey;
    if (e.shiftKey || modKey) return;
    props.setView({ kind: "list", id });
  };
  // Enter on a keyboard-focused user list opens it. The Dnd listbox owns
  // arrow-key navigation and updates navSelection's top key as the user
  // moves; we just translate that into a setView. stopPropagation keeps the
  // document-level onOpenKey from also firing and opening whatever's
  // selected in the main list.
  const [personalCollapsed, setPersonalCollapsed] = createSignal(loadPersonalCollapsed());
  const togglePersonal = () => {
    const next = !personalCollapsed();
    setPersonalCollapsed(next);
    savePersonalCollapsed(next);
  };
  const startAdding = () => {
    if (personalCollapsed()) {
      setPersonalCollapsed(false);
      savePersonalCollapsed(false);
    }
    setAdding(true);
  };
  const onNavKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Enter") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const target = e.target as Element | null;
    if (target?.closest('input, textarea, [contenteditable="true"]')) return;
    const top = navSelection.getSelectionTop();
    if (top === null) return;
    e.preventDefault();
    e.stopPropagation();
    props.setView({ kind: "list", id: String(top) });
  };
  const selectedNavIds = (id: string): string[] =>
    navSelection.isSelected(id) ? navSelection.getSelectedKeys().map(String) : [id];

  // Keep the active view's nav row in view. The view can change from
  // anywhere (find palette, URL, keyboard, list creation, a click), so
  // react to the view itself rather than to any one entry point; a
  // clicked row is already visible and `block: "nearest"` makes that a
  // no-op. Every entry (Focus, Upcoming, lists, Done, Bin) marks itself
  // with `data-active`, so that's the lookup rather than a per-kind id.
  // The Dnd here isn't a scroller (no fillHeight), so every list row is
  // mounted and `.nav-scroll` is the only scroll container. Deferred a
  // frame (with a couple of retries) so a list created and opened in the
  // same tick has its row in the DOM before we look for it.
  let navScrollEl!: HTMLDivElement;
  const revealActiveNav = () => {
    // Read here so the effect below tracks the view.
    void props.view.kind;
    if (props.view.kind === "list") void props.view.id;
    let attempts = 0;
    const tryScroll = () => {
      // Collapsed sidebar (`.app.nav-hidden`) has no layout to scroll;
      // the ResizeObserver below re-runs this when it comes back.
      if (!navScrollEl || navScrollEl.clientHeight === 0) return;
      const el = navScrollEl.querySelector<HTMLElement>(".nav-item[data-active]");
      if (el) el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      else if (++attempts < 3) requestAnimationFrame(tryScroll);
    };
    requestAnimationFrame(tryScroll);
  };
  createEffect(() => {
    // The list rows only exist while the group is expanded, so expanding
    // it should also bring the active row into view.
    personalCollapsed();
    revealActiveNav();
  });
  onMount(() => {
    let wasHidden = navScrollEl.clientHeight === 0;
    const ro = new ResizeObserver(() => {
      const hidden = navScrollEl.clientHeight === 0;
      if (wasHidden && !hidden) revealActiveNav();
      wasHidden = hidden;
    });
    ro.observe(navScrollEl);
    onCleanup(() => ro.disconnect());
  });

  const onReorder = (op: DndOp<NavList>) => {
    if (op.type !== "move") return;
    const ids = props.lists.map((l) => l.id);
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
        props.app.moveList(id, index);
        currentIds.splice(currentIndex, 1);
        currentIds.splice(index, 0, id);
      }
    }
  };
  return (
    <nav class="nav" onKeyDown={onNavKeyDown}>
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
          <span class="nav-item-icon" innerHTML={drawingPinSvg} />
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
      <div class="nav-group">
        {/* Workspace heading. Today there is exactly one workspace
            ("Personal"); shared workspaces will add sibling groups, each
            with its own heading. Click collapses the group; the state is
            local to this browser. */}
        <div
          class="nav-section nav-section-toggle"
          role="button"
          aria-expanded={!personalCollapsed()}
          onClick={togglePersonal}
        >
          <span class="nav-section-label">{m().nav.personal}</span>
          <span
            class="nav-section-caret"
            data-collapsed={personalCollapsed() ? "" : undefined}
            innerHTML={caretDownSvg}
          />
          {/* New-list affordance sits inside the heading, after the caret,
              rather than as a trailing "+ Add list" row: hover-only like
              the caret, so the group reads as a plain label at rest. The
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
        </div>
        <Show when={!personalCollapsed()}>
        {/* Inbox is reserved: it has no `ListMeta` row and carries the
            localized built-in label, so it's a static entry with no rename
            affordance and no context menu. It sits at the head of the lists
            group — visually one of the lists, but outside the Dnd so it can
            never be reordered, multi-selected, or dragged. */}
        <button
          type="button"
          class="nav-item"
          data-active={
            props.view.kind === "list" && props.view.id === "inbox"
              ? ""
              : undefined
          }
          data-drop-list-id="inbox"
          tabIndex={-1}
          onClick={() => props.setView({ kind: "list", id: "inbox" })}
        >
          <span class="nav-item-icon" innerHTML={archiveSvg} />
          {m().nav.inbox}
          <span class="nav-item-count">
            {(props.openCountsByList["inbox"] ?? 0) > 0
              ? props.openCountsByList["inbox"]
              : "-"}
          </span>
        </button>
        <Show when={props.lists.length > 0}>
          <Dnd
            items={dndLists()}
            setItems={setDndLists}
            getKey={(l) => l.id}
            itemHeight={navIsMobile() ? 40 : 28}
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
              let startRename: (() => void) | undefined;
              const selectedIds = (): string[] => selectedNavIds(l().id);
              const isMultiMenu = (): boolean => selectedIds().length > 1;
              return (
                <ContextMenu
                  onOpenChange={(open) => {
                    if (open && !navSelection.isSelected(l().id)) {
                      navSelection.selectOnly(l().id);
                    }
                  }}
                >
                  <ContextMenu.Trigger
                    as="button"
                    type="button"
                    class="nav-item"
                    data-active={
                      props.view.kind === "list" && props.view.id === l().id
                        ? ""
                        : undefined
                    }
                    data-drop-list-id={l().id}
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
                      onSave={(name) => props.app.renameList(l().id, name)}
                      registerStart={(fn) => (startRename = fn)}
                    />
                    <Show when={props.showListCounts}>
                      <span class="nav-item-count">
                        {(props.openCountsByList[l().id] ?? 0) > 0
                          ? props.openCountsByList[l().id]
                          : "-"}
                      </span>
                    </Show>
                  </ContextMenu.Trigger>
                  {/* Rename + Delete are single-list actions. Multi-select
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
                            requestAnimationFrame(() => startRename?.());
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
      </div>
      <div class="nav-footer">{props.footer}</div>
      <ConfirmDialog
        open={emptyBinConfirmOpen()}
        onOpenChange={setEmptyBinConfirmOpen}
        message={m().workspace.emptyBinConfirm}
        confirmLabel={m().workspace.emptyBin}
        onConfirm={() => props.app.emptyBin()}
      />
    </nav>
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

/** The app-menu dropdown (undo/redo, view toggles, import/export,
 *  settings, auth) plus its auxiliary chrome — the hidden import file
 *  input and the auth dialog the Sign in / Sign up items open. Lives in
 *  the sidebar footer (see `Nav`'s `footer` prop); when the sidebar is
 *  hidden that footer floats bottom-left so the menu stays reachable. */
export function NavMenu(props: {
  app: DocApp;
  session: Session;
  logout: () => void;
  onOpenSettings: () => void;
  onOpenShortcuts: () => void;
  onSession: (s: Session) => void;
  /** Desktop sidebar collapse state + toggle (the menu's Show / Hide
   *  sidebar item). */
  navHidden: boolean;
  onToggleNav: () => void;
  /** Desktop side panel state + toggle (Show / Hide side panel item).
   *  The panel hosts the open task surface, else the deadline list. */
  sidePanelOpen: boolean;
  /** False when the viewport is too narrow to fit the panel: the item is
   *  disabled (the preference is preserved, not cleared). */
  sidePanelAvailable: boolean;
  onToggleSidePanel: () => void;
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
    props.app.version();
    return props.app.canUndo();
  };
  const canRedo = (): boolean => {
    props.app.version();
    return props.app.canRedo();
  };

  // Trigger a browser download for an in-memory blob. Anchor + revoke
  // is the only cross-browser path; FileSystem Access API isn't on
  // Safari yet.
  const triggerDownload = (blob: Blob, filename: string): void => {
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  };

  // "Export JSON": pretty-printed semantic dump (lists + items).
  // Readable in any editor and round-trips through Import JSON, but
  // lossy: CRDT history, ordering metadata, and undo-stack info aren't
  // here. (A lossless plaintext-snapshot export exists in the core —
  // `exportSnapshot` — but stays unexposed until a matching import lands.)
  const downloadJson = (): void => {
    try {
      const json = props.app.engine.exportJson();
      const blob = new Blob([json], { type: "application/json" });
      const stamp = new Date().toISOString().slice(0, 10);
      triggerDownload(blob, `monoplan-${stamp}.json`);
    } catch (err) {
      console.error("export json failed:", err);
      alert(m().nav.exportFailed);
    }
  };

  // Hidden file input the "Import JSON" menu item triggers via .click().
  // Resetting `value` between picks is what lets the user choose the same
  // file twice in a row — without it the change event never fires the
  // second time.
  let importFileInput: HTMLInputElement | undefined;
  const onImportFilePicked = async (
    e: Event & { currentTarget: HTMLInputElement },
  ): Promise<void> => {
    const file = e.currentTarget.files?.[0];
    e.currentTarget.value = "";
    if (!file) return;
    try {
      const text = await file.text();
      const summary = props.app.importJson(text);
      alert(m().nav.importSucceeded(summary.itemsAdded, summary.listsAdded));
    } catch (err) {
      console.error("import json failed:", err);
      alert(m().nav.importFailed);
    }
  };

  return (
    <>
      <input
        ref={importFileInput}
        type="file"
        accept="application/json,.json"
        style={{ display: "none" }}
        onChange={onImportFilePicked}
      />
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
                props.app.undo();
              }}
            >
              <span>{m().nav.undo}</span>
              <kbd class="menu-shortcut">⌘Z</kbd>
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              disabled={!canRedo()}
              onSelect={() => {
                props.app.redo();
              }}
            >
              <span>{m().nav.redo}</span>
              <kbd class="menu-shortcut">⌘⇧Z</kbd>
            </DropdownMenu.Item>
            <DropdownMenu.Separator class="dropdown-menu-separator" />
            <DropdownMenu.Item
              class="dropdown-menu-item"
              disabled={!props.sidePanelAvailable}
              onSelect={() => props.onToggleSidePanel()}
            >
              {props.sidePanelOpen ? m().sidePanel.hide : m().sidePanel.show}
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class="dropdown-menu-item"
              onSelect={() => props.onToggleNav()}
            >
              {props.navHidden ? m().nav.showSidebar : m().nav.hideSidebar}
            </DropdownMenu.Item>
            <DropdownMenu.Separator class="dropdown-menu-separator" />
            <DropdownMenu.Item
              class="dropdown-menu-item"
              onSelect={() => downloadJson()}
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
