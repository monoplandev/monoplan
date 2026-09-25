// Board view of one list (spec/board.md). A second lens over the same
// Open projection the list view renders: five fixed lanes driven by
// item lifecycle — Backlog | Todo | In Progress | Review | Done — with
// no user-created, renamed, reordered, or deleted lanes. The lane *set*
// is fixed; lane *visibility* is not: a client may hide lanes (all but
// one), a display-only local preference owned by the workspace.
//
// The four open lanes are views of the list's single Open order,
// partitioned by each item's workflow register state; Done is the list's
// done-but-not-binned items, sorted by the register's transition time.
// Every lane hosts its own Dnd instance, so within-lane reorders use the
// standard placeholder/nudge machinery. Cross-lane drops ride the dnd's
// foreign-drop-zone contract: each lane carries a `data-drop-column-id`,
// a document-level `primavera-dnd-dragend` listener hit-tests the
// pointer and consumes the drag (preventDefault suppresses the source
// lane's snap-back reorder), and the move maps to a `setItemLifecycle`
// (the target lane *is* the target state) — the lane-drop primitive —
// plus an optional same-commit reorder within the shared Open order.

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  Show,
} from "solid-js";
import { Dnd, DndSelection, type DndImperative, type DndOp } from "./dnd/solid";
import type { DndDragEventDetail } from "./dnd";
import checkSvg from "./icons/check.svg?raw";
import plusSvg from "./icons/plus.svg?raw";
import { laneLabel, useAppI18n } from "./i18n.tsx";
import { isOverlayOpen } from "./overlay.ts";
import { Row } from "./Row.tsx";
import { planReorderMoves } from "./reorder.ts";
import {
  isBinned,
  isClosed,
  OPEN_STATES,
  type DocApp,
  type ItemView,
  type WorkflowState,
} from "./sync/store.ts";

/** Fixed lane keys — the workflow state names plus `done`; used as DOM
 *  `data-drop-column-id` attributes and as the keys of the per-lane
 *  handle / selection maps. Item ids are uuid-v7 hex or `inbox`, so
 *  these plain words can never collide with one. */
const DONE_LANE: WorkflowState = "done";

/** Imperative handle the workspace holds to steer focus back into the
 *  board (e.g. after the detail dialog closes), mirroring the list view's
 *  `DndImperative.focus`. */
export type BoardImperative = { focusActive: () => void };

/** Fixed card height — the `itemHeight` passed to each lane's Dnd. The
 *  foreign-drag preview (placeholder/nudge) and its insertion-slot math
 *  live in the Dnd controller now; the board only needs this to size the
 *  cards uniformly. Board cards are taller than list rows so the title can
 *  wrap to two lines (see `.board-col-dnd .row-text` in styles.css). */
const CARD_HEIGHT = 88;

/** Px gutter below each card within its fixed slot — mirrors the card's
 *  `height: calc(100% - 6px)` so the drop placeholder lines up with the
 *  cards. Passed to each lane's Dnd. */
const CARD_GAP = 6;

/** Mirrors `.row.row-card`'s border-radius so drag-overlay clones conform
 *  to the card instead of showing square slot corners behind it. */
const CARD_RADIUS = 8;

/** Pointer-driven horizontal autoscroll for the board strip. */
const H_SCROLL_EDGE = 48;
const H_SCROLL_STEP = 14;

export function Board(props: {
  app: DocApp;
  listId: string;
  onOpen: (id: string) => void;
  onSetDeadline: (ids: readonly string[], initial: string | null) => void;
  onSetWhen: (ids: readonly string[], initial: string | null) => void;
  /** Row context-menu jump to the item's other appearance (the Focus
   *  lens). Forwarded to each card. */
  onReveal?: (id: string, where: "list" | "focus") => void;
  /** Row context-menu move-to-list (opens the workspace's move palette).
   *  Forwarded to each card. */
  onMoveToList?: (ids: readonly string[]) => void;
  openOnTap: () => boolean;
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  /** Open the new-item dialog targeting a lane: an open workflow state,
   *  or `done: true` for the Done lane's log-a-completion capture. */
  onAddItem: (listId: string, state: WorkflowState, done?: boolean) => void;
  /** Ids to select and scroll into view once they land in their (shared)
   *  lane — a "+" capture, a duplicated block, or a find pick; `null`
   *  when nothing pending. */
  revealIds?: () => string[] | null;
  /** Called by the board once it has revealed `revealIds`. */
  clearReveal?: () => void;
  /** Whether the Done lane is shown (per-list view preference owned by
   *  the workspace; the one lane-visibility bit that can sync via the
   *  saved default view). Defaults to shown when absent. */
  showDoneColumn?: () => boolean;
  /** Which *open* lanes render — client-local, display-only lane hiding
   *  (spec/board.md "Lane visibility"), owned by the workspace. Hidden
   *  lanes don't render and are not drop targets. Defaults to all four.
   *  The workspace guarantees at least one lane stays visible. */
  visibleOpenLanes?: () => readonly WorkflowState[];
  /** Publishes the board's active (most recently populated) lane
   *  selection, or `null` when nothing is selected, so the workspace's
   *  global item shortcuts can act on it. */
  onActiveSelectionChange?: (sel: DndSelection | null) => void;
  /** Receives an imperative handle so the workspace can restore keyboard
   *  focus to the active lane (e.g. after the detail dialog closes). */
  ref?: (h: BoardImperative) => void;
}) {
  const { m } = useAppI18n();
  const app = props.app;
  const state = app.state;

  // Open projection partitioned by each item's workflow register state
  // into the four open lanes. One walk of the list's Open array; every
  // lane preserves the shared relative order (spec/board.md).
  const laneMembers = createMemo((): Record<WorkflowState, ItemView[]> => {
    const lanes: Record<WorkflowState, ItemView[]> = {
      backlog: [],
      todo: [],
      in_progress: [],
      review: [],
      done: [],
      cancelled: [], // closed states never appear in listOpen
    };
    for (const id of state.listOpen[props.listId] ?? []) {
      const it = state.itemsById[id];
      if (!it) continue;
      lanes[it.state].push(it);
    }
    return lanes;
  });

  // Members of the Done lane: this list's closed-but-not-binned items
  // (done and cancelled), newest-closed first (the workflow register's
  // `at`) — the same slice
  // (and sort) the list view's Done filter and the global Done view use.
  // Not order-container backed: the board's Open projection only covers
  // open items, so this is a scan of `itemsById` scoped to the list.
  // Runs only while a board is mounted.
  const doneMembers = createMemo((): ItemView[] => {
    const out: ItemView[] = [];
    for (const it of Object.values(state.itemsById)) {
      if (it.listId === props.listId && isClosed(it) && !isBinned(it)) {
        out.push(it);
      }
    }
    out.sort((a, b) => b.lifecycleAt - a.lifecycleAt);
    return out;
  });

  // Resolve a lane key to its member list.
  const membersOf = (laneKey: WorkflowState): ItemView[] =>
    laneKey === DONE_LANE ? doneMembers() : laneMembers()[laneKey];

  // Which lanes render, in fixed left-to-right order. Lane hiding is
  // display-only and rides the list's view spec (local override or the
  // saved default — see view.ts).
  const doneVisible = (): boolean => props.showDoneColumn?.() ?? true;
  const visibleOpen = (): readonly WorkflowState[] =>
    props.visibleOpenLanes?.() ?? OPEN_STATES;

  // Which lane a card currently lives in. A closed card's home is the
  // Done lane; a binned card never renders here.
  const sourceLaneOf = (id: string): WorkflowState => {
    const it = state.itemsById[id];
    if (!it) return "backlog";
    if (isClosed(it) && !isBinned(it)) return DONE_LANE;
    return it.state;
  };

  // End-of-lane drops have no `beforeKey`; anchor on the first Open item
  // after the lane's last surviving member so the block lands at the
  // lane's tail without disturbing the interleaving of the other lane.
  // `null` = append to the list end; `undefined` = the lane has no
  // surviving members, so linear position is irrelevant.
  const planAnchor = (
    memberIds: readonly string[],
    moved: ReadonlySet<string>,
    beforeKey: string | null,
  ): string | null | undefined => {
    if (beforeKey !== null) return beforeKey;
    const open = state.listOpen[props.listId] ?? [];
    const remaining = memberIds.filter((id) => !moved.has(id));
    if (remaining.length === 0) return undefined;
    const last = remaining[remaining.length - 1];
    for (let i = open.indexOf(last) + 1; i < open.length; i++) {
      if (!moved.has(open[i])) return open[i];
    }
    return null;
  };

  // Within-lane reorder: map the lane-local drop to moves in the shared
  // Open order. The register is untouched — same lane, same lifecycle.
  const reorderWithin = (laneKey: WorkflowState, op: DndOp<ItemView>): void => {
    if (op.type !== "move") return;
    const moved = op.keys.map(String);
    const memberIds = membersOf(laneKey).map((it) => it.id);
    const anchor = planAnchor(
      memberIds,
      new Set(moved),
      op.beforeKey === null ? null : String(op.beforeKey),
    );
    if (anchor === undefined) return;
    const open = state.listOpen[props.listId] ?? [];
    const moves = planReorderMoves(open, moved, anchor);
    if (moves.length === 0) return;
    app.withActionBatch(() => {
      for (const move of moves) app.moveItem(move.id, props.listId, move.index);
    });
  };

  // Live Dnd handles per lane (keyed by laneKey), registered by each
  // BoardColumn on mount. The cross-lane drag listener drives the hovered
  // foreign lane's placeholder/nudge/autoscroll preview through these and
  // reads back the previewed insertion slot at drop time — so the drop
  // lands exactly where the preview showed. This is the shared
  // "DragContext": the board coordinates its lanes.
  const laneHandles = new Map<WorkflowState, DndImperative>();
  const registerHandle = (laneKey: WorkflowState, handle: DndImperative | null) => {
    if (handle) laneHandles.set(laneKey, handle);
    else laneHandles.delete(laneKey);
  };
  const clearAllForeignHover = () => {
    for (const h of laneHandles.values()) h.clearForeignHover();
  };

  // One selection model per lane, owned by the board (keyed by the stable
  // laneKey) so a lane remount doesn't drop its selection and so the
  // cross-lane drop below can make the moved rows the target lane's new
  // selection. Cross-lane multi-select still isn't supported — each lane's
  // Dnd owns its own order.
  //
  // The board also tracks which lane's selection is "active" (the most
  // recently populated one) and publishes it upward, so the workspace's
  // global item shortcuts (Enter/open, x/done, ⌫/bin, ⌘C, ⌘D) act on the
  // board selection just as they do on the list view's single selection.
  const laneSelections = new Map<WorkflowState, DndSelection>();
  const [activeSelection, setActiveSelection] =
    createSignal<DndSelection | null>(null);
  // The lane that most recently held the active selection, kept after that
  // selection is cleared (a move-out, a bin) so focus restore still has a
  // listbox to land on when no lane is active.
  let lastActiveLane: WorkflowState | null = null;
  // Bring a lane's column fully into the board strip's horizontal view.
  // `inline: "nearest"` is test-then-minimal-scroll: an already-visible
  // column no-ops, a partly-clipped one slides just past its near edge
  // (plus `.board-col`'s scroll-margin gutter), and one wider than the
  // viewport aligns its near edge — the "if possible" case.
  const ensureLaneVisible = (laneKey: WorkflowState): void => {
    boardRef
      ?.querySelector(`[data-drop-column-id="${laneKey}"]`)
      ?.scrollIntoView({ inline: "nearest", block: "nearest", behavior: "smooth" });
  };
  const selectionFor = (laneKey: WorkflowState): DndSelection => {
    let s = laneSelections.get(laneKey);
    if (!s) {
      const sel = new DndSelection();
      sel.onChange(() => {
        if (sel.hasSelection()) {
          // Selecting in one lane is board-wide single selection: drop any
          // stale highlight in the other lanes and become the active one.
          for (const other of laneSelections.values()) {
            if (other !== sel && other.hasSelection()) other.clear();
          }
          setActiveSelection(sel);
          lastActiveLane = laneKey;
          // Activation is the one funnel every lane-landing path shares —
          // card click, ←/→ lane hop, cross-lane drop, "+" capture, find
          // pick — so the column scrolls into view for all of them.
          ensureLaneVisible(laneKey);
        } else if (activeSelection() === sel) {
          setActiveSelection(null);
        }
      });
      laneSelections.set(laneKey, sel);
      s = sel;
    }
    return s;
  };
  createEffect(() => props.onActiveSelectionChange?.(activeSelection()));
  onCleanup(() => props.onActiveSelectionChange?.(null));

  // After a cross-lane drop, make the dropped rows the target lane's
  // selection. The mutation is applied synchronously but the moved rows
  // only appear in the target's member list once the reactive graph
  // settles, so we stage the request and apply it when the rows have
  // actually landed (their order is then known, so the block ranges are
  // correct regardless of effect timing).
  const [pendingSelect, setPendingSelect] = createSignal<{
    laneKey: WorkflowState;
    ids: string[];
  } | null>(null);
  createEffect(() => {
    const p = pendingSelect();
    if (!p) return;
    const memberIds = membersOf(p.laneKey).map((it) => it.id);
    const present = new Set(memberIds);
    if (!p.ids.every((id) => present.has(id))) return;
    const sel = laneSelections.get(p.laneKey);
    if (sel) {
      sel.updateOrder(memberIds);
      sel.setSelectedKeys(p.ids);
      const handle = laneHandles.get(p.laneKey);
      handle?.focus();
      // Scroll the (last) selected card into view — after a board "+"
      // capture the new card may be below the fold in a tall lane.
      if (p.ids.length > 0) handle?.scrollToKey(p.ids[p.ids.length - 1]);
    }
    setPendingSelect(null);
  });

  // Reveal a just-created item (board "+" capture): once it lands in its
  // lane, stage it as that lane's selection — the effect above then
  // selects, focuses, and scrolls it into view. Waits for the reactive
  // graph to settle (the item may not be projected yet).
  createEffect(() => {
    const ids = props.revealIds?.() ?? null;
    if (!ids || ids.length === 0) return;
    // The revealed ids share a lane (a duplicated block, or a single
    // item); resolve it from the first and wait until every id has landed
    // there before staging them all as that lane's selection.
    if (!state.itemsById[ids[0]]) return;
    const laneKey = sourceLaneOf(ids[0]);
    const present = new Set(membersOf(laneKey).map((it) => it.id));
    if (!ids.every((id) => present.has(id))) return;
    setPendingSelect({ laneKey, ids });
    props.clearReveal?.();
  });

  // Foreign-drop wiring (see dnd/spec.md "Foreign drop zones"). The
  // workspace's own nav/bin drop listener coexists: hit-tests are
  // disjoint, and this one only consumes drops landing on a *different*
  // lane — same-lane drops fall through to the source Dnd's internal
  // reorder.
  const findLaneAt = (x: number, y: number): HTMLElement | null =>
    document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>("[data-drop-column-id]") ?? null;
  const isItemDrag = (detail: DndDragEventDetail): boolean => {
    const first = detail.firstItem;
    return typeof first === "object" && first !== null && "listId" in first;
  };
  const clearLaneHighlight = () => {
    document
      .querySelectorAll<HTMLElement>(".board-col[data-drop-active]")
      .forEach((el) => delete el.dataset.dropActive);
  };
  // Resolve a hovered element's lane key; anything unexpected reads as
  // Backlog (the keys are our own dataset values).
  const laneKeyOf = (el: HTMLElement): WorkflowState => {
    const raw = el.dataset.dropColumnId;
    return raw === "todo" ||
      raw === "in_progress" ||
      raw === "review" ||
      raw === "done"
      ? raw
      : "backlog";
  };
  const onDragMove = (e: Event) => {
    const ce = e as CustomEvent<DndDragEventDetail>;
    if (!isItemDrag(ce.detail)) return;
    // Horizontal autoscroll of the board strip near its edges —
    // pointer-move-driven, which is enough at card-drag speeds.
    if (boardRef) {
      const rect = boardRef.getBoundingClientRect();
      if (ce.detail.y >= rect.top && ce.detail.y <= rect.bottom) {
        if (ce.detail.x < rect.left + H_SCROLL_EDGE) {
          boardRef.scrollLeft -= H_SCROLL_STEP;
        } else if (ce.detail.x > rect.right - H_SCROLL_EDGE) {
          boardRef.scrollLeft += H_SCROLL_STEP;
        }
      }
    }
    clearLaneHighlight();
    const el = findLaneAt(ce.detail.x, ce.detail.y);
    const keys = ce.detail.keys.map(String);
    // The hovered lane is a *foreign* drop target only when it's a
    // different lane from where the dragged rows currently live.
    const targetLane =
      el && !keys.every((k) => sourceLaneOf(k) === laneKeyOf(el))
        ? laneKeyOf(el)
        : null;
    // Preview the drop in the hovered foreign lane; clear every other
    // lane (including the source, whose own controller suppresses its
    // placeholder/nudge once the pointer leaves its bounds).
    for (const [laneKey, handle] of laneHandles) {
      if (laneKey === targetLane) handle.setForeignHover(ce.detail.x, ce.detail.y);
      else handle.clearForeignHover();
    }
    if (targetLane !== null && el) el.dataset.dropActive = "";
  };
  const onDragEnd = (e: Event) => {
    const ce = e as CustomEvent<DndDragEventDetail>;
    clearLaneHighlight();
    if (!isItemDrag(ce.detail)) {
      clearAllForeignHover();
      return;
    }
    const el = findLaneAt(ce.detail.x, ce.detail.y);
    const laneKey = el ? laneKeyOf(el) : null;
    const keys = ce.detail.keys.map(String);
    // Same-lane (or off-board) drop → the source lane's internal reorder
    // owns it; nothing to consume here. (Done→Done lands here too: the
    // Done lane has no linear order, so a drop within it is a no-op.)
    if (laneKey === null || keys.every((k) => sourceLaneOf(k) === laneKey)) {
      clearAllForeignHover();
      return;
    }
    ce.preventDefault();
    // The target lane's preview computed the exact insertion slot — read
    // it back so the drop lands where the placeholder was. `foreignIdx` is
    // a slot in that lane's member order (0..count); >= count (or null, if
    // the pointer never registered a hover) means tail append.
    const foreignIdx = laneHandles.get(laneKey)?.getForeignHoverIndex() ?? null;
    clearAllForeignHover();
    // A drag carries rows from exactly one source lane (cross-lane
    // multi-select isn't supported), so it's all-open or all-done. Order
    // open rows by the shared Open order; done rows by newest-done.
    const keySet = new Set(keys);
    const open = state.listOpen[props.listId] ?? [];
    const fromOpen = open.filter((id) => keySet.has(id));
    const sourceIsDone = fromOpen.length === 0;
    const inOrder = sourceIsDone
      ? doneMembers()
          .map((it) => it.id)
          .filter((id) => keySet.has(id))
      : fromOpen;
    if (inOrder.length === 0) return;

    if (laneKey === DONE_LANE) {
      // Drop into Done → mark the (open) rows done. Done is
      // timestamp-ordered, so the drop position is ignored
      // (spec/board.md).
      app.setLifecycleMany(inOrder, "done");
    } else if (sourceIsDone) {
      // Drop a done row onto an open lane → that lane *is* the target
      // state. Its order entry was preserved, so it reappears at its
      // former Open slot; precise vertical placement within the lane is
      // out of scope here.
      app.setLifecycleMany(inOrder, laneKey);
    } else {
      // Open→open cross-lane drop: write the target lane's state and
      // place the rows before the previewed anchor in the shared Open
      // order, one commit.
      const target = laneKey;
      const memberIds = membersOf(laneKey).map((it) => it.id);
      const remaining = memberIds.filter((id) => !keySet.has(id));
      const anchor =
        foreignIdx !== null && foreignIdx < remaining.length
          ? remaining[foreignIdx]
          : planAnchor(memberIds, keySet, null);
      const planned =
        anchor === undefined ? [] : planReorderMoves(open, inOrder, anchor);
      app.withActionBatch(() => {
        for (const id of inOrder) app.setLifecycle(id, target);
        for (const p of planned) app.moveItem(p.id, props.listId, p.index);
      });
    }
    // Clear the source lane's now-stale selection, then stage the moved
    // rows to become the target lane's selection once they land.
    const sourceLanes = new Set(keys.map((k) => sourceLaneOf(k)));
    for (const c of sourceLanes) laneSelections.get(c)?.clear();
    setPendingSelect({ laneKey, ids: inOrder });
  };
  // Escape mid-drag: the source lane tears its own drag down; here we just
  // drop any cross-lane placeholder preview the drag had painted.
  const onDragCancel = () => {
    clearLaneHighlight();
    clearAllForeignHover();
  };
  document.addEventListener("primavera-dnd-dragmove", onDragMove);
  document.addEventListener("primavera-dnd-dragend", onDragEnd);
  document.addEventListener("primavera-dnd-dragcancel", onDragCancel);
  onCleanup(() => {
    document.removeEventListener("primavera-dnd-dragmove", onDragMove);
    document.removeEventListener("primavera-dnd-dragend", onDragEnd);
    document.removeEventListener("primavera-dnd-dragcancel", onDragCancel);
  });

  let boardRef: HTMLDivElement | undefined;

  // ArrowLeft / ArrowRight: hop the selection to the nearest lane in that
  // direction that holds cards, landing on the card at the same *screen*
  // height as the current one (clamped to the target lane's length). Lanes
  // render in the fixed ladder order (visible ones only). No wrap —
  // left/right is spatial. Empty lanes are skipped (nothing to land on);
  // with nothing selected, → enters at the first non-empty lane and ← at
  // the last.
  //
  // Matching by screen height rather than by slot index is what keeps the
  // hop from scrolling: two lanes scrolled to different offsets put slot N
  // at different heights, so landing on the target's slot N would drag its
  // viewport to wherever that card happens to be. Every lane's listbox
  // lays card N out at N × CARD_HEIGHT from its own top (the board never
  // expands cards), so the listboxes' viewport offsets are the whole story.
  const orderedLaneKeys = (): WorkflowState[] => [
    ...visibleOpen(),
    ...(doneVisible() ? [DONE_LANE] : []),
  ];
  const laneListboxTop = (laneKey: WorkflowState): number | null => {
    const el = boardRef?.querySelector(
      `[data-drop-column-id="${laneKey}"] [role="listbox"]`,
    );
    return el ? el.getBoundingClientRect().top : null;
  };
  const jumpLane = (dir: -1 | 1): void => {
    const order = orderedLaneKeys();
    const active = activeSelection();
    // Locate the active lane, the caret's slot within it, and the caret
    // card's vertical midpoint on screen.
    let curIdx = dir === 1 ? -1 : order.length;
    let curPos = 0;
    let curMidY: number | null = null;
    if (active) {
      for (const [laneKey, sel] of laneSelections) {
        if (sel !== active) continue;
        curIdx = order.indexOf(laneKey);
        const ids = membersOf(laneKey).map((it) => it.id);
        const top = active.getSelectionTop();
        curPos = top != null ? Math.max(0, ids.indexOf(String(top))) : 0;
        const listboxTop = laneListboxTop(laneKey);
        if (listboxTop != null) {
          curMidY = listboxTop + (curPos + 0.5) * CARD_HEIGHT;
        }
        break;
      }
    }
    for (let i = curIdx + dir; i >= 0 && i < order.length; i += dir) {
      const laneKey = order[i];
      const ids = membersOf(laneKey).map((it) => it.id);
      if (ids.length === 0) continue;
      // Prefer the card under the caret's midpoint in the target lane's
      // own scrolled frame; fall back to the slot index when there's no
      // geometry to read (no active lane, or a lane not yet laid out).
      let pos = curPos;
      const targetTop = curMidY == null ? null : laneListboxTop(laneKey);
      if (curMidY != null && targetTop != null) {
        pos = Math.floor((curMidY - targetTop) / CARD_HEIGHT);
      }
      const key = ids[Math.max(0, Math.min(pos, ids.length - 1))];
      const sel = selectionFor(laneKey);
      sel.updateOrder(ids);
      // Selecting here clears the other lanes via selectionFor's onChange.
      sel.selectOnly(key);
      const handle = laneHandles.get(laneKey);
      handle?.focus();
      handle?.scrollToKey(key);
      return;
    }
  };
  // Restore keyboard focus to the lane that currently owns the active
  // selection (its Dnd listbox is what up/down nav is bound to). Used after
  // the detail dialog closes so board nav resumes where it left off. With
  // no active selection — the move palette re-filed the selected cards
  // out of this board, or ⌫ binned them — fall back to the lane that last
  // held one, then to the first mounted lane, so focus never strands on
  // body: a focused lane with nothing selected answers ↓ / ↑ by selecting
  // its first / last card, and ← / → still hop lanes.
  const focusActiveLane = (): void => {
    const active = activeSelection();
    if (active) {
      for (const [laneKey, sel] of laneSelections) {
        if (sel !== active) continue;
        laneHandles.get(laneKey)?.focus();
        return;
      }
    }
    const fallback =
      (lastActiveLane !== null ? laneHandles.get(lastActiveLane) : undefined) ??
      orderedLaneKeys()
        .map((k) => laneHandles.get(k))
        .find((h) => h !== undefined);
    fallback?.focus();
  };
  props.ref?.({ focusActive: focusActiveLane });

  const onBoardKeyDown = (e: KeyboardEvent): void => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    // Row context menus portal out of the board, but Solid routes delegated
    // keydown back through the JSX tree — so lane hopping would fire while a
    // menu is open (and left/right belongs to the menu's submenus).
    if (isOverlayOpen()) return;
    const target = e.target as Element | null;
    if (target?.closest('input, textarea, [contenteditable="true"]')) return;
    e.preventDefault();
    jumpLane(e.key === "ArrowRight" ? 1 : -1);
  };

  return (
    <div class="board" role="group" tabIndex={-1} ref={boardRef} onKeyDown={onBoardKeyDown}>
      {/* The four open lanes (minus locally hidden ones), in fixed ladder
          order. The first visible lane claims the mount autofocus. */}
      <For each={visibleOpen()}>
        {(lane, i) => (
          <BoardColumn
            app={app}
            laneKey={lane}
            name={laneLabel(m(), lane)}
            selection={selectionFor(lane)}
            members={() => laneMembers()[lane]}
            onReorder={(op) => reorderWithin(lane, op)}
            onAddItem={() => props.onAddItem(props.listId, lane)}
            registerHandle={registerHandle}
            autofocus={i() === 0}
            onOpen={props.onOpen}
            onSetDeadline={props.onSetDeadline}
            onSetWhen={props.onSetWhen}
            onReveal={props.onReveal}
            onMoveToList={props.onMoveToList}
            openOnTap={props.openOnTap}
            duplicateBlock={props.duplicateBlock}
            copyBlock={props.copyBlock}
          />
        )}
      </For>
      {/* Fixed Done lane: this list's done items, newest first. A drop
          target (drag a card in to mark it done) and a drag source (drag a
          card out to move it back to an open state), but never internally
          reordered — it has no linear order of its own. Toggled by a
          per-list preference from the view-mode popover. */}
      <Show when={doneVisible()}>
        <BoardColumn
          app={app}
          laneKey={DONE_LANE}
          variant="done"
          name={m().board.doneLane}
          selection={selectionFor(DONE_LANE)}
          members={() => doneMembers()}
          onReorder={() => {}}
          onAddItem={() => props.onAddItem(props.listId, "backlog", true)}
          registerHandle={registerHandle}
          onOpen={props.onOpen}
          onSetDeadline={props.onSetDeadline}
          onSetWhen={props.onSetWhen}
          onReveal={props.onReveal}
          onMoveToList={props.onMoveToList}
          openOnTap={props.openOnTap}
          duplicateBlock={props.duplicateBlock}
          copyBlock={props.copyBlock}
        />
      </Show>
    </div>
  );
}

function BoardColumn(props: {
  app: DocApp;
  /** Fixed lane key: a workflow state name (`done` = the Done lane). */
  laneKey: WorkflowState;
  name: string;
  /** Selection model for this lane, owned by the board. */
  selection: DndSelection;
  members: () => ItemView[];
  onReorder: (op: DndOp<ItemView>) => void;
  /** Open the new-item dialog targeting this lane. */
  onAddItem: () => void;
  onOpen: (id: string) => void;
  onSetDeadline: (ids: readonly string[], initial: string | null) => void;
  onSetWhen: (ids: readonly string[], initial: string | null) => void;
  onReveal?: (id: string, where: "list" | "focus") => void;
  onMoveToList?: (ids: readonly string[]) => void;
  openOnTap: () => boolean;
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  /** Publish this lane's Dnd handle to the board so the cross-lane drag
   *  listener can drive its foreign-drop preview. */
  registerHandle: (laneKey: WorkflowState, handle: DndImperative | null) => void;
  /** Focus this lane's listbox on mount, so arrow-key nav works the moment
   *  the board opens — the Backlog lane claims it (matches how the list
   *  view autofocuses). Only one lane should set this. */
  autofocus?: boolean;
  /** `"done"` renders the Done lane: a check-marked label and no internal
   *  reorder — its cards are lifecycle-grouped and timestamp-sorted, not
   *  order-container backed. Its "+" logs a directly-completed item. */
  variant?: "done";
}) {
  const { m } = useAppI18n();
  const app = props.app;
  const isDoneCol = props.variant === "done";
  // Selection is owned by the board (per lane) — see Board.selectionFor.
  const selection = props.selection;
  const [dndItems, setDndItems] = createSignal<ItemView[]>([]);
  createEffect(() => setDndItems(props.members()));
  onCleanup(() => props.registerHandle(props.laneKey, null));

  return (
    <section
      class="board-col"
      classList={{ "board-col-done": isDoneCol }}
      data-drop-column-id={props.laneKey}
    >
      <header class="board-col-header">
        <span class="board-col-name board-col-name-fixed">
          <Show when={isDoneCol}>
            <span
              class="board-col-done-icon"
              aria-hidden="true"
              innerHTML={checkSvg}
            />
          </Show>
          {props.name}
        </span>
        <span class="board-col-count">{props.members().length}</span>
        <button
          type="button"
          class="board-col-add-btn"
          tabIndex={-1}
          aria-label={isDoneCol ? m().workspace.logCompleted : m().board.addItem}
          onClick={() => props.onAddItem()}
          innerHTML={plusSvg}
        />
      </header>
      <div class="board-col-body">
        <Dnd
          class="board-col-dnd"
          ref={(h) => props.registerHandle(props.laneKey, h)}
          items={dndItems()}
          setItems={setDndItems}
          getKey={(it) => it.id}
          selection={selection}
          itemHeight={CARD_HEIGHT}
          placeholderGap={CARD_GAP}
          overlayRadius={CARD_RADIUS}
          fillHeight
          reorder={!isDoneCol}
          autofocus={props.autofocus}
          onReorder={props.onReorder}
        >
          {(item, expanded) => (
            <Row
              item={item}
              expanded={expanded}
              app={app}
              selection={selection}
              viewKind={isDoneCol ? "done" : "list"}
              duplicateBlock={props.duplicateBlock}
              copyBlock={props.copyBlock}
              onOpen={props.onOpen}
              onSetDeadline={props.onSetDeadline}
              onSetWhen={props.onSetWhen}
              onReveal={props.onReveal}
              onMoveToList={props.onMoveToList}
              openOnTap={props.openOnTap}
              deadlineInFooter
            />
          )}
        </Dnd>
      </div>
    </section>
  );
}
