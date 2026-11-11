import {
  batch,
  createComputed,
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  Show,
  untrack,
  type JSX,
} from "solid-js";
import {
  Dnd,
  DndSelection,
  type DndImperative,
  type DndOp,
} from "./dnd/solid";
import type { DndDragEventDetail } from "./dnd";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { Popover } from "@kobalte/core/popover";
import { SegmentedControl } from "@kobalte/core/segmented-control";
import { Switch } from "@kobalte/core/switch";
import { Tooltip } from "@kobalte/core/tooltip";
import archiveSvg from "./icons/archive.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import cardStackSvg from "./icons/card-stack.svg?raw";
import caretRightSvg from "./icons/caret-right.svg?raw";
import checkSvg from "./icons/check.svg?raw";
import crumpledPaperSvg from "./icons/trash.svg?raw";
import dotsHorizontalSvg from "./icons/dots-horizontal.svg?raw";
import starSvg from "./icons/star.svg?raw";
import eyeNoneSvg from "./icons/eye-none.svg?raw";
import eyeOpenSvg from "./icons/eye-open.svg?raw";
import listBulletSvg from "./icons/list-bullet.svg?raw";
import mixerHzSvg from "./icons/mixer-hz.svg?raw";
import plusSvg from "./icons/plus.svg?raw";
import trashSvg from "./icons/trash.svg?raw";
import { Board, type BoardImperative } from "./Board.tsx";
import { ConfirmDialog } from "./ConfirmDialog.tsx";
import { createArchiveListFlow } from "./ArchiveListDialog.tsx";
import { NewDocDialog } from "./NewDocDialog.tsx";
import { captureInviteFromLocation } from "./invite.ts";
import { InviteDialog } from "./InviteDialog.tsx";
import type { Dek } from "@monoplan/core/wasm";
import { FindPalette } from "./FindPalette.tsx";
import { FindSheet } from "./FindSheet.tsx";
import type { FindResult } from "./findResults.tsx";
import { laneLabel, useAppI18n } from "./i18n.tsx";
import { readClip, toClipItem, writeClip, type ClipItem } from "./itemClip.ts";
import { ListIconPicker } from "./ListIconPicker.tsx";
import { attentionBadge, groupByDay, type DayGroup } from "./dayGroups.ts";
import { nowMs, todayStamp } from "./format.tsx";
import { restoreCapturedPositions } from "./linger.ts";
import { createPopoverTooltipGuard } from "./popoverTooltip.ts";
import { DoneFilterMenu } from "./DoneFilterMenu.tsx";
import {
  doneFilterAdmits,
  doneListKey,
  effectiveDoneFilter,
  EMPTY_DONE_FILTER,
  loadDoneFilter,
  offeredDoneLists,
  saveDoneFilter,
  toggled,
  type DoneFilter,
  type DoneListOption,
} from "./doneFilter.ts";
import type { ListOption } from "./ListPicker.tsx";
import { MovePalette } from "./MovePalette.tsx";
import { EditableNavLabel, Nav, NavFindButton, NavMenu, FirstRunAuthPrompt, StatusSlot } from "./nav.tsx";
import { MobileBars } from "./MobileShell.tsx";
import {
  isMacShell,
  onTitlebarMouseDown,
  postUndoState,
  setFindHandler,
  setUndoRedoHandler,
} from "./sync/mirrorHost.ts";
import { installConsoleApi } from "./consoleApi.ts";
import { digitNavTarget } from "./navShortcuts.ts";
import {
  focusItems,
  isOverlayOpen,
  onGlobalKey,
  registerFocusHome,
} from "./overlay.ts";
import type { ViewKey } from "./prefs.ts";
import { Row, DRAFT_ID_PREFIX } from "./Row.tsx";
import { planReorderMoves } from "./reorder.ts";
import { SelectionPanel, type SelectionAction } from "./SelectionPanel.tsx";
import { Settings } from "./Settings.tsx";
import { ShortcutsDialog } from "./ShortcutsDialog.tsx";
import {
  TaskDialog,
  type CaptureHandle,
  type NewItemTarget,
} from "./TaskDialog.tsx";
import { Upcoming, type UpcomingImperative } from "./Upcoming.tsx";
import { useSession } from "./SessionContext.tsx";
import { createMemberDirectory } from "./sync/members.ts";
import type { PresentAccount } from "./sync/runtime.ts";
import type { DocsLayer } from "./sync/docs.ts";
import {
  isBinned,
  isCancelled,
  isClosed,
  isClosedState,
  isEvent,
  isOpen,
  LIST_EVENTS,
  OPEN_STATES,
  type DocApp,
  type ItemView,
  type Place,
  type UndoOutcome,
  type ListView,
  type WorkflowState,
} from "./sync/store.ts";
import { createTheme, type ThemePreference } from "./theme.ts";
import {
  NAV_WIDTH_DEFAULT,
  NAV_WIDTH_MAX,
  NAV_WIDTH_MIN,
  clampNavWidth,
  loadNavWidth,
  saveNavWidth,
} from "./navWidth.ts";
import { namesItem, parseHash, stateHash, type Route } from "./url.ts";
import {
  encodeView,
  LIST_VIEW,
  parseView,
  viewsEqual,
  withLane,
  type ViewSpec,
} from "./view.ts";

// Done items linger in their live list this long after being marked
// done, so the user sees the strike-through before the row drops out.
// The state change is instant — this is purely a render-time tail
// derived from doneAt, not a separate "pending" set.
const DONE_LINGER_MS = 3_000;

/** Fixed list-row height for the virtualizer. Must agree with the
 *  `.row` padding in styles.css: 8px + 1.3em line + 8px. Touch
 *  viewports add 12px for a comfortable thumb target. */
function rowHeight(mobile: boolean): number {
  return mobile ? 44 : 32;
}

// Module-level so the OS-preference listener is registered exactly
// once for the lifetime of the page.
const theme = createTheme();

// Per-list *local* view override: list ⇄ board plus the board's Done
// lane, for the lists where this browser wants something other than the
// list's saved default (the same account may want a board on desktop and
// a flat list on a phone — see spec/board.md "Client (web) contract").
// Absent ≡ follow the synced default; only genuinely divergent lists get
// a key, so a client that never overrides anything tracks the account.
// Values are encoded `ViewSpec` strings — the same grammar the doc
// stores, so comparing local against default is a string compare.
const VIEW_PREF_KEY = "monoplan:list-view";
function loadViewPrefs(): Record<string, string> {
  try {
    const raw = localStorage.getItem(VIEW_PREF_KEY);
    return raw ? (JSON.parse(raw) as Record<string, string>) : {};
  } catch {
    return {};
  }
}

/** One lane in the view-mode popover: name, item count, and an eye
 *  button toggling client-local visibility (spec/board.md "Lane
 *  visibility"). Hidden lanes dim so the icon state needn't be read
 *  closely. Only the eye is interactive for now. */
function LaneRow(props: {
  name: string;
  count: number;
  visible: boolean;
  onToggle: (show: boolean) => void;
}) {
  const { m } = useAppI18n();
  return (
    <div class="lane-row" data-hidden={props.visible ? undefined : ""}>
      <span class="lane-row-name">{props.name}</span>
      <span class="lane-row-count">{props.count}</span>
      <button
        type="button"
        class="lane-row-eye"
        aria-pressed={props.visible}
        aria-label={
          props.visible
            ? m().board.hideLane(props.name)
            : m().board.showLane(props.name)
        }
        onClick={() => props.onToggle(!props.visible)}
        innerHTML={props.visible ? eyeOpenSvg : eyeNoneSvg}
      />
    </div>
  );
}

// Per-list *local* "badge each row with its lifecycle state" flag for the
// flat list view. Display-only and client-local like the lane prefs: the
// board makes state visible as lanes, so the list lens is the only place
// state is otherwise invisible, and whether that matters is a per-list
// call (a capture list is all Backlog; a project list is not). Off by
// default — only the lists with it on get a key.
const STATE_PREF_KEY = "monoplan:list-show-state";
function loadShowStatePrefs(): Record<string, true> {
  try {
    const raw = localStorage.getItem(STATE_PREF_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    if (!Array.isArray(parsed)) return {};
    const out: Record<string, true> = {};
    for (const id of parsed) if (typeof id === "string") out[id] = true;
    return out;
  } catch {
    return {};
  }
}

/** The header's "Display options" popover: tooltip on the trigger, popover
    body from children. The guard keeps the tooltip from showing while the
    popover is open (or when focus returns to the button on close). */
function DisplayOptionsPopover(props: { children: JSX.Element }) {
  const { m } = useAppI18n();
  const guard = createPopoverTooltipGuard();
  return (
    <Popover {...guard.popover} placement="bottom-end" gutter={6}>
      <Tooltip {...guard.tooltip} openDelay={200} closeDelay={0} placement="bottom">
        <Tooltip.Trigger
          as={Popover.Trigger}
          class="icon-button view-mode-trigger"
          tabIndex={-1}
          aria-label={m().workspace.displayOptions}
          innerHTML={mixerHzSvg}
        />
        <Tooltip.Portal>
          <Tooltip.Content class="tooltip-content">
            {m().workspace.displayOptions}
            <Tooltip.Arrow />
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip>
      <Popover.Portal>
        <Popover.Content {...guard.content} class="view-mode-popover">
          {props.children}
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
}

export function Workspace(props: {
  /** The personal layer over every open doc (`sync/docs.ts`): the doc
   *  map, routed mutations and the merged lenses. A list view renders
   *  from its own doc's `DocApp` (`viewApp` below); the cross-doc views
   *  (Focus, calendar, Done, Bin) render from the layer's merged reads
   *  and route each row's mutations by `ItemView.docId`. */
  docs: DocsLayer;
  // View lives in `MainApp` so device writes can persist it alongside
  // the sync frontier in one debounced put. See `currentView` on
  // `DeviceConfig`.
  view: () => ViewKey;
  setView: (v: ViewKey) => void;
  /** Who has a doc open on the server right now (`runtime.presence`):
   *  drives the avatars by each workspace name in the nav. */
  presence: (docId: string) => PresentAccount[];
  /** Create a doc the account owns and open it (`runtime.createDoc`). */
  createDoc: (name: string) => Promise<string>;
  /** Open a doc the account just joined through a sharing link
   *  (`runtime.joinDoc`); the DEK came out of the link. */
  joinDoc: (docId: string, dek: Dek) => Promise<string>;
  /** Delete a non-Home doc the account owns and forget it here
   *  (`runtime.deleteDoc`); rejects on Home or anonymous sessions. The
   *  shown name rides along for the app's native confirm. */
  deleteDoc: (docId: string, name: string) => Promise<void>;
  /** Leave a shared doc the account is a member of but does not own,
   *  and forget it here (`runtime.leaveDoc`). */
  leaveDoc: (docId: string) => Promise<void>;
}) {
  const { m, locale } = useAppI18n();
  const session = useSession();
  const docs = props.docs;
  const home = docs.home;
  const routed = docs.routed;
  const view = props.view;
  const setView = props.setView;
  // The doc the current view is scoped to: a list view's own doc, Home
  // for the lenses. Everything list-shaped (the board, the inline draft,
  // list rename / icon / archive, the display options) acts on it.
  const viewDocId = createMemo((): string => docs.viewDocId(view()));
  const viewApp = createMemo((): DocApp => docs.appForView(view()));
  // Header-menu archive shares the nav's flow (dialog when the list has
  // open items). The view stays put: the header then shows the
  // Archived badge and the menu flips to Unarchive.
  const archive = createArchiveListFlow();
  // Display label for a doc: Home is "Personal"; others carry their
  // name, or the untitled fallback.
  const docLabel = (docId: string): string =>
    docId === docs.homeDocId
      ? m().nav.personal
      : (docs.appOf(docId)?.state.settings.docName ?? m().nav.untitledDoc);

  // The Done view's filter (`doneFilter.ts`): checked workspaces and
  // lists, device-local. The stored choice is kept verbatim; what the
  // view and its menu read is `doneFilterNow`, narrowed to the docs that
  // are open and the lists on offer. Archived lists are offered too:
  // their closed items are still in the Done view. Declared ahead of
  // the `items` memo, which reads it on its first run.
  const [doneFilter, setDoneFilter] = createSignal<DoneFilter>(loadDoneFilter());
  const doneAllLists = createMemo((): DoneListOption[] =>
    docs.docs().flatMap((d) => {
      const docId = d.docId;
      const workspace = docId === docs.homeDocId ? undefined : docLabel(docId);
      const option = (listId: string, name: string): DoneListOption => ({
        key: doneListKey(docId, listId),
        docId,
        listId,
        name,
        workspace,
      });
      return [
        option("inbox", m().nav.inbox),
        option(LIST_EVENTS, m().nav.events),
        ...d.app.state.listsOrder.flatMap((id) => {
          const l = d.app.state.listsById[id];
          return l ? [option(l.id, l.name)] : [];
        }),
      ];
    }),
  );
  const openDocIds = (): string[] => docs.docs().map((d) => d.docId);
  const doneFilterNow = createMemo(
    (): DoneFilter => effectiveDoneFilter(doneFilter(), openDocIds(), doneAllLists()),
  );
  const doneOfferedLists = createMemo((): DoneListOption[] =>
    offeredDoneLists(doneFilterNow().docs, doneAllLists()),
  );
  // Edits start from the effective filter, so a toggle also drops the
  // stale ids (a closed doc, a deleted list) it was carrying.
  const updateDoneFilter = (edit: (f: DoneFilter) => DoneFilter): void => {
    const next = edit(doneFilterNow());
    setDoneFilter(next);
    saveDoneFilter(next);
  };
  // Open-item count for the list view's header, read off the projection
  // array like the nav's per-list count (not `items()`, whose linger
  // overlay briefly keeps a just-completed row). Zero off a list.
  const headerOpenCount = createMemo((): number => {
    const v = view();
    if (v.kind !== "list") return 0;
    return viewApp().state.listOpen[v.id]?.length ?? 0;
  });
  const [dndItems, setDndItems] = createSignal<ItemView[]>([]);
  const [themePref, setThemePref] = createSignal<ThemePreference>(theme.get());
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  // The sync facts are a console call, `monoplan.sync()` (consoleApi.ts).
  installConsoleApi(docs, session.online);
  const [logoutConfirmOpen, setLogoutConfirmOpen] = createSignal(false);
  // The "New workspace" dialog; opened from the nav footer's app menu.
  const [newDocOpen, setNewDocOpen] = createSignal(false);
  const [emptyBinConfirmOpen, setEmptyBinConfirmOpen] = createSignal(false);
  const [findOpen, setFindOpen] = createSignal(false);
  const [shortcutsOpen, setShortcutsOpen] = createSignal(false);
  // The item the user has entered (Enter on a row, a row's open control,
  // a Find pick, a link, a click into the side pane), or null. This is
  // what the address bar mirrors and what the modal / mobile page show.
  // The side pane shows `shownItemId` (below, with the selection wiring):
  // this when set, else the row the selection is on. `equals: false` so
  // re-entering the item the pane already shows (Enter on the list after
  // the pane handed focus back) still notifies the surface, which
  // re-lands the caret in the title.
  const [openItemId, setOpenItemIdRaw] = createSignal<string | null>(null, {
    equals: false,
  });
  // An `#item_` link whose id isn't in the store yet (not synced, or
  // gone). Retried on every store change until it resolves or the user
  // navigates elsewhere (`spec/urls.md`). Declared here so opens can drop
  // it.
  // An item link whose item no open doc has yet (`spec/urls.md`), kept
  // with its context so the agenda form still lands on the agenda.
  const [pendingItem, setPendingItem] = createSignal<{ id: string; on?: "upcoming" } | null>(null);
  // Identity for a view, for equality checks (`ViewKey` objects are
  // recreated freely).
  const viewKey = (v: ViewKey) =>
    v.kind === "list" ? `list:${docs.viewDocId(v)}:${v.id}` : v.kind;
  // The view the entered item was entered under. Navigating to another
  // view closes the item (the view-change effect below); a route that
  // reveals + enters in one batch has already switched the view by the
  // time it enters, so the record matches and the item survives.
  let openedViewKey: string | null = null;
  const setOpenItemId = (id: string | null) =>
    batch(() => {
      setPendingItem(null);
      openedViewKey = id === null ? null : viewKey(untrack(view));
      setOpenItemIdRaw(id);
    });
  // Whether the task surface's planned-date / deadline popovers are open.
  // Owned here rather than by the surface so a row context menu's "Set
  // date…" / "Set deadline…" can enter the item with the popover already
  // showing. The surface clears them as it closes.
  const [itemWhenOpen, setItemWhenOpen] = createSignal(false);
  const [itemDeadlineOpen, setItemDeadlineOpen] = createSignal(false);
  const openItemWhen = (id: string) =>
    batch(() => {
      setOpenItemId(id);
      setItemWhenOpen(true);
    });
  const openItemDeadline = (id: string) =>
    batch(() => {
      setOpenItemId(id);
      setItemDeadlineOpen(true);
    });
  // Rows the move palette will re-file (visible order), or null when the
  // palette is closed. Captured at open (from the `m` shortcut's selection
  // or a row context menu's target set) so the pick acts on what the user
  // saw, not on whatever the selection is by commit time.
  const [moveIds, setMoveIds] = createSignal<string[] | null>(null);
  // New-item capture target for the task surface (board "+" buttons, the
  // calendar's Add, the Done view's Log, and the list view's draft mirror
  // — see `startDraft`), or null when not capturing. Mutually exclusive
  // with `openItemId`. `docId` is the doc the target list lives in.
  const [newItemTarget, setNewItemTarget] = createSignal<
    (NewItemTarget & { docId: string }) | null
  >(null);
  // Ids to select + scroll into view in the board once they land in their
  // column (a "+" capture, a duplicated block, or a find-palette pick that
  // lands on a board list). A list so a multi-item duplicate selects the
  // whole block, not just one. Handed to the Board, which clears it once it
  // lands the selection.
  const [boardRevealIds, setBoardRevealIds] = createSignal<string[] | null>(
    null,
  );
  // The board's active column selection (or null), published up by Board so
  // the global item shortcuts act on it in board view — the workspace-level
  // `selection` below only ever holds the flat list view's selection.
  const [boardSelection, setBoardSelection] = createSignal<DndSelection | null>(
    null,
  );
  // UI-only mirror of the title being edited in the dialog: the list row
  // reflects it live while typing, but nothing is written to the replica
  // until the dialog flushes on close (so no op-per-keystroke). Cleared
  // whenever the dialog closes, via any path.
  const [liveEdit, setLiveEdit] = createSignal<{
    id: string;
    text: string;
  } | null>(null);
  // Which field the dialog focuses on open — the note badge opens to notes,
  // everything else to the title. Reset to title whenever the dialog closes.
  // Lists this browser renders differently from their saved default.
  // Persisted per browser (not synced); absent ≡ follow the default.
  const [viewOverrides, setViewOverrides] = createSignal<Record<string, string>>(
    loadViewPrefs(),
  );
  // Override-map key: a user list id is unique across docs, but every
  // doc has an `inbox`, so a non-Home Inbox is keyed by its doc too.
  const listKey = (docId: string, listId: string): string =>
    docId === docs.homeDocId ? listId : `${docId}/${listId}`;
  // The list's saved default view, or null when none is saved. Inbox has
  // no ListMeta row, so its default lives in the doc-level settings.
  const savedViewIn = (docId: string, listId: string): ViewSpec | null => {
    const st = docs.appOf(docId)?.state;
    return parseView(
      listId === "inbox" ? st?.settings.inboxView : st?.listsById[listId]?.defaultView,
    );
  };
  const savedView = (listId: string): ViewSpec | null => savedViewIn(viewDocId(), listId);
  // What this client actually renders: local override, else the synced
  // default, else the built-in flat list.
  const listViewIn = (docId: string, listId: string): ViewSpec =>
    parseView(viewOverrides()[listKey(docId, listId)]) ??
    savedViewIn(docId, listId) ??
    LIST_VIEW;
  const listView = (listId: string): ViewSpec => listViewIn(viewDocId(), listId);
  const boardListId = createMemo((): string | null => {
    const v = view();
    return v.kind === "list" && listView(v.id).board ? v.id : null;
  });
  // Remount key for the board: a list id is unique across docs except
  // the reserved `inbox`, so the doc is part of it.
  const boardKey = createMemo((): string | null => {
    const id = boardListId();
    return id === null ? null : `${viewDocId()}/${id}`;
  });
  // The list currently on screen (board or flat), or null in Focus /
  // Done / Bin — the view-mode popover's subject.
  const currentListId = createMemo((): string | null => {
    const v = view();
    return v.kind === "list" ? v.id : null;
  });
  // Whether what this client renders is already the list's saved default
  // (so "save as default" would be a no-op).
  const isSavedDefault = (listId: string): boolean => {
    const saved = savedView(listId);
    return saved !== null && viewsEqual(listView(listId), saved);
  };
  const putViewOverrides = (map: Record<string, string>) => {
    setViewOverrides(map);
    try {
      localStorage.setItem(VIEW_PREF_KEY, JSON.stringify(map));
    } catch {
      // Quota/private-mode failures just lose the preference.
    }
  };
  // Record a local view choice. A choice that matches what this client
  // would render anyway drops the override instead of pinning it, so the
  // client keeps following the default if another device changes it.
  const setLocalView = (listId: string, next: ViewSpec) => {
    const map = { ...viewOverrides() };
    const key = listKey(viewDocId(), listId);
    if (viewsEqual(next, savedView(listId) ?? LIST_VIEW)) delete map[key];
    else map[key] = encodeView(next);
    putViewOverrides(map);
  };
  const toggleBoard = (listId: string) => {
    const cur = listView(listId);
    setLocalView(listId, { ...cur, board: !cur.board });
  };
  // Lane visibility (spec/board.md "Lane visibility") is part of the
  // view spec, so it resolves like the lens itself — local override,
  // else the saved default — and "Save as default" publishes it. Hiding
  // is display-only: a hidden lane simply doesn't render (so it is not a
  // drop target); no item's state changes.
  const showDoneColumn = (listId: string): boolean =>
    listView(listId).lanes.includes("done");
  const setShowDoneColumn = (listId: string, show: boolean) =>
    setLaneVisible(listId, "done", show);
  const visibleOpenLanes = (listId: string): WorkflowState[] =>
    listView(listId).lanes.filter((l) => l !== "done");
  const setLaneVisible = (listId: string, lane: WorkflowState, show: boolean) => {
    // At least one lane always renders: `withLane` refuses the hide that
    // would blank the board.
    const next = withLane(listView(listId), lane, show);
    if (next) setLocalView(listId, next);
  };
  // Per-lane item counts for the board whose view-mode popover is open,
  // shown beside each lane toggle so hiding a lane is an informed choice.
  // Mirrors Board.tsx laneMembers / doneMembers: open items partitioned
  // by workflow state, plus this list's closed-not-binned items. Empty
  // (all zero) while no board is mounted.
  const boardLaneCounts = createMemo((): Record<WorkflowState, number> => {
    const counts: Record<WorkflowState, number> = {
      backlog: 0,
      todo: 0,
      in_progress: 0,
      review: 0,
      done: 0,
      cancelled: 0, // never rendered: cancelled cards count under Done
    };
    const listId = boardListId();
    if (listId === null) return counts;
    const st = viewApp().state;
    for (const id of st.listOpen[listId] ?? []) {
      const it = st.itemsById[id];
      if (it) counts[it.state]++;
    }
    for (const it of Object.values(st.itemsById)) {
      if (it.listId === listId && isClosed(it) && !isBinned(it)) counts.done++;
    }
    return counts;
  });
  // Default capture lane for a board: its first visible open lane
  // (Backlog unless hidden), falling back to Backlog when only the Done
  // lane renders.
  const defaultCaptureLane = (listId: string): WorkflowState =>
    visibleOpenLanes(listId)[0] ?? "backlog";
  // Save what this client is looking at as the list's default view for
  // every device, and drop the local override so this client follows the
  // default it just set.
  const saveViewAsDefault = (listId: string) => {
    viewApp().setDefaultView(listId, encodeView(listView(listId)));
    const map = { ...viewOverrides() };
    delete map[listKey(viewDocId(), listId)];
    putViewOverrides(map);
  };
  // Client-local per-list "show state" flag for the flat list view.
  const [showStatePrefs, setShowStatePrefs] = createSignal<Record<string, true>>(
    loadShowStatePrefs(),
  );
  const showState = (listId: string): boolean => showStatePrefs()[listId] === true;
  const setShowState = (listId: string, show: boolean) => {
    const map = { ...showStatePrefs() };
    if (show) map[listId] = true;
    else delete map[listId];
    setShowStatePrefs(map);
    try {
      const ids = Object.keys(map);
      if (ids.length === 0) localStorage.removeItem(STATE_PREF_KEY);
      else localStorage.setItem(STATE_PREF_KEY, JSON.stringify(ids));
    } catch {
      // Quota/private-mode failures just lose the preference.
    }
  };
  // Whether this client shows the list any differently from its
  // defaults: a local view override, or the local "show state" flag.
  const hasLocalDisplay = (listId: string): boolean =>
    listKey(viewDocId(), listId) in viewOverrides() || showState(listId);
  // Drop this client's display choices for the list, so it renders the
  // saved default view (the flat list when none is saved) with state
  // badges off. Local only: the saved default itself is untouched.
  const restoreViewDefaults = (listId: string) => {
    const map = { ...viewOverrides() };
    delete map[listKey(viewDocId(), listId)];
    putViewOverrides(map);
    setShowState(listId, false);
  };
  // Draft state: a transient ItemView injected into dndItems but not into
  // the store. `insertIndex` is captured at draft-start time so collapse
  // commits at the same slot the user originally clicked from (e.g. after
  // their selection), even if peer ops shift the list around in the
  // meantime. expandedKey is controlled here so we can drive it open on
  // draft start and react when the controller collapses (Escape, click-
  // outside, or any other path).
  const [draft, setDraft] = createSignal<{
    item: ItemView;
    insertIndex: number;
    listId: string;
    // Focus-lens capture: the item is created in `listId` (the inbox) and
    // simultaneously pinned into Focus at `insertIndex` (a *focus* slot,
    // not a list slot). Plain list drafts leave this false.
    focus?: boolean;
  } | null>(null);
  const [expandedKey, setExpandedKey] = createSignal<string | null>(null);
  // Draft mirror. With the side pane showing, a draft also opens the
  // pane's capture surface on the same slot (`newItemTarget.draft`), so
  // the date, deadline, place, notes, list and pin are a click away while
  // the row is still being typed — the board's capture, with the row as
  // its title editor. `draftText` is the row's text as typed, which the
  // pane follows; `captureHandle` is how the row's settle commits through
  // the pane, so the fields picked there land on the item. A pointerdown
  // in the pane ends the row (`draftHandoff` marks the collapse that
  // drives, so `settleDraft` knows): a named draft becomes its item on
  // the spot and the pane follows it, an unnamed one leaves the pane as
  // an ordinary capture at the slot.
  const [draftText, setDraftText] = createSignal("");
  let captureHandle: CaptureHandle | null = null;
  let draftHandoff = false;

  // Touch viewports get taller rows so each item's a comfortable tap
  // target (the desktop height is too tight for a thumb). Dnd's cfg()
  // reads itemHeight reactively via setConfig, so flipping the signal
  // live-updates the controller.
  const itemsMobileMq = window.matchMedia("(max-width: 768px) and (pointer: coarse)");
  const [itemsIsMobile, setItemsIsMobile] = createSignal(itemsMobileMq.matches);
  const onItemsMqChange = (e: MediaQueryListEvent) => setItemsIsMobile(e.matches);
  itemsMobileMq.addEventListener("change", onItemsMqChange);
  onCleanup(() => itemsMobileMq.removeEventListener("change", onItemsMqChange));

  // One selection model per Workspace instance — the Dnd component is
  // re-keyed on view change (so it remounts), but we re-use the selection
  // object so consumers always read from the same handle. Stale block
  // anchors from the previous view's keys would resolve to position 0
  // (giving phantom selection at the top of the new list), so clear when
  // the view switches.
  const selection = new DndSelection();
  createEffect(
    on(
      view,
      (v) => {
        selection.clear();
        // A draft is scoped to the list it was started in; switching
        // away discards it (no save) and collapses — its mirrored
        // capture with it (dropped, not committed: the target is cleared
        // before the surface settles, so nothing is created).
        setDraft(null);
        setExpandedKey(null);
        if (untrack(newItemTarget)?.draft) setNewItemTarget(null);
        // Moving on abandons an unresolved item link.
        setPendingItem(null);
        // Navigating away closes the entered item, so the address bar
        // drops to the new view's token and the side pane re-follows
        // that view's selection instead of holding a row from the
        // previous one (`spec/urls.md`). The selection clear above
        // doesn't do this (an emptied selection leaves the item alone,
        // see the selection-follow effect). Exempt: an item entered
        // under this very view, i.e. a reveal that switched view and
        // entered in one batch.
        if (untrack(openItemId) !== null && openedViewKey !== viewKey(v)) {
          setOpenItemId(null);
        }
      },
      { defer: true },
    ),
  );

  // The selection the global item shortcuts act on: the board's active
  // column selection in board view (or null when nothing's selected there),
  // otherwise the flat list view's single selection. Handlers bail on null.
  const actionSelection = (): DndSelection | null =>
    boardListId() !== null ? boardSelection() : selection;

  // Linger group for the active list or Focus view: the unbroken chain of
  // recently-done items walking back from the latest click. A new
  // Done click within DONE_LINGER_MS of the previous extends the
  // whole chain, so a burst of clicks all leave together at the
  // latest's expiry; a click after a gap starts a fresh chain.
  // Sourced from a capture taken as each row left — the live
  // projection drops done items instantly, so this is the only record
  // of what just left (and where it sat, for the re-insert below).
  // A list view reads its doc's store (`recentDone`, the rows that left
  // this list). The Focus lens reads the personal layer
  // (`focusRecentDone`): Focus is Home's alone but shows items of every
  // doc, so only that layer knows the slot a row held in the merged
  // order, whichever doc the row lives in.
  interface LingerCapture {
    id: string;
    index: number;
    doneAt: number;
  }
  const recentDoneFor = (v: ViewKey): readonly LingerCapture[] =>
    v.kind === "focus"
      ? docs.focusRecentDone()
      : v.kind === "list"
        ? viewApp()
            .recentDone()
            .filter((r) => r.listId === v.id)
        : [];
  const lingerChain = createMemo(
    (): { ids: Set<string>; expiry: number } => {
      const v = view();
      if (v.kind !== "list" && v.kind !== "focus") {
        return { ids: new Set(), expiry: -Infinity };
      }
      const done = recentDoneFor(v)
        .filter((r) => {
          const it = docs.getItem(r.id);
          return it !== undefined && isClosed(it) && !isBinned(it);
        })
        .sort((a, b) => b.doneAt - a.doneAt);
      if (done.length === 0) return { ids: new Set(), expiry: -Infinity };
      const ids = new Set<string>();
      let prev = done[0].doneAt;
      for (const r of done) {
        if (prev - r.doneAt >= DONE_LINGER_MS) break;
        ids.add(r.id);
        prev = r.doneAt;
      }
      return { ids, expiry: done[0].doneAt + DONE_LINGER_MS };
    },
  );

  // Self-arms a single timeout for the chain's expiry — fires once
  // when the whole group should flush. Re-arms automatically when a
  // new click extends the chain (lingerChain memo changes).
  const [lingerTick, setLingerTick] = createSignal(0);
  createEffect(() => {
    lingerTick();
    const { expiry } = lingerChain();
    const remaining = expiry - Date.now();
    if (remaining <= 0) return;
    const t = setTimeout(() => setLingerTick((n) => n + 1), remaining);
    onCleanup(() => clearTimeout(t));
  });

  // Per-view item slice. A list view reads only its own
  // `state.listOpen[id]` array, so mutations in other lists (or in
  // Done/Bin) never invalidate it; item field edits track per-item
  // paths independently of the ordering. Done/Bin scan `itemsById`
  // lazily — the scan only runs while that view is active, and no
  // list-view mutation path pays for it.
  // Re-insert the linger group at the positions its rows vacated. Each
  // index was captured after earlier Done rows had already left the
  // projection, so replay the removals in reverse capture order to
  // reconstruct the original layout.
  const overlayLinger = (out: ItemView[], v: ViewKey): void => {
    lingerTick();
    const { ids: lingerIds, expiry } = lingerChain();
    if (Date.now() >= expiry) return;
    const captured: Array<{ index: number; value: ItemView }> = [];
    for (const r of recentDoneFor(v)) {
      if (!lingerIds.has(r.id)) continue;
      const it = docs.getItem(r.id);
      if (!it || !isClosed(it) || isBinned(it)) continue;
      captured.push({ index: r.index, value: it });
    }
    restoreCapturedPositions(out, captured);
  };
  // The agenda's day groups (`dayGroups.ts`), built here rather than in
  // `Upcoming` so the same rows, in the same order, feed `items()` below:
  // the selection-scoped shortcuts filter by what is on screen and pick
  // the survivor after a removal from that order.
  const upcomingGroups = createMemo((): DayGroup[] =>
    groupByDay(
      docs.calendarItems(),
      todayStamp(nowMs()),
      {
        overdue: m().deadline.overdue,
        unscheduled: m().upcoming.unscheduled,
        today: m().deadline.today,
        tomorrow: m().deadline.tomorrow,
      },
      locale(),
    ),
  );
  const items = createMemo((): ItemView[] => {
    const v = view();
    if (v.kind === "list") {
      const st = viewApp().state;
      const out: ItemView[] = [];
      for (const id of st.listOpen[v.id] ?? []) {
        const it = st.itemsById[id];
        if (it) out.push(it);
      }
      overlayLinger(out, v);
      return out;
    }
    if (v.kind === "focus") {
      // The Focus lens: Home's entries in curated order, foreign refs
      // resolved against their docs (spec/focus.md), Open-filtered and
      // deduped by the layer. Done auto-removes the ref, so the linger
      // overlay is the only thing keeping a just-completed row visible
      // for the strike-through beat.
      const out = [...docs.focusItems()];
      overlayLinger(out, v);
      return out;
    }
    if (v.kind === "done") {
      // Done view holds the closed items (done and cancelled) and
      // excludes binned ones: a closed-then-binned item lives in the Bin
      // (see context menu — Bin owns the next transition). Every open
      // doc's, narrowed by the header filter; sorted by the workflow
      // register's transition time desc.
      return docs.doneItems().filter(doneFilterAdmits(doneFilterNow()));
    }
    // Upcoming renders its own day-grouped surface (`Upcoming.tsx`), not
    // the flat listbox; its rows in agenda order are what the list
    // selection runs over there, so they are the view's rows here too.
    if (v.kind === "upcoming") {
      // Each item once, at its first row: a multi-day span repeats its
      // item down the days it covers, and the selection is by id.
      const seen = new Set<string>();
      const out: ItemView[] = [];
      for (const g of upcomingGroups()) {
        for (const r of g.rows) {
          if (seen.has(r.item.id)) continue;
          seen.add(r.item.id);
          out.push(r.item);
        }
      }
      return out;
    }
    // The Bin spans every doc: a binned row must stay reachable.
    return docs.binnedItems();
  });

  // Every user list in CRDT order — archived included. Use this for
  // name/title resolution, search provenance, and archived-list views;
  // the active/archived splits below feed the nav and destination
  // pickers (`spec/data-model.md` "Archived lists").
  const allListsOf = (docId: string): ListView[] => {
    const st = docs.appOf(docId)?.state;
    if (!st) return [];
    return st.listsOrder
      .map((id) => st.listsById[id])
      .filter((l): l is ListView => l !== undefined);
  };
  const activeListsOf = (docId: string): ListView[] =>
    allListsOf(docId).filter((l) => l.archivedAt == null);
  // The current view's doc's lists (the header title, the task surface's
  // move destinations for a capture here).
  const allLists = createMemo((): ListView[] => allListsOf(viewDocId()));

  // Per-list open-item counts (Backlog + Live) for the mobile Find sheet,
  // read straight off every doc's per-list projection arrays — no item
  // scan. User list ids are unique across docs; the reserved `inbox` is
  // Home's (the sheet's Inbox row is Home's).
  const openCountsByList = createMemo((): Record<string, number> => {
    const counts: Record<string, number> = {};
    for (const d of docs.docs()) {
      for (const [listId, ids] of Object.entries(d.app.state.listOpen)) {
        if (listId === "inbox" && !d.home) continue;
        counts[listId] = ids.length;
      }
    }
    return counts;
  });

  // The Upcoming entry's badge: what is owed or on today, with the most
  // urgent tone (`attentionBadge`). Unlike the list counts this scans the
  // items, as the Upcoming view itself does; the shared `nowMs` tick rolls
  // it over at midnight.
  const upcomingBadge = createMemo(() =>
    attentionBadge(docs.calendarItems(), todayStamp(nowMs())),
  );

  const dndRevision = createMemo(() => {
    const v = view();
    return `${v.kind}:${v.kind === "list" ? `${docs.viewDocId(v)}/${v.id}` : "-"}`;
  });

  // Id of the currently-viewed list iff it can be renamed. Only
  // user-created lists qualify: reserved `inbox` carries the localized
  // built-in label, and the done/bin cross-cutting views aren't lists.
  const editableListId = createMemo(() => {
    const v = view();
    return v.kind === "list" && v.id !== "inbox" ? v.id : null;
  });

  // The currently-viewed list's id iff that list is archived — drives
  // the header's Archived badge and flips the list menu's Archive /
  // Unarchive action.
  const archivedViewListId = createMemo((): string | null => {
    const v = view();
    if (v.kind !== "list") return null;
    return viewApp().state.listsById[v.id]?.archivedAt != null ? v.id : null;
  });

  // Display label for any list id, matching the header/nav rules: the
  // reserved `inbox` and `events` carry localized built-in labels, others
  // use the stored name, looked up in the item's doc (`docId`, else the
  // view's). Used by Done- and Bin-view rows to badge each item with its
  // origin list.
  const listLabel = (listId: string, docId?: string): string =>
    listId === "inbox"
      ? m().nav.inbox
      : listId === LIST_EVENTS
        ? m().nav.events
        : (docs.listOf(docId ?? viewDocId(), listId)?.name ?? listId);

  // Item attribution (`spec/sharing-plan.md` §Attribution), shown in the
  // item dialog's "Created …" line for every stamped item on a shared
  // doc, one's own included: on a shared doc, who added what is the
  // point. Home is one person, so nothing is shown there. The member
  // directory turns the stamped account id into an email; it loads once
  // per shared doc per session, and learns anyone the server announces
  // as present since (a member who joined after the load opens the doc
  // and is announced before their first item can arrive).
  const members = createMemberDirectory();
  // Creating a doc wraps a DEK: one workspace per accountless install
  // (`auth.md`), and the macOS app's window holds no key
  // (`Session.mirror`; its New Workspace… is the app's own menu), so
  // both hide the verb. Deleting touches no key, so the window offers
  // it too; the app confirms natively and its host does the work.
  const canManageDocs = (): boolean => {
    const s = session.session();
    return !s.anonymous && !s.mirror;
  };
  const canDeleteDocs = (): boolean => !session.session().anonymous;
  createEffect(() => {
    if (session.session().anonymous) return;
    for (const d of docs.docs()) if (!d.home) members.ensure(d.docId);
  });
  createEffect(() => {
    for (const d of docs.docs()) if (!d.home) members.learn(d.docId, props.presence(d.docId));
  });
  const creatorOf = (item: ItemView): { accountId: string; email: string | null } | null => {
    const who = item.createdBy;
    if (!who || item.docId === docs.homeDocId) return null;
    return { accountId: who, email: members.emailOf(item.docId, who) ?? null };
  };

  // The list an item is filed under in its doc, for writes that need a
  // real home: a reserved list or a ListMeta row that still exists, else
  // that doc's inbox.
  const homeListId = (docId: string, listId: string | undefined): string =>
    listId !== undefined &&
    (listId === LIST_EVENTS || docs.listOf(docId, listId) !== undefined)
      ? listId
      : "inbox";

  // `createComputed`, not `createEffect`: effects run after the first
  // render, so the dnd host would mount with an empty list and receive
  // the real one a beat later. Rows don't show that (they mount fresh),
  // but anything positioned after them — the trailer — would be laid
  // out at the top and then moved. Computed runs before render, so the
  // host's first render already has the rows.
  createComputed(() => {
    const next = items();
    const d = draft();
    if (!d) {
      setDndItems(next);
      return;
    }
    const merged = [...next];
    const at = Math.min(Math.max(d.insertIndex, 0), merged.length);
    merged.splice(at, 0, d.item);
    setDndItems(merged);
  });

  const onReorder = (op: DndOp<ItemView>) => {
    if (op.type !== "move") return;
    const v = view();
    if (v.kind === "focus") {
      // Drag-to-reorder within the Focus lens maps to `moveInFocus`,
      // whose indices address Home's full entry list (foreign and
      // not-yet-pruned entries included, spec/focus.md) — so the plan is
      // made over that list, not the visible rows; the dragged keys and
      // the drop neighbour are entries too.
      const ids = home.state.focusEntries.map((e) => e.itemId);
      const moves = planReorderMoves(
        ids,
        op.keys.map(String),
        op.beforeKey === null ? null : String(op.beforeKey),
      );
      if (moves.length === 0) return;
      home.withActionBatch(() => {
        for (const move of moves) home.moveInFocus(move.id, move.index);
      });
      return;
    }
    if (v.kind !== "list") return;
    const vApp = viewApp();
    const ids = items().map((it) => it.id);
    const moves = planReorderMoves(
      ids,
      op.keys.map(String),
      op.beforeKey === null ? null : String(op.beforeKey),
    );
    if (moves.length === 0) return;
    vApp.withActionBatch(() => {
      for (const move of moves) {
        vApp.moveItem(move.id, v.id, move.index);
      }
    });
  };

  // Capture new items straight into the Focus lens: each is created in the
  // inbox (Focus owns no items of its own — it's a lens) and pinned with a
  // FocusRef at a contiguous run starting at visible slot `atIndex`, all in
  // one undo step. Returns the new item ids.
  const captureToFocus = (texts: string[], atIndex: number): string[] => {
    // Focus slots are visible positions; the core addresses entries.
    // Resolve once, before the batch (the entry list only updates on
    // flush), and lay the run out from there.
    const entryAt = docs.focusEntryIndex(atIndex);
    return home.withActionBatch(() => {
      // Append to Home's inbox (its open length is the append slot — a
      // large sentinel would overflow the wasm `usize`); the item's inbox
      // position is independent of its Focus position, fixed explicitly
      // below.
      const inboxLen = home.state.listOpen["inbox"]?.length ?? 0;
      const ids = home.addItemsAt("inbox", texts, inboxLen);
      ids.forEach((id, i) => home.addToFocus(id, entryAt + i));
      return ids;
    });
  };

  // The calendar's Add: a new event in Home's events list, all-day on
  // today until the user picks otherwise.
  const newEventTarget = () => ({
    listId: LIST_EVENTS,
    docId: docs.homeDocId,
    state: "backlog" as const,
    when: todayStamp(nowMs()),
  });

  // Start a draft row: pseudo-item at `at`, or (the shortcut path) just
  // below the topmost selected item, or at the top if nothing is
  // selected. Expanding it via the controlled `expandedKey` flips the row
  // into edit mode through the same path used for existing rows. If a
  // draft is already open, no-op — the natural click-outside collapse on
  // the existing draft will settle it first.
  const startDraft = (at?: number) => {
    const v = view();
    // Capture runs in list views and in the Focus lens; a Focus draft
    // creates its item in the inbox and pins it (see settleDraft).
    if (v.kind !== "list" && v.kind !== "focus") return;
    if (draft() !== null) return;
    const isFocus = v.kind === "focus";
    const listId = isFocus ? "inbox" : v.id;
    const ids = items().map((i) => i.id);
    let insertIndex = 0;
    if (at !== undefined) {
      insertIndex = Math.min(Math.max(at, 0), ids.length);
    } else {
      const top = selection.getSelectionTop();
      if (top !== null) {
        const idx = ids.indexOf(String(top));
        if (idx >= 0) insertIndex = idx + 1;
      }
    }
    const id = `${DRAFT_ID_PREFIX}${crypto.randomUUID()}`;
    const now = Date.now();
    const draftItem: ItemView = {
      id,
      docId: isFocus ? docs.homeDocId : viewDocId(),
      listId,
      text: "",
      notes: "",
      state: "backlog",
      lifecycleAt: now,
      createdAt: now,
    };
    setDraft({ item: draftItem, insertIndex, listId, focus: isFocus });
    setExpandedKey(id);
    // The pane's capture surface on the same slot (see the draft mirror
    // note by `draftText`). A Focus draft creates in the inbox and pins
    // at its Focus slot, which the surface's default add cannot do, so
    // it creates through `captureToFocus`; unpinned, it files in the
    // inbox the way the pin-less half of that does.
    if (sidePanelShown()) {
      setDraftText("");
      setNewItemTarget({
        listId,
        docId: draftItem.docId,
        state: "backlog",
        index: insertIndex,
        draft: "row",
        focus: isFocus,
        create: isFocus
          ? (text, pin) =>
              pin
                ? captureToFocus([text], insertIndex)[0]
                : home.addItemAt(
                    "inbox",
                    text,
                    home.state.listOpen["inbox"]?.length ?? 0,
                  )
          : undefined,
      });
    }
  };

  // A pointerdown in the side pane while a draft row has the caret: end
  // the row here, before the browser moves focus, so the collapse reaches
  // `settleDraft` with `draftHandoff` set (the row's own blur then finds it
  // already collapsed and does nothing) and the pane shows the item, or
  // the capture, by the time the click lands. Capture phase, so a control
  // in the pane that swallows the event still gets this first.
  const onPanePointerDown = () => {
    const d = draft();
    if (!d || newItemTarget()?.draft !== "row") return;
    if (expandedKey() !== d.item.id) return;
    draftHandoff = true;
    try {
      setExpandedKey(null);
    } finally {
      draftHandoff = false;
    }
  };

  // The pane losing its room (a narrow window) takes the mirror with it:
  // the row carries on as a plain draft, and a pane-phase capture is
  // dropped rather than reopened as the modal.
  createEffect(() => {
    if (!sidePanelShown() && untrack(newItemTarget)?.draft) {
      setNewItemTarget(null);
    }
  });

  // The list's trailing capture slot: a placeholder row after the last
  // item that reads as the next item and starts a draft in that slot
  // when clicked. List and Focus views only; boards capture through the
  // dialog. It stands in for the draft while none is open there; a
  // draft started mid-list (Space below a selection) leaves it in place.
  // Touch viewports skip it: the header Add is the capture affordance
  // there, and a tap target that reads as a row invites mis-taps.
  const canDraft = () => view().kind === "list" || view().kind === "focus";
  const showTrailer = () => {
    if (!canDraft() || itemsIsMobile()) return false;
    const d = draft();
    return d === null || d.insertIndex < items().length;
  };

  // Called by the draft Row from its collapse effect. Empty text → drop;
  // non-empty → real item via addItemAt at the captured slot, then
  // re-anchor selection so the user lands on what they just created.
  // `chain` is true when the user pressed Enter; on a successful save
  // we open a fresh draft below the new item so capture continues. An
  // empty Enter still ends the chain (it falls through to the cancel
  // path below). `blur` marks a pointer-driven collapse (focus left the
  // row): a dropped draft then clears the selection instead of
  // re-anchoring it, since the click that is landing decides the
  // selection next and a re-anchor would only flash the row above until
  // it does. Returns whether a fresh draft follows.
  const settleDraft = (
    text: string,
    chain: boolean,
    blur = false,
    when: string | null = null,
  ): boolean => {
    const d = draft();
    if (!d) return false;
    const nw = newItemTarget();
    const mirrored = nw?.draft !== undefined;
    // The plain add, for a draft with no pane capture to commit through.
    const createPlain = (t: string): string =>
      d.focus
        ? captureToFocus([t], d.insertIndex)[0]
        : viewApp().addItemAt(d.listId, t, d.insertIndex);
    // The date a phrase in the row's text resolved to (`whenToken.ts`):
    // set on the item once it exists, whichever path made it. A Focus
    // draft's item lives in Home.
    const applyWhen = (id: string) => {
      if (when) (d.focus ? home : viewApp()).setItemWhen(id, when);
    };
    // Re-anchor the selection on the row above the slot (or the slot
    // itself when nothing is above), so a dropped draft lands the user
    // back near where they were.
    const reanchor = () => {
      const rest = items();
      if (rest.length === 0) selection.clear();
      else selection.selectOnly(rest[Math.max(0, d.insertIndex - 1)].id);
    };
    setDraft(null);
    if (draftHandoff && nw?.draft === "row") {
      // Pointed into the pane. A named draft is its item from here: made
      // now, in one pass with the row leaving (this runs inside the row's
      // collapse effect, so the list sees one update), and the pane
      // promoted onto it in place — same elements, buffers kept — so the
      // click that is landing still finds the chip it aimed at (a date
      // picker opens on this very click) and the caret goes where it was
      // put. The item is entered by that; the selection catches up once
      // the list has it. An unnamed draft leaves the pane as a capture
      // on the slot; the selection falls back to the row above, and
      // `onCreated` moves it to the item should the capture commit.
      if (text) {
        let id = captureHandle?.promote(text) ?? null;
        if (id === null) {
          // No pane handle (the surface mid-remount): plain add, and the
          // pane follows the selection to it.
          id = createPlain(text);
          setNewItemTarget(null);
        }
        const newId = id;
        applyWhen(newId);
        queueMicrotask(() => selection.selectOnly(newId));
      } else {
        setNewItemTarget({ ...nw, draft: "pane" });
        queueMicrotask(reanchor);
      }
      return true;
    }
    if (!text) {
      if (mirrored) setNewItemTarget(null);
      // Cancel path. The dnd's applyExpanded(draftId) replaced selection
      // with the draft id; once setDraft(null) drops it from the order,
      // the leftover block's anchor stops resolving and the selection
      // chrome snaps to the first item. Escape re-anchors on the row
      // immediately above the captured slot (or the slot itself when
      // nothing is above) so cancel lands the user back near where they
      // were; a blur just clears, the landing click selects what it hit.
      if (blur) selection.clear();
      else reanchor();
      return false;
    }
    // Through the pane's capture when it mirrors this draft, so the fields
    // picked there land on the item (the handle is the pane's; it is null
    // only between the surface remounting, when the plain add stands in).
    const newId =
      (mirrored ? captureHandle?.commit(text) : null) ?? createPlain(text);
    applyWhen(newId);
    // The target clears after the commit: the surface's settle on a
    // cleared target creates nothing, and the pane goes back to following
    // the selection, which lands on the new item below.
    if (mirrored) setNewItemTarget(null);
    // Notes are no longer captured inline — a new item starts noteless and
    // the user adds notes later by opening it in the detail dialog.
    // The store dispatch that adds the new item runs before this
    // microtask, so by then `selection.updateOrder` has already seen
    // the new id and the selection anchor is valid. When chaining,
    // startDraft reads the topmost selection to pick the insert slot,
    // so the selectOnly above must land first — same microtask, same
    // ordering.
    queueMicrotask(() => {
      selection.selectOnly(newId);
      if (chain) startDraft();
    });
    return chain;
  };

  // Paste anywhere in a list view drops the clipboard contents in as items.
  // A block copied from Monoplan carries a structured payload (itemClip.ts)
  // and pastes as full clones: notes, dates, duration and, off the board,
  // workflow state. Anything else pastes one item per non-empty line. Skip
  // when the paste targets an editable element (add form, row edit, list
  // rename) so normal paste still works there. If any rows are selected,
  // insert immediately after the last-selected one; otherwise append.
  const onPaste = (e: ClipboardEvent) => {
    if (isOverlayOpen()) return;
    const v = view();
    if (v.kind !== "list" && v.kind !== "focus") return;
    const target = e.target as Element | null;
    if (target?.closest('input, textarea, [contenteditable="true"]')) return;
    const clip = readClip(e.clipboardData ?? null);
    const structured = clip !== null && clip.length > 0;
    let entries: ClipItem[];
    if (structured) {
      entries = clip;
    } else {
      const data = e.clipboardData?.getData("text") ?? "";
      entries = data
        .split(/\r?\n/)
        .map((l) => l.trim().replace(/^-\s+(?:\[[^\]]*\]\s*)?/, ""))
        .filter((l) => l.length > 0)
        .map((text) => ({ text, state: "backlog" as const }));
    }
    if (entries.length === 0) return;
    e.preventDefault();
    const texts = entries.map((c) => c.text);
    // The doc the paste lands in: Home for a Focus capture, else the
    // list's own.
    const pasteApp = v.kind === "focus" ? home : viewApp();

    // Fill each fresh item in from its clip entry. Items are created as
    // Backlog; a copied open state is re-applied in the same undo step,
    // which keeps the just-assigned Open position (spec/board.md). A closed
    // state is never carried: a pasted item is a fresh open task, as with Cmd+D.
    // `lane` overrides the copied state wholesale (the board's anchor).
    // Written through `target` directly: inside the batch the store has
    // not seen the new ids yet, so the routed layer could not place them.
    const fillFromClip = (ids: string[], lane?: WorkflowState): void => {
      ids.forEach((id, i) => {
        const c = entries[i];
        if (!c) return;
        copyItemDetails(pasteApp, id, c);
        const st =
          lane ?? (structured && !isClosedState(c.state) ? c.state : "backlog");
        if (st !== "backlog") pasteApp.setLifecycle(id, st);
      });
    };

    // Board: land the block below the bottom-most selected card, in that
    // card's lane — same shape as duplicateBlock. Slots come straight from
    // the shared Open order (Backlog and Live are one array), so a Done-lane
    // or empty selection finds no slot and the block appends, each card in
    // its copied lane (or Backlog for plain text).
    const boardId = boardListId();
    if (boardId !== null) {
      const linear = pasteApp.state.listOpen[boardId] ?? [];
      const selectedSlots = (actionSelection()?.getSelectedKeys() ?? [])
        .map((k) => linear.indexOf(String(k)))
        .filter((idx) => idx >= 0);
      const anchorIdx =
        selectedSlots.length === 0 ? -1 : Math.max(...selectedSlots);
      const anchor = anchorIdx >= 0 ? pasteApp.getItem(linear[anchorIdx]) : undefined;
      const boardIds = pasteApp.withActionBatch(() => {
        const created = pasteApp.addItemsAt(
          boardId,
          texts,
          anchorIdx >= 0 ? anchorIdx + 1 : linear.length,
        );
        fillFromClip(created, anchor?.state);
        return created;
      });
      if (boardIds.length === 0) return;
      // Hand the block to the reveal path so it becomes the active lane
      // selection and scrolls into view.
      setBoardRevealIds(boardIds);
      return;
    }

    const visible = items().map((it) => it.id);
    const selectedHere = selection
      .getSelectedKeys()
      .map((k) => visible.indexOf(String(k)))
      .filter((idx) => idx >= 0);
    const insertAt =
      selectedHere.length === 0 ? visible.length : Math.max(...selectedHere) + 1;
    const ids = pasteApp.withActionBatch(() => {
      const created =
        v.kind === "focus"
          ? captureToFocus(texts, insertAt)
          : pasteApp.addItemsAt(v.id, texts, insertAt);
      fillFromClip(created);
      return created;
    });
    if (ids.length === 0) return;
    // Wait for the dnd's source to absorb the new ids — see the
    // matching note in onDuplicate.
    queueMicrotask(() => {
      selection.selectOnly(ids[0]);
      if (ids.length > 1) selection.extendActive(ids[ids.length - 1]);
    });
  };
  document.addEventListener("paste", onPaste);
  onCleanup(() => document.removeEventListener("paste", onPaste));

  // Selection-scoped key actions (⌫, x, move) filter the selection down
  // to the ids that are actually on screen. On a board, the Done lane's
  // members come from a scan of `itemsById` (Board's `doneMembers`), not
  // the list's Open projection — so `items()` covers only the open lanes.
  // Add this board's closed-but-not-binned items so a card selected in the
  // Done lane is actionable too.
  const withBoardDone = (visibleSet: Set<string>): Set<string> => {
    const boardId = boardListId();
    if (boardId !== null) {
      for (const it of Object.values(viewApp().state.itemsById)) {
        if (it.listId === boardId && isClosed(it) && !isBinned(it)) {
          visibleSet.add(it.id);
        }
      }
    }
    return visibleSet;
  };

  // The active selection's ids, filtered to what's actually on screen
  // (visible order). `boardDone` also admits the board's Done-lane cards
  // (see `withBoardDone`); the Focus toggle leaves them out since a done
  // item can't be pinned. Empty when nothing's selected.
  const selectedVisibleIds = (boardDone: boolean): string[] => {
    const sel = actionSelection();
    if (!sel) return [];
    const visible = new Set(items().map((it) => it.id));
    const visibleSet = boardDone ? withBoardDone(visible) : visible;
    return sel
      .getSelectedKeys()
      .map(String)
      .filter((id) => visibleSet.has(id));
  };

  // The row to select once `removed` leaves the view: the first survivor
  // after the bottom-most removed row, else the new last survivor, else
  // null when nothing is left. `visibleIds` is the pre-removal order.
  const survivorAfter = (
    visibleIds: readonly string[],
    removed: ReadonlySet<string>,
  ): string | null => {
    let lastIdx = -1;
    for (let i = visibleIds.length - 1; i >= 0; i--) {
      if (removed.has(visibleIds[i])) {
        lastIdx = i;
        break;
      }
    }
    for (let i = lastIdx + 1; i < visibleIds.length; i++) {
      if (!removed.has(visibleIds[i])) return visibleIds[i];
    }
    for (let i = visibleIds.length - 1; i >= 0; i--) {
      if (!removed.has(visibleIds[i])) return visibleIds[i];
    }
    return null;
  };

  // Re-anchor `sel` on `nextId` after rows left the view. The survivor is
  // chosen in list order, which on the board may sit in a different
  // column than the active selection — so only re-select in the flat
  // list view; on the board just drop the (now-removed) selection.
  const selectSurvivor = (sel: DndSelection, nextId: string | null) => {
    if (nextId === null || boardListId() !== null) {
      sel.clear();
    } else {
      // Wait for the dnd source to absorb the removals before
      // selecting — matches onDuplicate/onPaste.
      queueMicrotask(() => sel.selectOnly(nextId));
    }
  };

  // Bin live or done items, hard-delete binned ones (the Bin view), then
  // move the selection onto a survivor. Shared by ⌫ and the side panel's
  // multi-select actions; `ids` are already visibility-filtered.
  const binOrDeleteIds = (ids: string[]) => {
    const sel = actionSelection();
    if (!sel || ids.length === 0) return;
    const v = view();
    const nextId = survivorAfter(
      items().map((it) => it.id),
      new Set(ids),
    );
    if (v.kind === "bin") routed.deleteBinnedMany(ids);
    else routed.setBinnedMany(ids, true);
    selectSurvivor(sel, nextId);
  };

  // Delete / Backspace on the active view: bin live or done items, hard-
  // delete binned ones. Skip when focus is inside an editable surface so
  // the AddForm, row edit, and list rename keep their native behaviour.
  const onDeleteKey = (e: KeyboardEvent) => {
    if (e.key !== "Delete" && e.key !== "Backspace") return;
    const ids = selectedVisibleIds(true);
    if (ids.length === 0) return;
    e.preventDefault();
    binOrDeleteIds(ids);
  };
  onGlobalKey(onDeleteKey);

  // Every id resolves to a closed (done or cancelled) item. False for an
  // empty set.
  const allClosedIds = (ids: string[]): boolean =>
    ids.length > 0 &&
    ids.every((id) => {
      const it = docs.getItem(id);
      return it !== undefined && isClosed(it);
    });
  const allCancelledIds = (ids: string[]): boolean =>
    ids.length > 0 &&
    ids.every((id) => {
      const it = docs.getItem(id);
      return it !== undefined && isCancelled(it);
    });

  // Toggle done on `ids`. Direction follows the group: any open → mark
  // all done, only flip back to open when every item is already closed
  // (a cancelled item reopens the same way). Shared by `x` and the side
  // panel's multi-select actions.
  const toggleDoneIds = (ids: string[]) => {
    if (ids.length === 0) return;
    const allClosed = allClosedIds(ids);
    if (allClosed && view().kind === "focus") routed.undoneIntoFocus(ids);
    else routed.setDoneMany(ids, !allClosed);
  };

  // Toggle cancelled on `ids`: any not-cancelled → cancel all; only
  // reopen (to Backlog, the core's un-done rule) when every item is
  // already cancelled. Shared by shift+x, the context menu and the side
  // panel.
  const toggleCancelledIds = (ids: string[]) => {
    if (ids.length === 0) return;
    if (allCancelledIds(ids)) {
      if (view().kind === "focus") routed.undoneIntoFocus(ids);
      else routed.setDoneMany(ids, false);
    } else {
      routed.setLifecycleMany(ids, "cancelled");
    }
  };

  // x: toggle done on the current selection. Mirrors the row checkbox and
  // the context menu's Mark done / Mark not done. Skip when focus is in an
  // editable surface so a literal "x" typed into a row/AddForm lands as
  // text. Includes the board's Done-lane cards so x on one un-does it
  // (back to Backlog — the core's undone rule, not the prior state).
  const onToggleDoneKey = (e: KeyboardEvent) => {
    if (e.key !== "x" && e.key !== "X") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const ids = selectedVisibleIds(true);
    if (ids.length === 0) return;
    e.preventDefault();
    toggleDoneIds(ids);
  };
  onGlobalKey(onToggleDoneKey);

  // shift+x: toggle cancelled on the current selection. Mirrors the
  // context menu's Cancel / Reopen. Same editable-surface guard as `x`.
  const onToggleCancelKey = (e: KeyboardEvent) => {
    if (e.key !== "x" && e.key !== "X") return;
    if (e.metaKey || e.ctrlKey || e.altKey || !e.shiftKey) return;
    const ids = selectedVisibleIds(true);
    if (ids.length === 0) return;
    e.preventDefault();
    toggleCancelledIds(ids);
  };
  onGlobalKey(onToggleCancelKey);

  // Every id is in the Focus lens. False for an empty set.
  const allFocusedIds = (ids: string[]): boolean =>
    ids.length > 0 && ids.every((id) => docs.isFocused(id));

  // Toggle Focus on `ids`. Direction follows the group like toggle-done:
  // any not-focused → add all (the core skips ids that are already
  // focused or not Open), only remove when every item is already in the
  // Focus lens. Shared by `s` and the side panel's multi-select actions.
  const toggleFocusIds = (ids: string[]) => {
    if (ids.length === 0) return;
    if (allFocusedIds(ids)) routed.removeFromFocusMany(ids);
    else routed.addToFocusMany(ids);
  };

  // s: toggle Focus (starred) on the current selection. Mirrors the row context
  // menu's Add to focus / Remove from focus.
  const onToggleFocusKey = (e: KeyboardEvent) => {
    if (e.key !== "s" && e.key !== "S") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const ids = selectedVisibleIds(false);
    if (ids.length === 0) return;
    e.preventDefault();
    toggleFocusIds(ids);
  };
  onGlobalKey(onToggleFocusKey);

  // Open the move palette on `sourceIds`, kept in visible order so the
  // block lands in the target list in the user's visible sequence. Board
  // Done-lane cards aren't in `items()` (same situation as ⌫ above) —
  // they survive the visibility filter via the extended set and follow
  // the in-view rows.
  const openMovePalette = (sourceIds: readonly string[]) => {
    const visibleIds = items().map((it) => it.id);
    const visibleSet = withBoardDone(new Set(visibleIds));
    const sourceSet = new Set(sourceIds.map(String));
    const inView = visibleIds.filter((id) => sourceSet.has(id));
    const inViewSet = new Set(inView);
    const rest = [...sourceSet].filter(
      (id) => visibleSet.has(id) && !inViewSet.has(id),
    );
    const ids = [...inView, ...rest];
    if (ids.length > 0) setMoveIds(ids);
  };

  // The doc the move palette lists first: the doc of the rows being
  // moved (the first row's, for a mixed selection in a lens), else the
  // view's. The other workspaces' lists follow, tagged with their name.
  const moveDocId = (): string => {
    const first = moveIds()?.[0];
    return (first !== undefined ? docs.docIdOf(first) : undefined) ?? viewDocId();
  };
  // The list a block of rows all live in, or null when they span lists
  // or docs. Read off the rows themselves rather than the view, so it
  // holds in the cross-list lenses (Starred / Upcoming / Done / Bin) too.
  const housingList = (
    ids: readonly string[],
  ): { docId: string; listId: string } | null => {
    let found: { docId: string; listId: string } | null = null;
    for (const id of ids) {
      const it = docs.getItem(id);
      const docId = docs.docIdOf(id);
      if (!it || docId === undefined) continue;
      if (found === null) found = { docId, listId: it.listId };
      else if (found.docId !== docId || found.listId !== it.listId) return null;
    }
    return found;
  };
  // The list the move palette ticks as current: the one housing every
  // acted-on row; none when the selection spans lists.
  const moveHousing = createMemo(() => housingList(moveIds() ?? []));
  const moveIsCurrent = (opt: ListOption): boolean => {
    const h = moveHousing();
    return h !== null && h.listId === opt.id && h.docId === opt.docId;
  };

  // Move destinations, per doc: Inbox, Events (moving there makes an
  // item an event, `spec/events-plan.md`), then every active user list —
  // archived lists are not offered, matching the task dialog's picker.
  // A shared workspace's lists are tagged with its name behind the
  // people glyph, whichever workspace is being worked in (`own`);
  // Personal's go unsaid, as everywhere. A list outside `own` is
  // `foreign`: picking it is a move between workspaces.
  const listOptionsOf = (docId: string, own: string): ListOption[] => {
    const workspace = docId === docs.homeDocId ? undefined : docLabel(docId);
    const foreign = docId !== own;
    return [
      { id: "inbox", name: m().nav.inbox, docId, workspace, foreign },
      { id: LIST_EVENTS, name: m().nav.events, docId, workspace, foreign },
      ...activeListsOf(docId).map((l) => ({
        id: l.id,
        name: l.name,
        icon: l.icon,
        docId,
        workspace,
        foreign,
      })),
    ];
  };
  /** Every other workspace's destinations, in nav order. */
  const otherListOptions = (own: string): ListOption[] =>
    docs
      .docs()
      .filter((d) => d.docId !== own)
      .flatMap((d) => listOptionsOf(d.docId, own));
  const moveListOptions = createMemo<ListOption[]>(() => {
    const own = moveDocId();
    return [...listOptionsOf(own, own), ...otherListOptions(own)];
  });

  // A move between workspaces (`spec/sharing-plan.md` "Cross-doc move")
  // is confirmed only when it changes who can see the rows: into a
  // shared workspace its members gain them, out of one its members lose
  // them. Between the account's own unshared workspaces it just
  // happens, and Cmd+Z takes it back. A non-Home doc counts as shared
  // unless its member list is loaded and says one.
  const [crossMove, setCrossMove] = createSignal<{
    title: string;
    message: string;
    run: () => void;
  } | null>(null);
  // Separate from the record so the text stays put while the dialog
  // closes.
  const [crossMoveOpen, setCrossMoveOpen] = createSignal(false);
  const hasOtherMembers = (docId: string): boolean =>
    docId !== docs.homeDocId && members.countOf(docId) !== 1;
  const confirmingCrossDoc = (
    ids: readonly string[],
    targetDocId: string,
    run: () => void,
  ): void => {
    const sourceOf = (id: string): string | undefined => {
      const docId = docs.docIdOf(id);
      return docId === targetDocId ? undefined : docId;
    };
    const crossing = ids.filter((id) => sourceOf(id) !== undefined);
    const parts: string[] = [];
    if (crossing.length > 0 && hasOtherMembers(targetDocId)) {
      parts.push(
        m().workspace.moveAcrossExposes(
          docLabel(targetDocId),
          members.countOf(targetDocId) ?? null,
          crossing.length,
        ),
      );
    }
    const leaving = crossing.filter((id) => hasOtherMembers(sourceOf(id)!));
    if (leaving.length > 0) {
      const sources = [...new Set(leaving.map((id) => sourceOf(id)!))];
      parts.push(
        m().workspace.moveAcrossRemoves(sources.map(docLabel).join(", "), leaving.length),
      );
    }
    if (parts.length === 0) {
      run();
      return;
    }
    setCrossMove({
      title: m().workspace.moveAcrossTitle(docLabel(targetDocId)),
      message: parts.join(" "),
      run,
    });
    setCrossMoveOpen(true);
  };

  // Land `ids` (visible order) as the first items of a list of `tApp`:
  // rows of another doc cross over (ids kept, binned rows restored,
  // closed rows still closed; one undo step across the docs), rows
  // already in the doc are un-binned and relocated in one step of
  // their own. A mixed block puts the doc's own rows above the
  // arrivals.
  const relocateTo = (ids: readonly string[], tApp: DocApp, listId: string): void => {
    const own = ids.filter((id) => tApp.getItem(id) !== undefined);
    const arriving = ids.filter((id) => tApp.getItem(id) === undefined);
    if (arriving.length > 0) docs.moveToDoc(arriving, tApp.docId, listId, 0);
    if (own.length === 0) return;
    const toUnbin = own.filter((id) => {
      const it = tApp.getItem(id);
      return it !== undefined && isBinned(it);
    });
    tApp.withActionBatch(() => {
      if (toUnbin.length > 0) tApp.setBinnedMany(toUnbin, false);
      for (const [i, id] of own.entries()) {
        tApp.moveItem(id, listId, i);
      }
    });
  };

  // Commit a palette pick: mirrors the drag-into-nav drop above — un-bin
  // first, then land as the first items of the target list in visible
  // order, all one undo step. A cross-list move is a relocation only: Done
  // items stay Done and land in the target's Done lane (`spec/board.md`).
  // Picking the list the rows already live in (the ticked one) is a
  // no-op (not a reorder-to-top surprise), matching the task dialog's
  // picker — unless a row is binned, where the pick restores it.
  const moveBlockToList = (
    sourceIds: readonly string[],
    targetDocId: string,
    targetListId: string,
  ) => {
    const mApp = docs.appOf(targetDocId);
    if (!mApp) return;
    const ids = sourceIds.filter((id) => docs.getItem(id) !== undefined);
    if (ids.length === 0) return;
    const h = housingList(ids);
    const anyBinned = ids.some((id) => {
      const it = docs.getItem(id);
      return it !== undefined && isBinned(it);
    });
    if (h !== null && h.listId === targetListId && h.docId === mApp.docId && !anyBinned) {
      return;
    }
    confirmingCrossDoc(ids, mApp.docId, () => landBlock(ids, mApp, targetListId));
  };
  const landBlock = (ids: readonly string[], mApp: DocApp, targetListId: string) => {
    const sel = actionSelection();
    const beforeIds = items().map((it) => it.id);
    relocateTo(ids, mApp, targetListId);
    // Where the moved rows stay in view (Focus and Done are cross-list
    // lenses) the selection stays with them. Where they leave (a list,
    // or the Bin, which the un-bin empties them out of) a lingering
    // selection would be a phantom block anchor (see the drag-out
    // handling below), so re-anchor on the nearest survivor as ⌫ does.
    if (sel) {
      const after = new Set(items().map((it) => it.id));
      const moved = new Set(ids);
      const stillHere = ids.some((id) => after.has(id));
      if (!stillHere) selectSurvivor(sel, survivorAfter(beforeIds, moved));
    }
    // One item made an event with no date: open it on its date popover,
    // as Set date… does, rather than leave it in Unscheduled unasked. A
    // block just moves; its undated rows land in Unscheduled.
    if (targetListId === LIST_EVENTS && ids.length === 1) {
      const it = mApp.getItem(ids[0]!);
      if (it && isOpen(it) && !it.when) openItemWhen(it.id);
    }
  };

  // m: open the move palette on the current selection.
  const onMoveKey = (e: KeyboardEvent) => {
    if (e.key !== "m" && e.key !== "M") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const sel = actionSelection();
    if (!sel) return;
    const ids = sel.getSelectedKeys().map(String);
    if (ids.length === 0) return;
    e.preventDefault();
    openMovePalette(ids);
  };
  onGlobalKey(onMoveKey);

  // Duplicate live items as a contiguous block immediately after the
  // bottom-most source row — same shape as paste — rather than each
  // clone sitting under its own original. Shared by Cmd+D and the row
  // context menu's Duplicate action so both behave identically.
  // A clone is a full copy, not just the title: carry the notes and both
  // dates across. Each is a no-op when the source lacks it, so the batch
  // stays a single undo step either way.
  // Written through the creating doc's own app: inside a batch the store
  // has not seen the new id yet, so the routed layer could not place it.
  const copyItemDetails = (
    target: DocApp,
    id: string,
    src: {
      notes?: string;
      deadline?: string;
      when?: string;
      duration?: number;
      place?: Place;
    },
  ): void => {
    if (src.notes) target.setItemNotes(id, src.notes);
    if (src.deadline) target.setItemDeadline(id, src.deadline);
    if (src.when) target.setItemWhen(id, src.when);
    if (src.when && src.duration) target.setItemDuration(id, src.duration);
    if (src.place) target.setItemPlace(id, src.place);
  };

  const duplicateBlock = (sourceIds: readonly string[]): void => {
    const v = view();
    if (v.kind === "focus" || v.kind === "upcoming") {
      // Focus and the agenda own no items — each clone is created in its
      // source's own list directly below its source (inheriting the
      // source's lane), falling back to the top of the inbox only if that
      // list is gone. In Focus the clones are then pinned as a contiguous
      // block right after the bottom-most source's Focus slot; on the
      // agenda the copied dates place them. One undo step.
      const pinToFocus = v.kind === "focus";
      const visible = items().map((it) => it.id);
      const sourceSet = new Set(sourceIds);
      const sources: {
        id: string;
        idx: number;
        text: string;
        notes: string;
        deadline: string | undefined;
        when: string | undefined;
        duration: number | undefined;
        place: Place | undefined;
        state: WorkflowState;
        listId: string;
        /** The source's doc: the clone is created there. */
        app: DocApp;
      }[] = [];
      visible.forEach((id, idx) => {
        if (!sourceSet.has(id)) return;
        const it = docs.getItem(id);
        const sApp = it ? docs.appOf(it.docId) : undefined;
        if (!it || !sApp || !isOpen(it)) return;
        const listId = homeListId(it.docId, it.listId);
        sources.push({
          id,
          idx,
          text: it.text,
          notes: it.notes,
          deadline: it.deadline,
          when: it.when,
          duration: it.duration,
          place: it.place,
          state: it.state,
          listId,
          app: sApp,
        });
      });
      if (sources.length === 0) return;
      const insertAt = sources[sources.length - 1].idx + 1;
      // Focus slots are visible positions; the core addresses Home's
      // entries. Resolve once before the batch (entries only update on
      // flush) and lay the block out from there.
      const entryAt = pinToFocus ? docs.focusEntryIndex(insertAt) : 0;
      // `state.listOpen` doesn't update until the batch flushes, so track
      // each touched list's Open order locally to keep the slots right when
      // several sources share a list (keyed by doc too: every doc has an
      // `inbox`).
      const openOrder: Record<string, string[]> = {};
      const newIds = routed.withActionBatch(() => {
        const created: string[] = [];
        sources.forEach((s, i) => {
          const order = (openOrder[`${s.app.docId}/${s.listId}`] ??= [
            ...(s.app.state.listOpen[s.listId] ?? []),
          ]);
          // Source missing from its own list's Open order (the inbox
          // fallback above, or a stale projection) — land the clone on top.
          const at = order.indexOf(s.id) + 1;
          const id = s.app.addItemAt(s.listId, s.text, at);
          order.splice(at, 0, id);
          copyItemDetails(s.app, id, s);
          if (s.state !== "backlog") s.app.setLifecycle(id, s.state);
          if (pinToFocus) {
            if (s.app === home) home.addToFocus(id, entryAt + i);
            else home.addForeignToFocus(s.app.docId, id, entryAt + i);
          }
          created.push(id);
        });
        return created;
      });
      if (newIds.length === 0) return;
      queueMicrotask(() => {
        selection.selectOnly(newIds[0]);
        if (newIds.length > 1)
          selection.extendActive(newIds[newIds.length - 1]);
      });
      return;
    }
    if (v.kind !== "list") return;
    const vApp = viewApp();
    const visible = items().map((it) => it.id);
    const sourceSet = new Set(sourceIds);
    // Clones inherit the source's board lane: a dupe of an In Progress
    // card stays In Progress, a Backlog dupe stays Backlog.
    const sourcesInOrder: {
      idx: number;
      text: string;
      notes: string;
      deadline: string | undefined;
      when: string | undefined;
      duration: number | undefined;
      place: Place | undefined;
      state: WorkflowState;
    }[] = [];
    visible.forEach((id, idx) => {
      if (!sourceSet.has(id)) return;
      const it = vApp.getItem(id);
      if (!it || !isOpen(it)) return;
      sourcesInOrder.push({
        idx,
        text: it.text,
        notes: it.notes,
        deadline: it.deadline,
        when: it.when,
        duration: it.duration,
        place: it.place,
        state: it.state,
      });
    });
    if (sourcesInOrder.length === 0) return;
    const insertAt = sourcesInOrder[sourcesInOrder.length - 1].idx + 1;
    const texts = sourcesInOrder.map((s) => s.text);
    // Create the block (as Backlog), then flip each clone to its source's
    // lane in the same undo step. The state flip keeps the clone's
    // just-assigned Open position (spec/board.md).
    const newIds = vApp.withActionBatch(() => {
      const ids = vApp.addItemsAt(v.id, texts, insertAt);
      ids.forEach((id, i) => {
        const src = sourcesInOrder[i];
        if (!src) return;
        copyItemDetails(vApp, id, src);
        if (src.state !== "backlog") vApp.setLifecycle(id, src.state);
      });
      return ids;
    });
    if (newIds.length === 0) return;
    if (boardListId() !== null) {
      // On the board, hand the whole clone block to the reveal path so it
      // becomes the active lane selection and scrolls into view.
      setBoardRevealIds(newIds);
      return;
    }
    // Wait for the dnd's source to absorb the new ids — selectOnly on a
    // key the order map doesn't yet know about leaves it visually
    // unselected.
    queueMicrotask(() => {
      selection.selectOnly(newIds[0]);
      if (newIds.length > 1) selection.extendActive(newIds[newIds.length - 1]);
    });
  };

  // Copy items to the clipboard, in visible order. Plain text is just the
  // titles, one per line; the html and custom types carry the full items
  // so a paste back into Monoplan clones them (itemClip.ts). With the
  // `copy` event's DataTransfer every type is set synchronously; without
  // one (context menu, palette) the async API takes the standard two.
  const copyBlock = (
    sourceIds: readonly string[],
    dt?: DataTransfer | null,
  ): void => {
    const visible = items().map((it) => it.id);
    const sourceSet = new Set(sourceIds);
    const inOrder: ItemView[] = [];
    visible.forEach((id) => {
      if (!sourceSet.has(id)) return;
      const it = docs.getItem(id);
      if (it) inOrder.push(it);
    });
    if (inOrder.length === 0) {
      for (const id of sourceIds) {
        const it = docs.getItem(id);
        if (it) inOrder.push(it);
      }
    }
    if (inOrder.length === 0) return;
    writeClip(inOrder.map(toClipItem), dt);
  };

  // Cmd/Ctrl+D: duplicate the current selection.
  const onDuplicateKey = (e: KeyboardEvent) => {
    if (e.key !== "d" && e.key !== "D") return;
    if (!(e.metaKey || e.ctrlKey)) return;
    if (e.shiftKey || e.altKey) return;
    const ids = actionSelection()?.getSelectedKeys().map(String) ?? [];
    if (ids.length === 0) return;
    e.preventDefault();
    duplicateBlock(ids);
  };
  onGlobalKey(onDuplicateKey);

  // Copying rows rides the browser's own copy command (Cmd/Ctrl+C, Edit >
  // Copy) so the `copy` event's DataTransfer can carry every clipboard
  // type. The selection to copy: null when the copy isn't ours — focus is
  // in an editable surface or an inert shell (native copy grabs the
  // user's text fragment), a window selection is highlighted (the user is
  // copying text, not rows), an overlay is open, or nothing is selected.
  // The guard reads the focused element, not the event target: a `copy`
  // event targets wherever the text caret sits, which can be the task
  // pane while a row holds focus, and the keydown net below must agree
  // with the event on whose copy this is.
  const rowCopyIds = (): string[] | null => {
    if (isOverlayOpen()) return null;
    const el = document.activeElement;
    if (
      el?.closest(
        'input, textarea, [contenteditable="true"], [data-shortcuts-inert]',
      )
    )
      return null;
    const winSel = window.getSelection();
    if (winSel && !winSel.isCollapsed && winSel.toString().length > 0)
      return null;
    const ids = actionSelection()?.getSelectedKeys().map(String) ?? [];
    return ids.length === 0 ? null : ids;
  };
  // Browsers enable the copy command only over a text selection unless
  // the page cancels `beforecopy`, which is how a row selection with no
  // highlighted text still gets a `copy` event.
  const onBeforeCopy = (e: Event) => {
    if (pendingCopyIds !== null || rowCopyIds() !== null) e.preventDefault();
  };
  // Set while `copyRows` runs the copy command itself: the ids to copy,
  // bypassing the guards (the intent is explicit) and telling the event
  // apart from a browser-initiated copy.
  let pendingCopyIds: string[] | null = null;
  let copyEventSeen = false;
  const onCopyEvent = (e: ClipboardEvent) => {
    const ids = pendingCopyIds ?? rowCopyIds();
    if (ids === null) return;
    copyEventSeen = true;
    e.preventDefault();
    console.log("[clip] copy via copy event", {
      ids,
      commanded: pendingCopyIds !== null,
    });
    copyBlock(ids, e.clipboardData);
  };
  document.addEventListener("beforecopy", onBeforeCopy);
  document.addEventListener("copy", onCopyEvent);
  onCleanup(() => {
    document.removeEventListener("beforecopy", onBeforeCopy);
    document.removeEventListener("copy", onCopyEvent);
  });
  // Copy a block of rows by running the copy command now, so the `copy`
  // event lands synchronously with every type. Cmd+C can't lean on the
  // browser's own command for this: Firefox delivers that one a task
  // later (a hop through its parent process), which raced an earlier
  // timer-based net and let a late async write flatten the event's richer
  // one to plain text. Needs user activation (a keystroke, a menu click),
  // which every caller has; where the command won't run or the event
  // never reaches us, the async API takes the standard two types.
  const copyRows = (ids: readonly string[]): void => {
    pendingCopyIds = [...ids];
    copyEventSeen = false;
    let ran = false;
    try {
      ran = document.execCommand("copy");
    } catch {
      ran = false;
    }
    pendingCopyIds = null;
    if (ran && copyEventSeen) return;
    console.log("[clip] copy command unavailable, async fallback", {
      ran,
      copyEventSeen,
    });
    copyBlock(ids);
  };
  // Cmd/Ctrl+C: take over the keystroke (so the browser's own, possibly
  // delayed, copy doesn't follow) and copy through `copyRows`.
  const onCopyKey = (e: KeyboardEvent) => {
    if (e.key !== "c" && e.key !== "C") return;
    if (!(e.metaKey || e.ctrlKey)) return;
    if (e.shiftKey || e.altKey) return;
    const ids = rowCopyIds();
    if (ids === null) return;
    e.preventDefault();
    copyRows(ids);
  };
  onGlobalKey(onCopyKey);

  // Cmd/Ctrl+Z (undo) and Cmd/Ctrl+Shift+Z (redo). Skipped when focus
  // is in an editable surface so the browser's native text-undo handles
  // mid-typing in inputs/textareas/contenteditable rows. Only swallows
  // the keystroke when the replica actually applied a step — otherwise
  // the OS / browser still gets a shot at it.
  const onUndoRedoKey = (e: KeyboardEvent) => {
    if (e.key !== "z" && e.key !== "Z") return;
    if (!(e.metaKey || e.ctrlKey)) return;
    if (e.altKey) return;
    const did = e.shiftKey ? docs.redo() : docs.undo();
    if (did) e.preventDefault();
  };
  onGlobalKey(onUndoRedoKey);

  // In the app's window the menu bar's Edit › Undo / Redo is the one
  // undo surface (the sidebar menu hides its rows). A click there, or
  // a ⌘Z the handler above let through, arrives here: a focused text
  // field keeps its own undo (what the browser would have done with
  // the keystroke); otherwise the doc's, with the overlay guard the
  // key path has. The items are enabled from the same two facts.
  if (isMacShell()) {
    const editableFocused = (): boolean =>
      !!document.activeElement?.closest('input, textarea, [contenteditable="true"]');
    setUndoRedoHandler((kind) => {
      if (editableFocused()) {
        document.execCommand(kind);
        return;
      }
      if (isOverlayOpen()) return;
      if (kind === "undo") docs.undo();
      else docs.redo();
    });
    const [editing, setEditing] = createSignal(editableFocused());
    const onFocusChange = () => setEditing(editableFocused());
    document.addEventListener("focusin", onFocusChange);
    document.addEventListener("focusout", onFocusChange);
    // The window's search bubble, as the sidebar's Find button.
    setFindHandler(() => setFindOpen(true));
    onCleanup(() => {
      setUndoRedoHandler(null);
      setFindHandler(null);
      document.removeEventListener("focusin", onFocusChange);
      document.removeEventListener("focusout", onFocusChange);
    });
    createEffect(() => {
      docs.version();
      const e = editing();
      postUndoState(e || docs.canUndo(), e || docs.canRedo());
    });
  }

  let dndHandle: DndImperative | null = null;
  let boardHandle: BoardImperative | null = null;
  let upcomingHandle: UpcomingImperative | null = null;
  // Restore keyboard focus after the detail dialog closes: the board (many
  // column listboxes), the agenda and the list view (one) each expose
  // their own handle; only one is mounted at a time.
  const restoreItemsFocus = () => {
    if (view().kind === "upcoming") upcomingHandle?.focus();
    else if (boardListId() !== null) boardHandle?.focusActive();
    else dndHandle?.focus();
  };
  // Every dialog (auth, settings, confirm, shortcuts, the calendar pickers
  // opened from the list) closes to the same place via overlay.ts.
  registerFocusHome(restoreItemsFocus);

  // Enter: open the topmost selected item in the detail dialog. The dialog
  // owns Enter while open (commits & closes, or in the side pane commits
  // & hands focus back here — a second Enter then re-enters the pane),
  // and onGlobalKey's overlay / editable-surface guards keep this from
  // firing there.
  const onOpenKey = (e: KeyboardEvent) => {
    if (e.key !== "Enter") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const sel = actionSelection();
    const top = sel?.getSelectionTop() ?? null;
    if (top === null) return;
    e.preventDefault();
    setOpenItemId(String(top));
  };
  onGlobalKey(onOpenKey);

  // ? opens the keyboard-shortcut cheat sheet (the sheet handles its own
  // `?` to close). `?` is Shift+/, so shift is expected; bail on the other
  // modifiers. onGlobalKey already skips it while typing or when another
  // overlay is open.
  const onHelpKey = (e: KeyboardEvent) => {
    if (e.key !== "?") return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    e.preventDefault();
    setShortcutsOpen(true);
  };
  onGlobalKey(onHelpKey);

  // Space: shortcut for the Add button. In the list view it starts an
  // inline draft below the topmost selection. In a board it opens the
  // new-item dialog (boards capture via the dialog, not an inline draft),
  // placing the new card just below the selected card in its column — or
  // at the top of the default column when nothing is selected.
  const onSpaceAdd = (e: KeyboardEvent) => {
    if (e.key !== " ") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    if (view().kind !== "list" && view().kind !== "focus") return;

    // Focus is never a board, so boardId is null there and we fall through
    // to the inline-draft path below.
    const boardId = boardListId();
    if (boardId !== null) {
      e.preventDefault();
      const vApp = viewApp();
      const top = actionSelection()?.getSelectionTop() ?? null;
      const anchor = top !== null ? vApp.getItem(String(top)) : undefined;
      if (anchor && isOpen(anchor)) {
        // Below the selected card, in the selected card's lane.
        const linear = vApp.state.listOpen[boardId] ?? [];
        const at = linear.indexOf(anchor.id);
        setNewItemTarget({
          listId: boardId,
          docId: vApp.docId,
          state: anchor.state,
          index: at >= 0 ? at + 1 : 0,
        });
      } else {
        // Nothing selected: top of the first visible open lane.
        setNewItemTarget({
          listId: boardId,
          docId: vApp.docId,
          state: defaultCaptureLane(boardId),
          index: 0,
        });
      }
      return;
    }

    if (draft() !== null) return;
    e.preventDefault();
    startDraft();
  };
  onGlobalKey(onSpaceAdd);

  // [ / ]: cycle through the nav views in top-to-bottom order — Focus, Home,
  // Done, Bin, then the user lists (Bin only earns a slot while it holds
  // items, matching its nav visibility). Wraps at both ends. From a view that
  // isn't in the sequence (e.g. an emptied Bin), ] enters at the top and
  // [ at the bottom, so the bracket pair always re-enters the set.
  const onBracketNavigate = (e: KeyboardEvent) => {
    if (e.key !== "[" && e.key !== "]") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    // Sidebar order: the fixed views (Bin only while it has a row), then
    // each doc's section (Home first): its Inbox, then its user lists.
    const seq: ViewKey[] = [
      { kind: "focus" },
      { kind: "upcoming" },
      { kind: "done" },
      ...(docs.binCount() > 0 ? [{ kind: "bin" } as ViewKey] : []),
      ...docs.docs().flatMap((d): ViewKey[] => [
        docs.listView(d.docId, "inbox"),
        ...activeListsOf(d.docId).map((l) => docs.listView(d.docId, l.id)),
      ]),
    ];
    const idx = seq.findIndex((s) => viewKey(s) === viewKey(view()));
    const step = e.key === "]" ? 1 : -1;
    const nextIdx =
      idx === -1
        ? step === 1
          ? 0
          : seq.length - 1
        : (idx + step + seq.length) % seq.length;
    e.preventDefault();
    setView(seq[nextIdx]!);
  };
  onGlobalKey(onBracketNavigate);

  // 1–4: jump straight to the fixed nav views in sidebar order (Focus /
  // Upcoming / Done / Inbox). Bin has no digit. Mapping and modifier guard
  // live in digitNavTarget (pure, unit-tested); onGlobalKey supplies the
  // overlay / editable-surface guards.
  const onDigitNavigate = (e: KeyboardEvent) => {
    const target = digitNavTarget(e);
    if (!target) return;
    e.preventDefault();
    setView(target);
  };
  onGlobalKey(onDigitNavigate);

  // Drag items into a list nav button to move them to that list as the
  // first items, onto Bin to bin them, or onto Focus to pin them into the
  // Focus lens. Discriminate from the nav's own list-reorder drag by
  // checking detail.items[0] for an item-shaped record (`listId` is
  // present on ItemView, absent on ListView). Bubbling + composed means a
  // single document-level listener catches both Dnd instances.
  type DropTarget =
    | { kind: "list"; el: HTMLElement; listId: string; docId: string }
    | { kind: "bin"; el: HTMLElement }
    | { kind: "focus"; el: HTMLElement };
  const findDropTarget = (x: number, y: number): DropTarget | null => {
    const el = document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>(
        "[data-drop-list-id], [data-drop-bin], [data-drop-focus]",
      );
    if (!el) return null;
    if (el.dataset.dropListId !== undefined) {
      return {
        kind: "list",
        el,
        listId: el.dataset.dropListId,
        docId: el.dataset.dropDocId ?? docs.homeDocId,
      };
    }
    if (el.dataset.dropFocus !== undefined) {
      return { kind: "focus", el };
    }
    return { kind: "bin", el };
  };
  const clearDropHighlight = () => {
    document
      .querySelectorAll<HTMLElement>("[data-drop-active]")
      .forEach((el) => delete el.dataset.dropActive);
  };
  // Classify via `firstItem` — `detail.items` is a lazy getter over the
  // whole dragged selection, and this runs on every pointermove.
  const isItemDrag = (detail: DndDragEventDetail): boolean => {
    const first = detail.firstItem;
    return typeof first === "object" && first !== null && "listId" in first;
  };

  const onDndDragMove = (e: Event) => {
    const ce = e as CustomEvent<DndDragEventDetail>;
    if (!isItemDrag(ce.detail)) return;
    clearDropHighlight();
    const target = findDropTarget(ce.detail.x, ce.detail.y);
    if (target) target.el.dataset.dropActive = "";
  };
  const onDndDragEnd = (e: Event) => {
    const ce = e as CustomEvent<DndDragEventDetail>;
    clearDropHighlight();
    if (!isItemDrag(ce.detail)) return;
    const target = findDropTarget(ce.detail.x, ce.detail.y);
    if (!target) return;
    ce.preventDefault();
    const draggedKeys = new Set(ce.detail.keys.map(String));
    // Sort by the source view's visible order so multi-select drops
    // preserve the user's visible ordering rather than landing in
    // selection order. Dragged rows always come from the active view.
    const idsInOrder = items()
      .map((it) => it.id)
      .filter((id) => draggedKeys.has(id));
    if (target.kind === "bin") {
      const toBin = idsInOrder.filter((id) => {
        const it = docs.getItem(id);
        return it !== undefined && !isBinned(it);
      });
      if (toBin.length === 0) return;
      routed.setBinnedMany(toBin, true);
      selection.clear();
      return;
    }
    if (target.kind === "focus") {
      // Same behaviour as the row context menu's "Add to focus": the core
      // skips any id that is already focused or not Open, so a mixed drop
      // resolves cleanly. Items keep their list — nothing moves.
      routed.addToFocusMany(idsInOrder);
      selection.clear();
      return;
    }
    // Relocation only, into whichever doc the target list is in (rows
    // of another doc cross over, confirmed first when that changes who
    // sees them): binned rows are restored into the target (the Bin is
    // outside the workspace, so "move while binned" has no meaning), but
    // Done rows stay Done — reopening is the lane drag / un-done key.
    const tApp = docs.appOf(target.docId);
    if (!tApp) return;
    const movable = idsInOrder.filter((id) => docs.getItem(id) !== undefined);
    if (movable.length === 0) return;
    confirmingCrossDoc(movable, tApp.docId, () => {
      relocateTo(movable, tApp, target.listId);
      // When dragging out of the current list, the rows are no longer
      // visible here — leaving them "selected" means a phantom block
      // anchor lingers. Same-list drops keep selection so the user can
      // continue acting on the rows they just rearranged.
      const v = view();
      const sameList =
        v.kind === "list" && v.id === target.listId && viewDocId() === target.docId;
      if (!sameList) selection.clear();
    });
  };
  document.addEventListener("primavera-dnd-dragmove", onDndDragMove);
  document.addEventListener("primavera-dnd-dragend", onDndDragEnd);
  onCleanup(() => {
    document.removeEventListener("primavera-dnd-dragmove", onDndDragMove);
    document.removeEventListener("primavera-dnd-dragend", onDndDragEnd);
  });

  // Jump to `target` and re-anchor the dnd selection on `id`. The
  // selection + scroll bounce is deferred past the view-change effect
  // (which clears selection) and past the keyed Dnd remount, so the
  // new controller's source has the row's index when scrollToKey lands.
  const revealItem = (id: string, target: ViewKey): void => {
    setView(target);
    // The agenda runs the same selection over its own rows and scrolls
    // through its own handle; a row the agenda doesn't show (an undated
    // task) just isn't selected.
    if (target.kind === "upcoming") {
      setTimeout(() => {
        selection.selectOnly(id);
        upcomingHandle?.scrollToKey(id);
      }, 0);
      return;
    }
    // If the destination list renders as a board, the list-view Dnd isn't
    // mounted — hand the id to the Board's reveal path (select + scroll in
    // the resolved column) instead of the list-view scroll below.
    if (target.kind === "list" && listViewIn(docs.viewDocId(target), target.id).board) {
      setBoardRevealIds([id]);
      return;
    }
    setTimeout(() => {
      selection.selectOnly(id);
      dndHandle?.scrollToKey(id);
    }, 0);
  };

  // After an undo / redo lands, put the user on what it changed. An item
  // the current view already shows (including a board's Done lane and a
  // lingering row) is selected in place, several at once when the step
  // touched several; otherwise the first surviving item is revealed in
  // its home view (Bin / Done / list). Ids that no longer resolve (an
  // undone add) are ignored, so an item-less step does nothing beyond
  // landing on a list the step touched, if that list still exists.
  const revealUndoOutcome = (outcome: UndoOutcome, docId: string): void => {
    const surviving = outcome.itemIds.filter((id) => docs.getItem(id));
    if (surviving.length === 0) {
      const listId = outcome.listIds.find((id) => docs.listOf(docId, id));
      if (listId === undefined) return;
      const target = docs.listView(docId, listId);
      if (viewKey(target) !== viewKey(view())) setView(target);
      return;
    }
    const visible = withBoardDone(new Set(items().map((it) => it.id)));
    const inView = surviving.filter((id) => visible.has(id));
    if (inView.length === 0) {
      revealItem(surviving[0], viewForItem(docs.getItem(surviving[0])!));
      return;
    }
    if (boardListId() !== null) {
      // The board reveals one lane at a time: keep the ids sharing the
      // first one's lane (a done card's lane is Done, not its state).
      const laneOf = (id: string): string => {
        const it = docs.getItem(id)!;
        return isClosed(it) && !isBinned(it) ? "done" : it.state;
      };
      const lane = laneOf(inView[0]);
      setBoardRevealIds(inView.filter((id) => laneOf(id) === lane));
      return;
    }
    // Same deferral as `revealItem`: past the store-driven re-render so
    // the dnd's source has the rows when the scroll lands.
    setTimeout(() => {
      selection.setSelectedKeys(inView);
      dndHandle?.scrollToKey(inView[0]);
    }, 0);
  };
  onCleanup(docs.onUndoRedo(revealUndoOutcome));

  // Selecting a palette result: jump to the view that contains it. Built-in
  // views (Focus / Upcoming / Done / Bin / Inbox) and lists go straight to
  // that view. Items
  // pick the view based on their lifecycle — binned items live in the Bin,
  // done-only items in Done, otherwise their list.
  const onFindSelect = (r: FindResult) => {
    if (r.kind === "view") {
      setView(r.id === "inbox" ? { kind: "list", id: "inbox" } : { kind: r.id });
      return;
    }
    if (r.kind === "list") {
      setView(docs.listView(r.docId ?? docs.homeDocId, r.id));
      return;
    }
    // An Open event's home is the calendar, which has no row to select
    // (and no day for a past one): open it there, as its link does.
    const picked = docs.getItem(r.id);
    if (picked && isEvent(picked) && isOpen(picked)) {
      openItemFromUrl(r.id);
      return;
    }
    revealItem(
      r.id,
      r.lifecycle === "binned"
        ? { kind: "bin" }
        : r.lifecycle === "done" || r.lifecycle === "cancelled"
          ? { kind: "done" }
          : docs.listView(r.docId ?? docs.homeDocId, r.listId || "inbox"),
    );
  };

  // Shared by both Find surfaces (desktop palette / the phone's menu page).
  const onFindOpenChange = (open: boolean) => {
    setFindOpen(open);
    if (open) return;
    // Hand keyboard focus back to the items listbox on close (Escape
    // or a pick) — the palette stole it into its search input and
    // the view's shortcuts are keyed off the listbox having focus.
    // A pick may also switch views, so defer past the <Show keyed>
    // remount like the nav's onSelect does.
    requestAnimationFrame(() => restoreItemsFocus());
  };
  const onFindPick = (r: FindResult) => {
    // A pick navigates the workspace behind any open item's page; close
    // the item so the result is actually visible. One batch, so the
    // address-bar effect sees the close and the destination together.
    batch(() => {
      setOpenItemId(null);
      onFindSelect(r);
    });
  };

  // Row context menu: hop an item between its home list and the Focus lens,
  // landing selected on the other side. "list" resolves the item's owning
  // list (falling back to the inbox if the id has gone stale); "focus" is
  // only offered for items that already carry a Focus ref, so the lens is
  // guaranteed to render it.
  const revealItemIn = (id: string, where: "list" | "focus"): void => {
    if (where === "focus") {
      revealItem(id, { kind: "focus" });
      return;
    }
    const it = docs.getItem(id);
    // An event's home is the calendar: go there with the item open.
    if (it && isEvent(it)) {
      batch(() => {
        if (view().kind !== "upcoming") setView({ kind: "upcoming" });
        setOpenItemId(id);
      });
      return;
    }
    const docId = it?.docId ?? docs.homeDocId;
    revealItem(id, docs.listView(docId, homeListId(docId, it?.listId)));
  };

  // ---------- Fragment URLs (`spec/urls.md`) ----------
  //
  // The address bar mirrors `(view, explicitly opened item)`; hash
  // navigation (a pasted link, Back / Forward, an internal note link)
  // applies the route back onto the same two signals. One effect derives
  // the hash from both, so a route that sets view + item together writes
  // the URL once, and a state change that already matches the hash (i.e.
  // one we just applied from the hash) writes nothing. The side pane
  // following the list selection is not an "open" as far as the URL is
  // concerned (it isn't `openItemId`, see `shownItemId`): the address
  // bar keeps the view token until the user actually enters the item
  // (Enter, row open, a link, a click into the pane).

  // The view that shows `it`: Bin if binned, Done if closed (done or
  // cancelled), else its home: the calendar for an event, its list
  // otherwise. A stale list id falls back to Inbox; archived lists still
  // render.
  const viewForItem = (it: ItemView): ViewKey =>
    isBinned(it)
      ? { kind: "bin" }
      : isClosed(it)
        ? { kind: "done" }
        : isEvent(it)
          ? { kind: "upcoming" }
          : docs.listView(it.docId, homeListId(it.docId, it.listId));

  // Reveal + open an item by id, in whichever doc holds it. False when no
  // open doc has it. `on: "upcoming"` (`#upcoming_<id>`) opens it over the
  // calendar whatever its home, as an Open event already does; only a
  // binned item still goes to the Bin, since the agenda never shows one.
  const openItemFromUrl = (id: string, on?: "upcoming"): boolean => {
    const it = docs.getItem(id);
    if (!it) return false;
    const target: ViewKey =
      on === "upcoming" && !isBinned(it) ? { kind: "upcoming" } : viewForItem(it);
    batch(() => {
      revealItem(id, target);
      setOpenItemId(id);
    });
    return true;
  };

  // Mobile shell: at narrow viewports the sidebar (and its footer chrome)
  // is replaced by MobileBars (MobileShell.tsx), and the Find palette by
  // FindSheet, the phone's menu page over the same result set. Declared
  // ahead of the router: the menu page is a route of its own there.
  const mobileMq = window.matchMedia("(max-width: 768px) and (pointer: coarse)");
  const [isMobile, setIsMobile] = createSignal(mobileMq.matches);
  const onMqChange = (e: MediaQueryListEvent) => {
    setIsMobile(e.matches);
  };
  mobileMq.addEventListener("change", onMqChange);
  onCleanup(() => mobileMq.removeEventListener("change", onMqChange));
  // The menu page is the phone's: `findOpen` on a phone is the page, on
  // desktop the palette, which is transient and never a route. A
  // viewport crossing closes whichever was up rather than carrying one
  // shell's surface into the other.
  const menuOpen = () => isMobile() && findOpen();
  createEffect(on(isMobile, () => setFindOpen(false), { defer: true }));

  // Applies a route; false when it names nothing this shell can show (the
  // menu page on desktop), so the caller rewrites the address bar.
  const applyRoute = (route: Route): boolean => {
    if (route.kind === "menu") {
      if (!isMobile()) return false;
      batch(() => {
        setOpenItemId(null);
        setFindOpen(true);
      });
      return true;
    }
    // A view or item route leaves the menu page (Back from it, or a link
    // tapped while it is up).
    if (isMobile()) setFindOpen(false);
    if (route.kind === "view") {
      // A list is looked up across every open doc (`spec/urls.md`): an
      // unknown one (deleted, or from another account) is ignored. The
      // reserved inbox has no ListMeta row; its URL names its doc when
      // that isn't Home, and an unknown doc is ignored the same way.
      let target = route.view;
      if (target.kind === "list") {
        if (target.id === "inbox") {
          if (target.docId !== undefined && !docs.appOf(target.docId)) return true;
        } else {
          const docId = docs.docIdOfList(target.id);
          if (docId === undefined) return true;
          target = docs.listView(docId, target.id);
        }
      }
      const sameView = viewKey(target) === viewKey(untrack(view));
      batch(() => {
        // Closing the entered item leaves the side pane on the selection
        // (`shownItemId`), so the pop the mirror effect issues for a
        // hand-back doesn't blank it.
        setOpenItemId(null);
        // Only switch when the route names a different view. Closing an
        // opened item pops the history entry it pushed, and that popstate
        // routes back to the view already on screen: a fresh `ViewKey`
        // object for the same view would still trip the view-change
        // effect (selection cleared, draft dropped), which is how
        // Enter-to-close in the list view used to lose the row's
        // selection. Board selections live inside Board and never saw it.
        if (!sameView) setView(target);
      });
      return true;
    }
    if (!openItemFromUrl(route.id, route.on)) setPendingItem({ id: route.id, on: route.on });
    return true;
  };

  // Boot: a hash on the URL overrides the prefs-restored view. Applied
  // synchronously, before the mirror effect below first runs, so the
  // effect's first pass sees state that already matches the hash and
  // doesn't clobber it with the restored view.
  {
    const route = parseHash(location.hash);
    if (route) applyRoute(route);
  }

  // Pending item link: retry whenever the store changes.
  createEffect(() => {
    const pending = pendingItem();
    if (pending === null) return;
    docs.version();
    if (docs.getItem(pending.id)) untrack(() => openItemFromUrl(pending.id, pending.on));
  });

  // State → address bar. History rules: a view change pushes; an open
  // pushes once (so Back closes the item) and is marked in
  // `history.state`; item → item replaces; closing an item we pushed for
  // goes Back instead of leaving a duplicate entry. In the side pane a
  // close is the pane handing focus back (Escape, Enter): it carries on
  // showing the selection, so an Enter / Escape cycle there nets zero
  // history. The first run (boot) always replaces.
  //
  // The phone's menu page is an entry of its own, marked the same way:
  // opening pushes it (over an entered item it takes that item's entry,
  // since the item closes), closing with nothing else changing goes
  // Back, and a pick from it (a view or an item) takes the entry over,
  // so a menu visit never leaves a stale `#menu` behind the destination.
  const ITEM_ENTRY = { monoplanItem: true };
  const MENU_ENTRY = { monoplanMenu: true };
  let prevUrlState: { viewKey: string; item: string | null; menu: boolean } | undefined;
  createEffect(() => {
    const v = view();
    const item = openItemId();
    const menu = menuOpen();
    const hash = stateHash(v, item, menu);
    const cur = { viewKey: viewKey(v), item, menu };
    const prev = prevUrlState;
    prevUrlState = cur;
    if (location.hash === hash) return;
    if (prev === undefined) {
      history.replaceState(null, "", hash);
      return;
    }
    // A context form of the open item (`#upcoming_<id>`, just applied
    // from the hash) is the same place: canonicalise the address in its
    // own entry rather than pushing the view change it carried.
    if (!menu && item !== null && namesItem(location.hash, item)) {
      history.replaceState(history.state, "", hash);
      return;
    }
    const viewChanged = prev.viewKey !== cur.viewKey;
    const opened = prev.item === null && item !== null;
    const closed = prev.item !== null && item === null;
    if (!prev.menu && menu) {
      if (closed && history.state?.monoplanItem === true) {
        history.replaceState(MENU_ENTRY, "", hash);
      } else {
        history.pushState(MENU_ENTRY, "", hash);
      }
      return;
    }
    if (prev.menu && !menu && history.state?.monoplanMenu === true) {
      if (!viewChanged && !opened) {
        // The popstate handler lands on the view beneath, already on
        // screen, and leaves it alone.
        history.back();
        return;
      }
      history.replaceState(opened ? ITEM_ENTRY : null, "", hash);
      return;
    }
    if (closed && !viewChanged && history.state?.monoplanItem === true) {
      // The popstate handler sees the view we're already on and leaves
      // it (and the selection) alone.
      history.back();
      return;
    }
    if (viewChanged) {
      history.pushState(null, "", hash);
    } else if (opened) {
      history.pushState(ITEM_ENTRY, "", hash);
    } else {
      // Keep the entry's marker so a later close still pops it.
      history.replaceState(history.state, "", hash);
    }
  });

  // Tab title: "Monoplan - <list>", or "Monoplan - <item>" while an item
  // is entered (same signal the address bar mirrors, so the side pane
  // following the selection leaves the title on the list). An item with
  // no text yet (a fresh draft) keeps the list's name. Restored to the
  // bare name when the workspace unmounts (logout back to the login
  // screen).
  createEffect(() => {
    const id = openItemId();
    const itemText = id === null ? "" : (docs.getItem(id)?.text ?? "").trim().split("\n")[0];
    const label = itemText || viewTitle(view(), allLists(), m());
    document.title = `Monoplan - ${label}`;
  });
  onCleanup(() => {
    document.title = "Monoplan";
  });

  // Address bar → state: Back / Forward, a hand-edited hash, an internal
  // link click. An unrecognised hash is rewritten to the current state.
  // A sharing link pasted into this tab's address bar is a fragment-only
  // navigation (no page load, so `index.tsx`'s capture never ran): park
  // its secret here, exactly as at boot, before the router can see it.
  // The capture strips the fragment, so the rewrite below restores the
  // view on screen and the dialog opens off the parked secret.
  const onPopState = () => {
    captureInviteFromLocation();
    const route = parseHash(location.hash);
    if (route && applyRoute(route)) return;
    history.replaceState(history.state, "", stateHash(view(), openItemId(), menuOpen()));
  };
  window.addEventListener("popstate", onPopState);
  onCleanup(() => window.removeEventListener("popstate", onPopState));

  // While a row is expanded: right-click inside it → native browser menu;
  // right-click anywhere else → noop. Capture-phase so we run before
  // Kobalte's ContextMenu trigger sees the event.
  const onContextMenu = (e: MouseEvent) => {
    const expanded = document.querySelector<HTMLElement>('.row[data-expanded=""]');
    if (!expanded) return;
    if (expanded.contains(e.target as Node)) {
      e.stopPropagation();
    } else {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  document.addEventListener("contextmenu", onContextMenu, true);
  onCleanup(() => document.removeEventListener("contextmenu", onContextMenu, true));

  // Narrow desktop: below this width the side panel's third column
  // squeezes the main surface into uselessness, so the task surface uses
  // its modal shell (a floating sheet over the content) instead. Widening
  // the window brings the panel straight back.
  const narrowMq = window.matchMedia("(max-width: 919px)");
  const [isNarrow, setIsNarrow] = createSignal(narrowMq.matches);
  const onNarrowChange = (e: MediaQueryListEvent) => setIsNarrow(e.matches);
  narrowMq.addEventListener("change", onNarrowChange);
  onCleanup(() => narrowMq.removeEventListener("change", onNarrowChange));

  // Desktop side panel (right-hand column). Not a preference: it is there
  // whenever the viewport has room for it, and the task surface's shell
  // follows the viewport alone (panel, floating sheet, mobile page). While
  // an item is open (or a capture is in progress) the panel hosts the
  // task surface; otherwise it just invites a click. Deadlines live on
  // the Upcoming view, never here.
  const sidePanelShown = () => !isMobile() && !isNarrow();
  // The panel's task host element, set by ref while the panel is mounted.
  // Gated on `sidePanelShown` so the dialog falls back to its modal shell
  // the moment the panel loses its room (the stale element is never
  // handed out).
  const [panelMount, setPanelMount] = createSignal<HTMLElement | null>(null);

  // Every mutation of the active selection (list view's, or the board's
  // active lane). `equals: false`: the selection object is mutated in
  // place, so each change must notify. Feeds `multiSelectIds` and the
  // side pane's followed item below.
  const [selectionTick, setSelectionTick] = createSignal<DndSelection | null>(
    null,
    { equals: false },
  );
  selection.onChange(setSelectionTick);
  createEffect(
    on(boardSelection, (sel) => {
      if (!sel) return;
      setSelectionTick(sel);
      onCleanup(sel.onChange(setSelectionTick));
    }),
  );
  // A multi-row selection: the visible ids of the active selection while
  // it spans more than one row, else null. The side panel shows its bulk
  // actions surface (`SelectionPanel`) for it in place of a single item.
  // Tracks `selectionTick` (every block mutation) plus the view's rows, so
  // it re-derives once rows the actions removed have left the list.
  const multiSelectIds = createMemo((): string[] | null => {
    const sel = selectionTick();
    if (!sel || sel !== actionSelection()) return null;
    const ids = selectedVisibleIds(true);
    return ids.length > 1 ? ids : null;
  });

  // Bulk actions for that selection, mirroring the row context menu's
  // multi-target entries (same labels, same shortcut hints) for the view
  // the rows are in: Bin rows restore / delete, everything else toggles
  // done / focus, re-dates, moves, copies, duplicates or bins. Each action
  // closes over the ids it was built for, so it still acts on what the
  // user saw even if the selection shifts underfoot.
  const multiSelectActions = createMemo((): SelectionAction[] => {
    const ids = multiSelectIds();
    if (!ids) return [];
    const msgs = m();
    const kind = view().kind;
    if (kind === "bin") {
      return [
        {
          label: msgs.common.restore,
          run: () => routed.setBinnedMany(ids, false),
        },
        { label: msgs.common.copy, shortcut: "⌘C", run: () => copyRows(ids) },
        {
          label: msgs.common.delete,
          shortcut: "⌫",
          run: () => binOrDeleteIds(ids),
        },
      ];
    }
    const openIds = ids.filter((id) => {
      const it = docs.getItem(id);
      return it !== undefined && isOpen(it);
    });
    const out: SelectionAction[] = [
      {
        label: allClosedIds(ids) ? msgs.workspace.markNotDone : msgs.workspace.markDone,
        shortcut: "X",
        run: () => toggleDoneIds(ids),
      },
      {
        label: allCancelledIds(ids) ? msgs.workspace.reopen : msgs.workspace.markCancelled,
        shortcut: "⇧X",
        run: () => toggleCancelledIds(ids),
      },
    ];
    // Focus membership: pin / unpin from a list or board (open rows only,
    // like the row menu's `canPinToFocus`); the Focus lens itself only
    // ever removes.
    if (kind === "list" && openIds.length > 0) {
      out.push({
        label: allFocusedIds(openIds) ? msgs.focus.remove : msgs.focus.add,
        shortcut: "S",
        run: () => toggleFocusIds(openIds),
      });
    } else if (kind === "focus") {
      out.push({
        label: msgs.focus.remove,
        shortcut: "S",
        run: () => routed.removeFromFocusMany(ids),
      });
    }
    // Deadlines only matter while an item is open (the row badge and menu
    // entry hide for done rows too). Single item only: the pick happens
    // in the item's own deadline popover, as the row menu's entry does.
    if (openIds.length === 1) {
      out.push({
        label: `${msgs.deadline.label}: ${msgs.deadline.setDate}`,
        run: () => openItemDeadline(openIds[0]!),
      });
    }
    out.push(
      { label: msgs.common.move, shortcut: "M", run: () => openMovePalette(ids) },
      { label: msgs.common.copy, shortcut: "⌘C", run: () => copyRows(ids) },
    );
    if (openIds.length > 0) {
      out.push({
        label: msgs.workspace.duplicate,
        shortcut: "⌘D",
        run: () => duplicateBlock(openIds),
      });
    }
    out.push({
      label: msgs.workspace.moveToBin,
      shortcut: "⌫",
      run: () => binOrDeleteIds(ids),
    });
    return out;
  });

  // The row the side pane follows: while the pane is showing and nothing
  // is being captured, the topmost selected item (list view, or the
  // board's active lane). Null for a multi-row selection (the pane shows
  // its bulk actions instead), an empty one, a draft row, or a key the
  // store no longer has. Modal / mobile shells never follow anything.
  const followedItemId = createMemo((): string | null => {
    if (!sidePanelShown() || newItemTarget() !== null) return null;
    const sel = selectionTick();
    if (!sel || sel !== actionSelection()) return null;
    if (multiSelectIds() !== null) return null;
    const top = sel.getSelectionTop();
    if (top === null) return null;
    const id = String(top);
    return docs.getItem(id) ? id : null;
  });

  // What the task surface shows: the entered item wherever it lives, else
  // (side pane only) the followed row. Entering is what focuses the
  // editor and what the URL follows; following is ambient and keeps the
  // keys on the list. The pane losing its room (a narrow window, the
  // mobile shell) drops the followed half on its own, so only an entered
  // item carries over to the modal / page shell;
  // the pane coming back picks the selection up again.
  const shownItemId = createMemo((): string | null => {
    const entered = openItemId();
    if (entered !== null) return entered;
    return followedItemId();
  });
  createEffect(() => {
    if (shownItemId() === null) setLiveEdit(null);
  });

  // An entered item follows the selection out: moving to a different row,
  // or growing to several, drops it, so the URL falls back to the view and
  // the pane shows the selection instead. An emptied selection is left
  // alone: a link reveal clears then reselects on its way in, and Escape
  // handles its own case below. Only while the pane is showing; the modal
  // and mobile page don't have a live selection under them.
  createEffect(
    on(selectionTick, (sel) => {
      if (!sel || !sidePanelShown() || sel !== actionSelection()) return;
      const entered = untrack(openItemId);
      if (entered === null) return;
      const top = sel.getSelectionTop();
      const movedAway = top !== null && String(top) !== entered;
      if (movedAway || multiSelectIds() !== null) setOpenItemId(null);
    }),
  );

  // Escape: the dnd (bound on its listbox, so it runs first) has already
  // cleared the selection, which takes the pane's followed row with it;
  // an entered item outlives the selection, so it is closed here. Only
  // when the selection really is empty afterwards — an Escape that merely
  // collapsed an inline edit leaves the row selected and the pane alone.
  // `isTrusted` skips the Row's synthetic Escapes (Enter-commit and blur
  // drive collapse through the dnd by dispatching their own), so clicking
  // out of an inline edit into the pane doesn't close it underfoot.
  // Modal / mobile shells take Escape themselves, so nothing to do there.
  const onEscapeKey = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || !e.isTrusted) return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    if (!sidePanelShown() || openItemId() === null) return;
    if (actionSelection()?.hasSelection()) return;
    setOpenItemId(null);
  };
  onGlobalKey(onEscapeKey);

  // The task surface works on one doc's `DocApp` (its notes bridge, undo
  // and list picker are per doc), so it is keyed on the doc of what it
  // shows: a capture's target doc, else the shown item's, else Home.
  // Crossing docs remounts it; the outgoing instance settles its buffers
  // on cleanup.
  const taskDocId = createMemo((): string => {
    const nw = newItemTarget();
    if (nw) return nw.docId;
    const id = shownItemId();
    return (id !== null ? docs.docIdOf(id) : undefined) ?? docs.homeDocId;
  });
  const taskApp = createMemo((): DocApp => docs.appOf(taskDocId()) ?? home);

  const navigateTo = (v: ViewKey) => {
      setView(v);
      // Move keyboard focus to the items listbox once Solid has
      // settled the new view — keyboard users land ready to arrow /
      // Enter-to-expand / Space-to-add, mouse users get the same
      // priming so a follow-up arrow key Just Works. rAF defers past
      // the <Show keyed> remount when the view's container changes.
      //
      // Skip the steal if the user has by now started editing
      // something — a double-click on a nav label fires two clicks
      // (queueing two rAFs) *then* dblclick → startEdit, which
      // focuses the contenteditable via a microtask. Microtasks
      // drain before the next rAF, so without this guard the
      // pending rAF would yank focus right back out of rename mode.
      requestAnimationFrame(() => {
        const ae = document.activeElement;
        if (
          ae instanceof HTMLElement &&
          (ae.isContentEditable ||
            ae.tagName === "INPUT" ||
            ae.tagName === "TEXTAREA")
        ) {
          return;
        }
        if (view().kind === "upcoming") upcomingHandle?.focus();
        else dndHandle?.focus();
      });
  };

  // Sidebar width: dragged from the nav's right edge, saved per device
  // (navWidth.ts). The drag tracks the pointer's travel from where it
  // went down, so the edge doesn't jump to the pointer on grab; the
  // width is written once, on release. Double-click restores the default.
  const [navWidth, setNavWidth] = createSignal(loadNavWidth());
  const [navResizing, setNavResizing] = createSignal(false);
  let navDrag: { pointerId: number; startX: number; startWidth: number } | null =
    null;
  const onNavResizeDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    // No text selection (or window drag in the macOS shell) underneath.
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    navDrag = { pointerId: e.pointerId, startX: e.clientX, startWidth: navWidth() };
    setNavResizing(true);
  };
  const onNavResizeMove = (e: PointerEvent) => {
    if (!navDrag || e.pointerId !== navDrag.pointerId) return;
    setNavWidth(clampNavWidth(navDrag.startWidth + e.clientX - navDrag.startX));
  };
  const onNavResizeEnd = (e: PointerEvent) => {
    if (!navDrag || e.pointerId !== navDrag.pointerId) return;
    navDrag = null;
    setNavResizing(false);
    saveNavWidth(navWidth());
  };
  const onNavResizeReset = () => {
    setNavWidth(NAV_WIDTH_DEFAULT);
    saveNavWidth(NAV_WIDTH_DEFAULT);
  };

  return (
    <div
      class="app"
      classList={{
        "side-panel-open": sidePanelShown(),
      }}
      style={{ "--nav-width": `${navWidth()}px` }}
    >
      {/* The macOS app's window has no title bar: a thin strip across the
          top of every pane drags it (the sidebar's own `.nav-titlebar`
          is the tall one under the traffic lights). The main surface and
          side panel are nudged down under it (`[data-shell="mac"]`). */}
      <Show when={isMacShell()}>
        <div class="app-titlebar" onMouseDown={onTitlebarMouseDown} />
      </Show>
      <Show when={!isMobile()}>
      <div
        class="nav-resizer"
        classList={{ "nav-resizer-active": navResizing() }}
        role="separator"
        aria-orientation="vertical"
        aria-valuemin={NAV_WIDTH_MIN}
        aria-valuemax={NAV_WIDTH_MAX}
        aria-valuenow={navWidth()}
        onPointerDown={onNavResizeDown}
        onPointerMove={onNavResizeMove}
        onPointerUp={onNavResizeEnd}
        onPointerCancel={onNavResizeEnd}
        onDblClick={onNavResizeReset}
      />
      <Nav
        docs={docs}
        binCount={docs.binCount()}
        focusCount={docs.focusItems().length}
        upcomingBadge={upcomingBadge()}
        showListCounts={home.state.settings.showListCounts}
        view={view()}
        setView={navigateTo}
        onDeleteDoc={canDeleteDocs() ? props.deleteDoc : null}
        onLeaveDoc={canDeleteDocs() ? props.leaveDoc : null}
        presence={props.presence}
        members={members}
        // No footer at all in the app's window (spec/mac-window-plan.md
        // "Settings"): the menu bar has Settings and Undo / Redo, the
        // account and search bubbles over the page have the account
        // menu and Find, and the tray shows the sync state; the
        // shortcuts dialog is not reachable there for now.
        footer={
          session.session().mirror ? undefined : (
            <>
              <NavMenu
                docs={docs}
                session={session.session()}
                logout={() => setLogoutConfirmOpen(true)}
                onOpenSettings={() => setSettingsOpen(true)}
                onOpenShortcuts={() => setShortcutsOpen(true)}
                onSession={session.swapSession}
                // One workspace per accountless install (`auth.md`); signup
                // is the gate to more.
                onNewDoc={canManageDocs() ? () => setNewDocOpen(true) : null}
              />
              <StatusSlot
                docs={docs}
                online={session.online()}
                lastSyncAt={session.lastSyncAt()}
                session={session.session()}
                onSession={session.swapSession}
              />
              <NavFindButton onClick={() => setFindOpen(true)} />
            </>
          )
        }
      />
      </Show>
      <Show
        when={isMobile()}
        fallback={
          <FindPalette
            docs={docs}
            open={findOpen()}
            onOpenChange={onFindOpenChange}
            onSelect={onFindPick}
          />
        }
      >
        <FindSheet
          docs={docs}
          open={findOpen()}
          view={view()}
          onOpenChange={onFindOpenChange}
          onSelect={onFindPick}
          onOpenSettings={() => setSettingsOpen(true)}
          status={
            /* Mobile: the sync indicator sits in the Find page's top
               line (there's no sidebar footer, and the pill doesn't carry
               it). It mounts with the page, so the first-run auth prompt
               is owned by the FirstRunAuthPrompt beside MobileBars. */
            <StatusSlot
              class="find-sheet__status"
              docs={docs}
              online={session.online()}
              lastSyncAt={session.lastSyncAt()}
              session={session.session()}
              onSession={session.swapSession}
              autoPrompt={false}
            />
          }
          focusCount={docs.focusItems().length}
          upcomingBadge={upcomingBadge()}
          binCount={docs.binCount()}
          openCountsByList={openCountsByList()}
          showListCounts={home.state.settings.showListCounts}
        />
      </Show>
      <MovePalette
        open={moveIds() !== null}
        onOpenChange={(open) => {
          if (open) return;
          setMoveIds(null);
          // Hand keyboard focus back to the items listbox — the palette
          // stole it into its filter input. Same restore every dialog
          // does on close.
          focusItems();
        }}
        options={moveListOptions}
        isCurrent={moveIsCurrent}
        count={() => moveIds()?.length ?? 0}
        onPick={(opt) => {
          const ids = moveIds();
          if (ids) moveBlockToList(ids, opt.docId ?? moveDocId(), opt.id);
        }}
      />
      <ConfirmDialog
        open={crossMoveOpen()}
        onOpenChange={setCrossMoveOpen}
        title={crossMove()?.title ?? ""}
        message={crossMove()?.message ?? ""}
        confirmLabel={m().common.move}
        onConfirm={() => crossMove()?.run()}
      />
      {/* Not in the app's window (spec/mac-window-plan.md "Settings"):
          the app's Settings window has every pane; the dialog is not
          reachable there and keeps no conditions for it. */}
      <Show when={!session.session().mirror}>
      <Settings
        open={settingsOpen()}
        onOpenChange={setSettingsOpen}
        themePref={themePref()}
        onThemeChange={(pref) => {
          setThemePref(pref);
          theme.set(pref);
        }}
        showListCounts={home.state.settings.showListCounts}
        onShowListCountsChange={(show) => home.setShowListCounts(show)}
        session={session.session()}
        logout={() => setLogoutConfirmOpen(true)}
        onAccountDeleted={() =>
          // The server has already forgotten the account and every
          // device, so this is the local half of sign-out only.
          void session.logout({ wipeLocal: true, serverGone: true })
        }
        docs={docs}
      />
      </Show>
      {/* Sign-out confirmation, same wording as the macOS app's alert:
          the local copy goes, unsynced changes with it, synced data
          comes back on the next sign-in. Confirming is what wipes. */}
      <ConfirmDialog
        open={logoutConfirmOpen()}
        onOpenChange={setLogoutConfirmOpen}
        title={m().nav.logOutTitle}
        message={m().nav.logOutConfirm}
        confirmLabel={m().nav.logOut}
        onConfirm={() => void session.logout({ wipeLocal: true })}
      />
      <NewDocDialog
        open={newDocOpen()}
        onOpenChange={setNewDocOpen}
        onCreate={props.createDoc}
        onCreated={(docId) => navigateTo(docs.listView(docId, "inbox"))}
      />
      <InviteDialog
        joinDoc={props.joinDoc}
        onJoined={(docId) => navigateTo(docs.listView(docId, "inbox"))}
      />
      <ConfirmDialog
        open={emptyBinConfirmOpen()}
        onOpenChange={setEmptyBinConfirmOpen}
        title={m().workspace.emptyBinTitle}
        message={m().workspace.emptyBinConfirm}
        confirmLabel={m().workspace.emptyBin}
        onConfirm={() => docs.emptyBin()}
      />
      {archive.dialog()}
      <Show keyed when={taskApp()}>
        {(tApp) => (
      <TaskDialog
        creatorOf={creatorOf}
        itemId={shownItemId}
        setItemId={setOpenItemId}
        newItem={newItemTarget}
        setNewItem={(v) =>
          // A re-target stays in the capture's doc unless the picker
          // named a list of another workspace (`v.docId`), which
          // remounts the surface on that doc.
          setNewItemTarget(v ? { ...v, docId: v.docId ?? untrack(taskDocId) } : null)
        }
        app={tApp}
        lists={() => activeListsOf(tApp.docId)}
        otherLists={() => otherListOptions(tApp.docId)}
        onMoveAcross={(id, docId, listId) => {
          // The picker's own-doc move appends; so does this one. The
          // open item stays the open item: the surface follows it into
          // its new doc (keyed on `taskApp`).
          const target = docs.appOf(docId);
          if (!target) return;
          confirmingCrossDoc([id], docId, () => {
            const at = target.state.listOpen[listId]?.length ?? 0;
            if (docs.moveToDoc([id], docId, listId, at).length === 0) return;
            const it = target.getItem(id);
            if (listId === LIST_EVENTS && it && isOpen(it) && !it.when) openItemWhen(id);
          });
        }}
        // Only a shared workspace is named in the list picker: Personal
        // is the default and goes unsaid, as on the agenda.
        docLabel={() =>
          tApp.docId === docs.homeDocId ? null : docLabel(tApp.docId)
        }
        // Known places are the account's, not the item's doc's.
        knownPlaces={(limit) => docs.placeSuggestions("", limit)}
        // Focus is Home's regardless of the item's doc: a shared doc's
        // item is pinned as a foreign ref, and the star reads Home's
        // entries so it lights for both. Written through Home directly
        // rather than `routed`, which cannot place an id the store has
        // not seen yet (a capture pinned inside its creating batch).
        focus={{
          isFocused: docs.isFocused,
          add: (id) =>
            tApp === home
              ? home.addToFocus(id)
              : home.addForeignToFocus(tApp.docId, id, 0),
          remove: (id) => home.removeFromFocus(id),
        }}
        entered={() => openItemId() !== null}
        whenOpen={itemWhenOpen}
        setWhenOpen={setItemWhenOpen}
        deadlineOpen={itemDeadlineOpen}
        setDeadlineOpen={setItemDeadlineOpen}
        onClosed={restoreItemsFocus}
        onReleaseFocus={() => {
          // Handing focus back closes the entered item; the pane carries
          // on showing it as the followed row, and the address bar drops
          // back to the view as it would for the modal (`spec/urls.md`).
          setOpenItemId(null);
          restoreItemsFocus();
        }}
        onFocused={() => {
          const id = shownItemId();
          if (id !== null) setOpenItemId(id);
        }}
        draftText={draftText}
        captureRef={(h) => {
          // The surface remounts per doc (keyed on `taskApp`): the outgoing
          // instance nulls this on dispose before the incoming one mounts.
          captureHandle = h;
        }}
        onLiveText={(text) => {
          const id = shownItemId();
          if (id) setLiveEdit({ id, text });
        }}
        onCreated={(id) => {
          // Board view has a card to reveal. In a list or Focus view the
          // only capture is a draft's: one left unnamed for the pane and
          // committed there lands the selection on its item once the
          // store has it, as the row path does; a row-committed one was
          // selected by the row path, which clears the target first.
          const v = view();
          if (boardListId() !== null) setBoardRevealIds([id]);
          else if (v.kind === "list" || v.kind === "focus") {
            queueMicrotask(() => {
              if (docs.getItem(id)) selection.selectOnly(id);
            });
          }
        }}
        onReveal={revealItemIn}
        panelMount={() => (sidePanelShown() ? panelMount() : null)}
      />
        )}
      </Show>
      <ShortcutsDialog
        open={shortcutsOpen()}
        onOpenChange={setShortcutsOpen}
      />
      <div class="content">
      <main class="main" tabIndex={-1}>
        <header class="main-header">
          {/* Title group at the left edge; .main-header's space-between
              keeps the action buttons on the right regardless of group
              width. */}
          <div class="main-header-title">
            {/* User-created lists carry a display icon (chosen emoji or
                the default glyph). Reserved `inbox` has no `ListMeta` row,
                so it can't store one — gated out. */}
            <Show when={editableListId() !== null}>
              <ListIconPicker
                icon={viewApp().state.listsById[editableListId() ?? ""]?.icon}
                onPick={(icon) => viewApp().setListIcon(editableListId() ?? "", icon)}
                onClear={() => viewApp().setListIcon(editableListId() ?? "", "")}
              />
            </Show>
            {/* Reserved views (Focus, Home/Inbox, Done, Bin) can't store a
                custom icon, so mirror their fixed navbar glyph here as a
                static, non-interactive counterpart to the list icon. */}
            <Show when={viewIcon(view())} keyed>
              {(svg) => <span class="list-icon-static" innerHTML={svg} />}
            </Show>
            <h1>
            {/* A list (Inbox included) lives in one workspace; prefix its
                name with the workspace's so the title reads
                "Household › Groceries" and the same list name across two
                docs stays distinguishable. Muted so the list name leads.
                Home's lists carry no prefix: Personal is the default and
                goes unsaid. The lenses span docs, so they carry none
                either. */}
            <Show when={view().kind === "list" && viewDocId() !== docs.homeDocId}>
              <span class="title-doc-prefix">
                {docLabel(viewDocId())}
                <span
                  class="title-doc-sep"
                  aria-hidden="true"
                  innerHTML={caretRightSvg}
                />
              </span>
            </Show>
            <Show
              keyed
              when={editableListId()}
              fallback={viewTitle(view(), allLists(), m())}
            >
              {(listId) => (
                <EditableNavLabel
                  class="editable-title"
                  name={allLists().find((l) => l.id === listId)?.name ?? listId}
                  onSave={(name) => viewApp().renameList(listId, name)}
                />
              )}
            </Show>
          </h1>
            {/* Open-item count beside a list title: the same number the
                nav shows per list, hidden at zero so an empty list reads
                clean. */}
            <Show when={headerOpenCount() > 0}>
              <span class="main-header-count">{headerOpenCount()}</span>
            </Show>
            <Show when={archivedViewListId()}>
              <span class="badge archived-badge">{m().nav.archived}</span>
            </Show>
          </div>
          <div class="main-header-actions">
            <Show
              when={
                view().kind === "bin" &&
                items().length > 0
              }
            >
              <button
                type="button"
                class="add-button"
                tabIndex={-1}
                onClick={() => setEmptyBinConfirmOpen(true)}
              >
                <span class="add-button-icon" innerHTML={trashSvg} />
                <span>{m().workspace.emptyBin}</span>
              </button>
            </Show>
            {/* The view's icon buttons (list menu, display options, add)
                share one group. Hidden via CSS when the view contributes
                none; on mobile the group is a single glass bubble. */}
            <div class={isMobile() ? "header-group glass" : "header-group"}>
            {/* List actions menu: the same Archive / Unarchive the nav's
                context menu offers, kept here so restoring an archived
                list stays discoverable from the view header too. Only
                user lists — `inbox` can't be archived. */}
            <Show when={editableListId()}>
              {(listId) => (
                <DropdownMenu>
                  <DropdownMenu.Trigger
                    class="icon-button"
                    tabIndex={-1}
                    aria-label={m().common.menu}
                    innerHTML={dotsHorizontalSvg}
                  />
                  <DropdownMenu.Portal>
                    <DropdownMenu.Content class="dropdown-menu-content">
                      <DropdownMenu.Item
                        class="dropdown-menu-item"
                        onSelect={() => {
                          if (archivedViewListId() !== null) {
                            viewApp().setListArchived(listId(), false);
                          } else {
                            archive.request({ app: viewApp(), listId: listId() });
                          }
                        }}
                      >
                        {archivedViewListId() !== null
                          ? m().nav.unarchiveList
                          : m().nav.archiveList}
                      </DropdownMenu.Item>
                    </DropdownMenu.Content>
                  </DropdownMenu.Portal>
                </DropdownMenu>
              )}
            </Show>
            <Show when={view().kind === "list"}>
              <DisplayOptionsPopover>
                <SegmentedControl
                  class="theme-segmented"
                  aria-label={m().board.viewMode}
                  value={boardListId() !== null ? "board" : "list"}
                  onChange={(value) => {
                    const v = view();
                    if (v.kind !== "list") return;
                    const wantBoard = value === "board";
                    if (wantBoard !== (boardListId() !== null)) {
                      toggleBoard(v.id);
                    }
                  }}
                >
                  <SegmentedControl.Indicator class="theme-segment-indicator" />
                  <SegmentedControl.Item value="list" class="theme-segment">
                    <SegmentedControl.ItemInput />
                    <SegmentedControl.ItemControl class="theme-segment-control">
                      <SegmentedControl.ItemLabel class="view-mode-label">
                        <span
                          class="view-mode-icon"
                          aria-hidden="true"
                          innerHTML={listBulletSvg}
                        />
                        <span>{m().board.list}</span>
                      </SegmentedControl.ItemLabel>
                    </SegmentedControl.ItemControl>
                  </SegmentedControl.Item>
                  <SegmentedControl.Item value="board" class="theme-segment">
                    <SegmentedControl.ItemInput />
                    <SegmentedControl.ItemControl class="theme-segment-control">
                      <SegmentedControl.ItemLabel class="view-mode-label">
                        <span
                          class="view-mode-icon"
                          aria-hidden="true"
                          innerHTML={cardStackSvg}
                        />
                        <span>{m().board.board}</span>
                      </SegmentedControl.ItemLabel>
                    </SegmentedControl.ItemControl>
                  </SegmentedControl.Item>
                </SegmentedControl>
                {/* Flat list only: badge rows with their lifecycle state.
                    The board shows state as the lane, so the switch
                    yields to the lane rows there. */}
                <Show when={boardListId() === null && currentListId()}>
                  {(listId) => (
                    <Switch
                      class="done-switch"
                      checked={showState(listId())}
                      onChange={(checked) => setShowState(listId(), checked)}
                    >
                      <Switch.Label class="done-switch-label">
                        {m().workspace.showState}
                      </Switch.Label>
                      <Switch.Input class="done-switch-input" />
                      <Switch.Control class="done-switch-control">
                        <Switch.Thumb class="done-switch-thumb" />
                      </Switch.Control>
                    </Switch>
                  )}
                </Show>
                <Show when={boardListId()}>
                  {(listId) => (
                    <>
                      {/* Lane visibility (display-only — spec/board.md
                          "Lane visibility"), part of the view spec that
                          "Save as default" publishes. At least one lane
                          always stays on; the setters refuse the hide
                          that would blank the board. */}
                      <For each={OPEN_STATES}>
                        {(lane) => (
                          <LaneRow
                            name={laneLabel(m(), lane)}
                            count={boardLaneCounts()[lane]}
                            visible={visibleOpenLanes(listId()).includes(lane)}
                            onToggle={(show) => setLaneVisible(listId(), lane, show)}
                          />
                        )}
                      </For>
                      <LaneRow
                        name={m().board.showDoneColumn}
                        count={boardLaneCounts().done}
                        visible={showDoneColumn(listId())}
                        onToggle={(show) => setShowDoneColumn(listId(), show)}
                      />
                    </>
                  )}
                </Show>
                {/* Restore defaults drops this client's display choices
                    for the list (disabled when there are none). Below
                    it, publish this view as the list's default for
                    every device. Disabled once it already is the
                    default — the label then reads as a state, not an
                    action. */}
                <Show when={currentListId()}>
                  {(listId) => (
                    <>
                    <button
                      type="button"
                      class="view-default-button"
                      disabled={!hasLocalDisplay(listId())}
                      onClick={() => restoreViewDefaults(listId())}
                    >
                      {m().board.restoreDefaults}
                    </button>
                    <button
                      type="button"
                      class="view-default-button"
                      disabled={isSavedDefault(listId())}
                      onClick={() => saveViewAsDefault(listId())}
                    >
                      {isSavedDefault(listId())
                        ? m().board.savedAsDefault
                        : m().board.saveAsDefault}
                    </button>
                    </>
                  )}
                </Show>
              </DisplayOptionsPopover>
            </Show>
            {/* The Done view always badges each item with its origin list
                (like Focus). Its header control is a filter over which
                workspaces and lists the closed items come from
                (`doneFilter.ts`): device-local, both facets empty by
                default, which is every open doc. */}
            <Show when={view().kind === "done"}>
              <DoneFilterMenu
                workspaces={docs.docs().map((d) => ({
                  docId: d.docId,
                  label: docLabel(d.docId),
                  shared: !d.home,
                }))}
                lists={doneOfferedLists()}
                docChecked={(id) => doneFilterNow().docs.includes(id)}
                toggleDoc={(id) =>
                  updateDoneFilter((f) => ({ ...f, docs: toggled(f.docs, id) }))
                }
                listChecked={(key) => doneFilterNow().lists.includes(key)}
                toggleList={(key) =>
                  updateDoneFilter((f) => ({ ...f, lists: toggled(f.lists, key) }))
                }
                docCount={doneFilterNow().docs.length}
                listCount={doneFilterNow().lists.length}
                clear={() => updateDoneFilter(() => EMPTY_DONE_FILTER)}
              />
              <Tooltip openDelay={200} closeDelay={0} placement="bottom">
                <Tooltip.Trigger
                  as="button"
                  type="button"
                  class="icon-button header-add-button"
                  tabIndex={-1}
                  aria-label={m().workspace.log}
                  onClick={() =>
                    setNewItemTarget({
                      listId: "inbox",
                      docId: docs.homeDocId,
                      state: "backlog",
                      done: true,
                    })
                  }
                >
                  <span class="add-button-icon" innerHTML={plusSvg} />
                </Tooltip.Trigger>
                <Tooltip.Portal>
                  <Tooltip.Content class="tooltip-content">
                    {m().workspace.log}
                    <Tooltip.Arrow />
                  </Tooltip.Content>
                </Tooltip.Portal>
              </Tooltip>
            </Show>
            {/* The calendar merges every doc; each can be hidden here. A
                synced personal pref on the Home doc
                (`spec/sharing-plan.md`). Only offered once there is more
                than one doc. */}
            <Show when={view().kind === "upcoming" && docs.docs().length > 1}>
              <DisplayOptionsPopover>
                <div class="lane-row lane-row-heading">
                  <span class="lane-row-name">{m().upcoming.docs}</span>
                </div>
                <For each={docs.docs()}>
                  {(d) => (
                    <Switch
                      class="done-switch"
                      checked={!docs.calendarHiddenDocs().includes(d.docId)}
                      onChange={(checked) =>
                        home.setCalendarDocHidden(d.docId, !checked)
                      }
                    >
                      <Switch.Label class="done-switch-label">
                        {docLabel(d.docId)}
                      </Switch.Label>
                      <Switch.Input class="done-switch-input" />
                      <Switch.Control class="done-switch-control">
                        <Switch.Thumb class="done-switch-thumb" />
                      </Switch.Control>
                    </Switch>
                  )}
                </For>
              </DisplayOptionsPopover>
            </Show>
            {/* Add on the list, focus and upcoming views. The calendar is
                the home of events (`spec/events-plan.md`) but not a
                listbox to draft into, so it captures a new event through
                the dialog, on today so it is never out of sight; the
                dialog's list picker re-files it as a task. */}
            <Show
              when={
                view().kind === "list" ||
                view().kind === "focus" ||
                view().kind === "upcoming"
              }
            >
              <Tooltip openDelay={200} closeDelay={0} placement="bottom">
              <Tooltip.Trigger
                as="button"
                type="button"
                class="icon-button header-add-button"
                tabIndex={-1}
                onClick={(e) => {
                  // The dnd controller has a document-level click listener
                  // that collapses any expansion when a click lands outside
                  // the expanded row. The Add button is outside the dnd, so
                  // this same click would immediately collapse the draft we
                  // just opened. stopImmediatePropagation halts further
                  // document-level listeners (Solid's delegate runs first
                  // since it registers eagerly during render; the dnd's
                  // listener registers later in onMount).
                  e.stopImmediatePropagation();
                  const boardId = boardListId();
                  if (view().kind === "upcoming") {
                    setNewItemTarget(newEventTarget());
                  } else if (boardId !== null) {
                    // Board view has no inline draft flow; capture a new item
                    // into the first visible open lane, mirroring that
                    // lane's own "+".
                    setNewItemTarget({
                      listId: boardId,
                      docId: viewDocId(),
                      state: defaultCaptureLane(boardId),
                    });
                  } else {
                    startDraft();
                  }
                }}
                disabled={boardListId() === null && draft() !== null}
                aria-label={m().common.add}
              >
                <span class="add-button-icon" innerHTML={plusSvg} />
              </Tooltip.Trigger>
              <Tooltip.Portal>
                <Tooltip.Content class="tooltip-content">
                  {m().common.add}
                  <Tooltip.Arrow />
                </Tooltip.Content>
              </Tooltip.Portal>
              </Tooltip>
            </Show>
            </div>
          </div>
        </header>
        <Show
          when={view().kind === "upcoming"}
          fallback={
        <Show
          keyed
          when={boardKey()}
          fallback={
            <Show
              when={dndItems().length > 0 || showTrailer()}
              fallback={
                <div class="dnd-host empty">{m().workspace.emptyState}</div>
              }
            >
              <Show keyed when={dndRevision()}>
                <Dnd
                  class="dnd-host"
                  ref={(h) => (dndHandle = h)}
                  items={dndItems()}
                  setItems={setDndItems}
                  getKey={(it) => it.id}
                  selection={selection}
                  expandedKey={expandedKey()}
                  onExpandedChange={(k) =>
                    setExpandedKey(k == null ? null : String(k))
                  }
                  itemHeight={rowHeight(itemsIsMobile())}
                  expandable
                  clearOnBlankClick
                  fillHeight
                  autofocus
                  reorder={view().kind === "list" || view().kind === "focus"}
                  onReorder={onReorder}
                  onTrailerNavigate={() => startDraft(items().length)}
                  trailer={
                    showTrailer() ? (
                      <div
                        class="row row-new"
                        role="button"
                        tabIndex={-1}
                        aria-label={m().workspace.newItemPlaceholder}
                        on:click={(e) => {
                          // Native + stopPropagation: the listbox's own
                          // click handler would treat this as a blank
                          // click, and the controller's document-level
                          // click-outside would collapse the draft this
                          // click just opened (its row lands under the
                          // same point, but don't rely on that).
                          e.stopPropagation();
                          startDraft(items().length);
                        }}
                      >
                        <span class="row-new-check" aria-hidden="true" innerHTML={plusSvg} />
                        <div class="row-body">
                          <span class="row-text row-new-text">
                            {m().workspace.newItemPlaceholder}
                          </span>
                        </div>
                        {/* Reserves the row menu's slot (`.row-menu`) so the
                            text column lines up with the real rows above. */}
                        <span class="row-menu row-menu-spacer" aria-hidden="true" />
                      </div>
                    ) : undefined
                  }
                >
                  {(item, expanded) => {
                    // Overlay the dialog's in-progress title onto its row so the
                    // list mirrors the edit live. Only the edited row's object is
                    // swapped (others pass through by reference); the list is
                    // virtualized, so this recomputes for visible rows only.
                    const shownItem = createMemo(() => {
                      const ov = liveEdit();
                      const it = item();
                      return ov && ov.id === it.id ? { ...it, text: ov.text } : it;
                    });
                    return (
                      <Row
                        item={shownItem}
                        expanded={expanded}
                        app={routed}
                        selection={selection}
                        viewKind={view().kind}
                        showList={() => {
                          const k = view().kind;
                          return k === "focus" || k === "done" || k === "bin";
                        }}
                        listLabel={listLabel}
                        isSharedDoc={(docId) => docId !== docs.homeDocId}
                        showState={() => {
                          const v = view();
                          return v.kind === "list" && showState(v.id);
                        }}
                        duplicateBlock={duplicateBlock}
                        copyBlock={copyRows}
                        onDraftSettle={settleDraft}
                        onDraftInput={setDraftText}
                        holdEmptyDraft={() => items().length === 0}
                        onOpen={(id) => setOpenItemId(id)}
                        onReveal={revealItemIn}
                        onMoveToList={openMovePalette}
                        openOnTap={itemsIsMobile}
                      />
                    );
                  }}
                </Dnd>
              </Show>
            </Show>
          }
        >
          {(_key) => (
            <Board
              app={viewApp()}
              rowApp={routed}
              listId={boardListId()!}
              onOpen={(id) => setOpenItemId(id)}
              onReveal={revealItemIn}
              onMoveToList={openMovePalette}
              openOnTap={itemsIsMobile}
              duplicateBlock={duplicateBlock}
              copyBlock={copyRows}
              onAddItem={(listId, state, done) =>
                setNewItemTarget({ listId, docId: viewDocId(), state, done })
              }
              revealIds={boardRevealIds}
              clearReveal={() => setBoardRevealIds(null)}
              onActiveSelectionChange={setBoardSelection}
              showDoneColumn={() => showDoneColumn(boardListId()!)}
              visibleOpenLanes={() => visibleOpenLanes(boardListId()!)}
              ref={(h) => (boardHandle = h)}
            />
          )}
        </Show>
          }
        >
          <Upcoming
            app={routed}
            groups={upcomingGroups()}
            selection={selection}
            listLabel={listLabel}
            workspaceLabel={(docId) => (docId === docs.homeDocId ? null : docLabel(docId))}
            onOpen={(id) => setOpenItemId(id)}
            openOnTap={itemsIsMobile}
            ref={(h) => (upcomingHandle = h)}
            duplicateBlock={duplicateBlock}
            copyBlock={copyRows}
            onReveal={revealItemIn}
            onMoveToList={openMovePalette}
          />
        </Show>
      </main>
      </div>
      <Show when={sidePanelShown()}>
        <aside
          class="side-panel"
          aria-label={m().sidePanel.title}
          ref={(el) => {
            // Capture phase (see `onPanePointerDown`); Solid's `on:` is
            // bubble-only. The listener goes with the element, which the
            // enclosing Show remounts with the pane.
            el.addEventListener("pointerdown", onPanePointerDown, true);
          }}
        >
          {/* Blank state: nothing while a single row (or no row) is
              selected and no item is open. A multi-row selection shows
              its count where the task surface's header would sit, and
              the bulk actions stand in for the surface until the
              selection is back to one row (or an explicit Enter opens
              the topmost). */}
          <Show when={shownItemId() === null && newItemTarget() === null}>
            <Show when={multiSelectIds()}>
              {(ids) => (
                <>
                  <header class="side-panel-blank">
                    <h2 class="selection-panel-title">
                      {m().sidePanel.selectedCount(ids().length)}
                    </h2>
                  </header>
                  <SelectionPanel
                    actions={multiSelectActions()}
                    onClear={() => actionSelection()?.clear()}
                  />
                </>
              )}
            </Show>
          </Show>
          {/* The task surface portals in here while an item is open (see
              TaskDialog's `panelMount`). The div stays mounted so the
              portal target is stable; CSS hides it while empty, and the
              column sits blank rather than showing a hint. */}
          <div class="side-panel-task" ref={setPanelMount} />
        </aside>
      </Show>
      <Show when={isMobile()}>
        <MobileBars
          view={view()}
          inboxCount={home.state.listOpen["inbox"]?.length ?? 0}
          upcomingBadge={upcomingBadge()}
          setView={(v) => {
            // The pills stay live over an open item's page and over the
            // menu page; close both so the destination view is visible.
            // One batch: the address-bar effect then sees one transition
            // (the menu's entry handed to the destination) rather than a
            // close and a push.
            batch(() => {
              setOpenItemId(null);
              if (findOpen()) onFindOpenChange(false);
              navigateTo(v);
            });
          }}
          onMenu={() => {
            // The menu is a page: opening it leaves an open item's page
            // (one page at a time), and the item's history entry becomes
            // the menu's. Tapping its pill while on it stays put, as the
            // view pills do; the way out is a pick, another pill, or Back.
            if (findOpen()) return;
            batch(() => {
              setOpenItemId(null);
              onFindOpenChange(true);
            });
          }}
          onAdd={
            menuOpen()
              ? null
              : view().kind === "upcoming"
                ? () => {
                    // No listbox to draft into on the calendar: capture a
                    // new event through the dialog, as the header's Add
                    // does on desktop.
                    setOpenItemId(null);
                    setNewItemTarget(newEventTarget());
                  }
                : (view().kind === "list" || view().kind === "focus") &&
                    boardListId() === null
                  ? () => {
                      // The pills stay live over an open item's page; Add
                      // captures into the list behind it, so close it
                      // first.
                      setOpenItemId(null);
                      startDraft();
                    }
                  : null
          }
          menuOpen={menuOpen()}
          addDisabled={draft() !== null}
        />
        {/* Always mounted while mobile, so the first-run auth prompt
            still fires on load as on desktop, where the sidebar's
            StatusSlot owns it. */}
        <FirstRunAuthPrompt
          session={session.session()}
          onSession={session.swapSession}
        />
      </Show>
    </div>
  );
}

function viewTitle(
  v: ViewKey,
  lists: { id: string; name: string }[],
  m: ReturnType<typeof useAppI18n>["m"] extends () => infer T ? T : never,
): string {
  if (v.kind === "list") {
    // Reserved `inbox` has no `ListMeta` row — it always renders the
    // localized built-in label.
    if (v.id === "inbox") return m.nav.inbox;
    return lists.find((l) => l.id === v.id)?.name ?? v.id;
  }
  if (v.kind === "focus") return m.nav.focus;
  if (v.kind === "upcoming") return m.nav.upcoming;
  if (v.kind === "done") return m.nav.done;
  return m.nav.bin;
}

/** Fixed navbar glyph for a reserved view (Focus, Home/Inbox, Done, Bin),
 *  or `null` for user lists (which carry their own `ListIconPicker`). Keep
 *  these in sync with the icons used in `nav.tsx`. */
function viewIcon(v: ViewKey): string | null {
  if (v.kind === "focus") return starSvg;
  if (v.kind === "upcoming") return calendarSvg;
  if (v.kind === "done") return checkSvg;
  if (v.kind === "bin") return crumpledPaperSvg;
  if (v.kind === "list" && v.id === "inbox") return archiveSvg;
  return null;
}
