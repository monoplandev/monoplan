// The Upcoming view: the agenda. Every Open item with a planned date,
// bucketed by day (`dayGroups.ts`, grouped by the Workspace so the same
// rows feed its selection-scoped shortcuts), Today always anchored so
// the surface reads as "the days ahead" even when nothing is on. An Open
// task whose `when` has passed sits in an Overdue section above Today;
// an event's past `when` is over and places nothing here. Rows carry no date pills: the
// day header is the date, and the task surface shows the rest. The
// checkbox leads each row, then a fixed-width lane carries the start time
// ("All day" on an event with no time, nothing on a task with none, or
// the planned date on a task that slipped into Overdue, which has no day
// header to say it); the place is a muted line under the title.
// An event ticks off in place and stays on its `when` day once done or
// cancelled, so the day still records what happened and what was called
// off. A task does not: closed, it leaves the agenda after a short
// linger where it was (the Done view has it), so what is left is not
// read through what is settled. Past days hold events only. Muted text marks the days gone by,
// every row on them, not the closed rows; Overdue is not one of them.
//
// Events (`spec/events-plan.md`): an open event's row has a small
// upright pill, a calendar's event bar, where a task has its checkbox, since nothing asks for the tick
// and the event just goes by. One marked done or cancelled (from the
// task surface or the row's menu) shows the box, ticked or crossed,
// until its day has gone by; there every event has the pill again, a
// cancelled one a bare cross. A row in a list names it on the sub line
// under the title, with the list's icon.
//
// Selection and keys: the agenda is a listbox in the list view's sense
// (click selects, the side pane follows, Enter / double-click opens, the
// item shortcuts act on the selection) but not a `Dnd`: day sections are
// the point, rows are not one height, and the flat virtualised list can't
// host group headers. So it runs the Workspace's `DndSelection` over one
// day's rows at a time (a multi-selection never spans days), answers the
// same keys through the shared `applySelectionKeyAction`, and moves the
// selection to another day when a click or plain ↑ / ↓ lands there. Rows
// drag between days to reschedule (see "Drag to reschedule" below).
//
// Windowed by day, not by row: the day sections from a pane's height
// above the pane to one below are drawn, spacers stand in for the rest
// (each day's last measured height, or an estimate from its rows), and
// the window follows the scroll. A jump to a day that is not drawn (a
// row off in the past, back to Today) draws it first. Today and the mini
// calendar scroll there with the browser's smooth scroll, over days all
// drawn beforehand, and the window holds still until it lands; a far day
// is drawn next to the pane, the days between left out until then.
//
// The days on screen go out to the Workspace (`onVisibleRange`) for
// the mini calendar to shade: the first and last day section shown by
// more than a sliver, read a frame after every scroll, smooth ones
// included (one run of a far flight's two at a time).
//
// The days gone by (a year of them, `AGENDA_PAST_DAYS`: events that
// happened, items settled on the day they were planned for) sit above,
// as on the Mac and iOS Calendar. The view opens scrolled so Overdue's
// header (Today's when nothing is overdue) is at the top of the pane,
// and the past is a scroll up away. It is put there again on every
// change to the rows until the user scrolls or picks something (the
// docs can still be loading when the view mounts); from then on a change
// keeps what is at the top of the pane where it is, so rows arriving in
// a past day do not move the page.
//
// Columns (`layout="columns"`, the agenda column view): the same day
// sections side by side, one fixed-width column a day, the pane
// scrolling sideways and each column down its own rows under a pinned
// header. Rows, selection, keys, the draft row and the drag are the
// list's. Only the windowing differs: a column's place is its index
// times the pitch (`agendaColumns.ts`), so there is nothing to measure
// and drawing a column never moves another. It opens with Overdue's
// column (else Today's) at the left edge, the past a scroll left away.
// ← / → move the selection to the day beside it, and a vertical wheel
// over a day's header scrolls the days sideways (a mouse wheel anywhere
// else in a column scrolls its rows).

import {
  batch,
  createComputed,
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
  untrack,
} from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import { Avatar } from "./Avatar.tsx";
import { ItemContextMenu, type RevealTarget } from "./ItemContextMenu.tsx";
import { pressCheck } from "./Row.tsx";
import {
  agendaItems,
  dropOnDay,
  firstDayOfWeek,
  OVERDUE_KEY,
  weekStarts,
  type DayGroup,
  type DayRow,
} from "./dayGroups.ts";
import {
  AGENDA_COL_GAP,
  AGENDA_COL_PITCH,
  AGENDA_COL_WIDTH,
  columnWindow,
  neighbourDayRow,
} from "./agendaColumns.ts";
import { DndAutoscroll } from "./dnd/dom/autoscroll.ts";
import { DndDragOverlay } from "./dnd/dom/drag-overlay.ts";
import {
  applySelectionKeyAction,
  mapDndKeyEvent,
  type DndSelection,
} from "./dnd";
import {
  addDaysToStamp,
  endTimeOf,
  formatClockTime,
  formatDayLabel,
  formatDurationShort,
  nowMs,
  todayStamp,
  whenDay,
  whenStartMs,
  whenTime,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import { isOverlayOpen } from "./overlay.ts";
import fileSvg from "./icons/file.svg?raw";
import mapPinSvg from "./icons/map-pin.svg?raw";
import plusSvg from "./icons/plus.svg?raw";
import starFilledSvg from "./icons/star-filled.svg?raw";
import usersSvg from "./icons/users.svg?raw";
import {
  isCancelled,
  isClosed,
  isEvent,
  type ItemActions,
  type ItemView,
} from "./sync/store.ts";

/** The first and last day on screen, as `YYYY-MM-DD`. */
export interface DayRange {
  from: string;
  to: string;
}

/** Where the view's opening place (Overdue, else Today: what the
 *  header's "Today" scrolls back to) sits against the pane: on screen,
 *  or off it before (above; left in columns) or after. */
export type TodayPlace = "here" | "before" | "after";

/** On screen by less than this (px) a day does not count as shown: a
 *  sliver of a day's end at the top, or of its header at the bottom,
 *  would flick the range at every boundary. */
const RANGE_MIN_SHOWN = 24;

/** The slice of `DndImperative` the Workspace drives on this view. */
export interface UpcomingImperative {
  /** Move keyboard focus to the agenda so arrow keys act on it. Never
   *  scrolls (see `DndImperative.focus`). */
  focus(): void;
  /** Bring a row into view, if it is on the agenda. */
  scrollToKey(id: string): void;
  /** The header's "Today": scroll back to where the view opens,
   *  Overdue's header or Today's when nothing is overdue. */
  scrollToToday(): void;
  /** The mini calendar's pick: put day `YYYY-MM-DD`'s header at the
   *  top of the pane. A day with no section (empty days hidden) lands
   *  on the next day that has one, else the last. */
  scrollToDay(day: string): void;
}

/** The cancelled checkbox's cross (`.task-check[data-cancelled]`), bare:
 *  the mark of a cancelled event on a day gone by. */
const crossSvg =
  '<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg"><path d="M5 5l6 6M11 5l-6 6" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round"/></svg>';

/** An event's mark in the checkbox's slot: the short upright bar a
 *  calendar leads an event with (`.upcoming-row-event-pill`). */
const EVENT_PILL = '<span class="upcoming-row-event-pill"></span>';

const isMac =
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad|iPod/.test(navigator.userAgent);

export function Upcoming(props: {
  /** The agenda merges every doc, so this is the cross-doc layer's
   *  routed actions (`sync/docs.ts`). */
  app: ItemActions;
  groups: DayGroup[];
  /** The Workspace's list selection, run here over the agenda's rows. */
  selection: DndSelection;
  /** Display label for a list id in the row's doc (Inbox is localized,
   *  others by name). */
  listLabel: (listId: string, docId?: string, dated?: boolean) => string;
  /** A list's chosen emoji, looked up in the row's doc; undefined when
   *  it has none (the row then draws the default list glyph). */
  listIcon: (listId: string, docId?: string) => string | undefined;
  /** The agenda merges every workspace, so a row from a shared one
   *  (anything but Home) names it under the title with the shared
   *  people glyph; null for Home rows, which carry no marker. */
  workspaceLabel: (docId: string) => string | null;
  /** The item's assignee as a person, or null to show none (Home,
   *  unassigned), as `Row` takes it: an open row carries their avatar
   *  at the head of its sub line. */
  assigneeOf?: (item: ItemView) => { accountId: string; email: string | null } | null;
  onOpen: (id: string) => void;
  /** Touch: a tap opens the row. */
  openOnTap?: () => boolean;
  ref?: (handle: UpcomingImperative) => void;
  /** Context menu wiring, as the list rows take it (`ItemContextMenu`). */
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  onReveal?: (id: string, where: RevealTarget) => void;
  onMoveToList?: (ids: readonly string[]) => void;
  /** A day's Add (its header's "+", `data-add-day`):
   *  capture a new item on day `YYYY-MM-DD`. Today and later only. */
  onAddOnDay?: (day: string) => void;
  /** The day's draft row, at the end of that day's rows (in place of an
   *  empty day's "Nothing scheduled"); `key` remounts it per draft. */
  draft?: () => { day: string; key: string } | null;
  /** The draft row's text on every edit (the pane's capture follows it). */
  onDraftInput?: (text: string) => void;
  /** The draft row settled, with its trimmed text: Enter, Escape (or ↑
   *  out of it), or focus leaving it. Called once per draft. */
  onDraftSettle?: (text: string, how: "enter" | "escape" | "blur") => void;
  /** Days down the page (the default) or a column a day. Read once: the
   *  Workspace remounts the view when the mode changes. */
  layout?: "list" | "columns";
  /** The days on screen, first and last (`YYYY-MM-DD`), when they
   *  change; null on leaving. For the mini calendar's shading. */
  onVisibleRange?: (range: DayRange | null) => void;
  /** Where Today sits against the pane, when it changes (the header's
   *  "Today" points at it, and is gone while it is on screen). */
  onTodayPlace?: (place: TodayPlace) => void;
}) {
  const { m, locale } = useAppI18n();

  let hostEl!: HTMLDivElement;
  const cols = props.layout === "columns";

  // The selection lives in one day (`selDay`): its order is that day's
  // rows, so a shift or ⌘ range never crosses into another day, and the
  // selected rows are that day's rows of the selected items. A click or
  // a key that lands on another day moves the selection there, starting
  // afresh. Fresh array per change: `updateOrder` is identity-guarded.
  const [selDay, setSelDay] = createSignal<string | null>(null);
  const dayIds = (day: string | null): string[] =>
    day === null
      ? []
      : (props.groups.find((g) => g.key === day)?.rows.map((r) => r.item.id) ??
        []);
  const order = createMemo<string[]>(() => dayIds(selDay()));
  // What the selection is indexed over right now.
  let indexed = new Set<string>();
  const useDay = (day: string | null) => {
    setSelDay(day);
    const o = untrack(order);
    indexed = new Set(o);
    props.selection.updateOrder(o);
  };
  // Every block's ends are rows of the indexed day. Not so after the
  // Workspace selects an item by id (a new item, a duplicate) or after
  // an edit moves a selected item off the day.
  const consistent = (): boolean =>
    props.selection
      .getSelection()
      .blocks.every((b) => indexed.has(String(b.anchor)) && indexed.has(String(b.to)));
  // An item's day when it has to be found by id: its first row from
  // Overdue on (`agendaItems`).
  const flat = createMemo(() => agendaItems(props.groups));
  const firstDayOf = createMemo(
    () => new Map(flat().map((r) => [r.item.id, r.group])),
  );
  // The selected ids as of the last consistent selection, what `settle`
  // carries over when the rows change under it. Seeded from what is
  // selected on mount, so a switch between list and columns keeps it.
  let selectedIds: string[] = props.selection.getSelectedKeys().map(String);
  // The day a drop landed on, preferred by the next `settle`.
  let landingDay: string | null = null;
  // Put the selection on `ids` in one day: `prefer` when it holds any
  // of them, else the day of the first one still on the agenda.
  let settling = false;
  const settle = (ids: readonly string[], prefer: string | null) => {
    let day =
      prefer !== null && ids.some((id) => dayIds(prefer).includes(id))
        ? prefer
        : null;
    for (const id of ids) {
      if (day !== null) break;
      day = firstDayOf().get(id) ?? null;
    }
    settling = true;
    useDay(day);
    const here = ids.filter((id) => indexed.has(id));
    if (here.length === 0) props.selection.clear();
    else props.selection.setSelectedKeys(here);
    settling = false;
  };

  // Selected set as a signal so only rows whose membership flips re-render
  // their chrome. `equals: false`: the selection mutates in place.
  const [selTick, setSelTick] = createSignal(0, { equals: false });
  onCleanup(
    props.selection.onChange((sel) => {
      if (!settling && sel.hasSelection() && !consistent()) {
        const ends = sel
          .getSelection()
          .blocks.flatMap((b) => [String(b.anchor), String(b.to)]);
        settle([...new Set(ends)], selDay());
      }
      selectedIds = sel.getSelectedKeys().map(String);
      setSelTick((n) => n + 1);
    }),
  );
  const selectedSet = createMemo(() => {
    selTick();
    return props.selection.getSelectedKeySet();
  });
  // The rows changed (an edit here or elsewhere): re-index the day and
  // keep what is still selected on it, following the selection to
  // another day when its items all left this one.
  createEffect(
    on(
      () => props.groups,
      () => {
        const prefer = landingDay ?? selDay();
        landingDay = null;
        const was = selectedIds;
        useDay(selDay());
        if (was.length === 0) return;
        const now = props.selection.getSelectedKeys().map(String);
        const same =
          prefer === selDay() &&
          consistent() &&
          now.length === was.length &&
          now.every((id, i) => id === was[i]);
        if (!same) settle(was, prefer);
      },
    ),
  );
  // Selection lights `selDay`'s rows of the selected items.
  const isLit = (id: string, day: string): boolean =>
    day === selDay() && selectedSet().has(id);
  // Every row on the page in display order, an item's several rows
  // included: what plain ↑ / ↓ step through.
  const pageRows = createMemo(() =>
    props.groups.flatMap((g) =>
      g.rows.map((r) => ({ id: r.item.id, day: g.key })),
    ),
  );
  // A multi-day span drags by its first day only: the day of each span
  // item that holds its lowest `span.day` (day 1, or the earliest day
  // shown when day 1 is not: a task span that began before today starts
  // on today's page). Its other days are tail rows, which select but
  // do not pick up a drag (`data-span-tail`).
  const spanHead = createMemo(() => {
    const head = new Map<string, { day: string; n: number }>();
    for (const g of props.groups) {
      for (const r of g.rows) {
        if (!r.span) continue;
        const at = head.get(r.item.id);
        if (!at || r.span.day < at.n) {
          head.set(r.item.id, { day: g.key, n: r.span.day });
        }
      }
    }
    return head;
  });
  const isSpanTail = (r: DayRow, day: string): boolean => {
    if (!r.span) return false;
    const head = spanHead().get(r.item.id);
    return head !== undefined && head.day !== day;
  };

  // Week labels: the day that opens a week (by the locale's first day)
  // has "Week 48" at the end of its header. Over every group, not just
  // the drawn ones, so a label does not move as the window does.
  const weeks = createMemo(() =>
    weekStarts(props.groups, firstDayOfWeek(locale())),
  );

  // Day windowing: only the day sections near the pane are in the DOM,
  // from one pane's height above it to one below, with a spacer each
  // side standing in for the rest. A day's height is what it measured
  // when it was last drawn, else an estimate from its rows; drawing a
  // day changes the estimate to the truth, and the hold below keeps
  // what is at the top of the pane where it is when that moves things.
  const measured = new Map<string, number>();
  const DAY_HEADER_EST = 28;
  const ROW_EST = 40;
  const SLOT_EST = 32;
  const DAY_GAP_EST = 7;
  const heightOf = (g: DayGroup): number =>
    measured.get(g.key) ??
    DAY_HEADER_EST +
      DAY_GAP_EST +
      (g.rows.length === 0 ? SLOT_EST : g.rows.length * ROW_EST);
  // `out[i]` is where day `i` starts; `out[n]` is the whole height.
  const offsets = (groups: readonly DayGroup[]): number[] => {
    const out = [0];
    for (const g of groups) out.push(out[out.length - 1]! + heightOf(g));
    return out;
  };
  // A day's height is its box plus the margin after it. Fractional,
  // not `offsetHeight`: the header's hairline is half a pixel, and a
  // rounded height per day adds up to a page that moves as days swap
  // in and out of the spacers.
  const measureDrawn = () => {
    for (const el of hostEl.querySelectorAll<HTMLElement>("[data-group]")) {
      const gap = parseFloat(getComputedStyle(el).marginBottom) || 0;
      measured.set(el.dataset.group!, el.getBoundingClientRect().height + gap);
    }
  };
  const anchorIndex = (): number => {
    const i = props.groups.findIndex(
      (g) => g.urgency === "overdue" || g.urgency === "today",
    );
    return i < 0 ? 0 : i;
  };
  type Span = { start: number; end: number };
  const [win, setWin] = createSignal<Span>({ start: anchorIndex(), end: anchorIndex() });
  // A second run of days drawn straight after (or before) the window with
  // no spacer between: the far end of a smooth scroll, drawn next to the
  // pane so the scroll is a short one (see `scrollToGroup`). Null except
  // in flight.
  const [far, setFar] = createSignal<Span | null>(null);
  // Clamped against the groups as they are now: a change can shorten them.
  const clamp = (s: Span): Span => {
    const n = props.groups.length;
    const start = Math.min(s.start, Math.max(0, n - 1));
    const end = Math.min(Math.max(s.end, start), n - 1);
    return { start, end };
  };
  const span = createMemo(() => clamp(win()));
  // The drawn runs in page order. Runs that touch or overlap (a change
  // to the groups mid-flight) are drawn as one, so no day is drawn twice.
  const runs = createMemo((): Span[] => {
    const s = span();
    const f = far();
    if (!f) return [s];
    const c = clamp(f);
    if (c.start > s.end + 1) return [s, c];
    if (c.end < s.start - 1) return [c, s];
    return [{ start: Math.min(s.start, c.start), end: Math.max(s.end, c.end) }];
  });
  const shown = createMemo(() =>
    runs().flatMap((r) => props.groups.slice(r.start, r.end + 1)),
  );
  const spacers = createMemo(() => {
    const rs = runs();
    if (cols) {
      const n = props.groups.length;
      return {
        top: rs[0]!.start * AGENDA_COL_PITCH,
        bottom: Math.max(0, n - 1 - rs[rs.length - 1]!.end) * AGENDA_COL_PITCH,
      };
    }
    const at = offsets(props.groups);
    return {
      top: at[rs[0]!.start]!,
      bottom: at[at.length - 1]! - at[rs[rs.length - 1]!.end + 1]!,
    };
  });

  // Opening on Overdue / Today, and holding the page still under a
  // change (see the header comment).
  let anchoring = true;
  const offsetOf = (el: Element): number =>
    el.getBoundingClientRect().top - hostEl.getBoundingClientRect().top;
  // Overdue comes first in the DOM when it is there.
  const anchorEl = () =>
    hostEl.querySelector('[data-urgency="overdue"], [data-urgency="today"]');
  // The row (or day header) at the top of the pane and where it sits,
  // read before a change reaches the DOM and put back after it.
  type Held = { selector: string; offset: number } | null;
  let held: Held = null;
  // Probed a little way down as well: the pane's top can sit in the
  // margin between two days, which belongs to neither.
  const topRow = (): Held => {
    const box = hostEl.getBoundingClientRect();
    for (const dy of [2, 12, 24]) {
      const at = document.elementFromPoint(box.left + 24, box.top + dy);
      if (!at || !hostEl.contains(at)) continue;
      const row = at.closest<HTMLElement>("[data-key]");
      const day = at.closest<HTMLElement>("[data-group]");
      if (!day || !hostEl.contains(day)) continue;
      const selector = row
        ? `[data-key="${CSS.escape(row.dataset.key!)}"][data-day="${CSS.escape(row.dataset.day!)}"]`
        : `[data-group="${CSS.escape(day.dataset.group!)}"]`;
      return { selector, offset: offsetOf(row ?? day) };
    }
    return null;
  };
  const restore = (h: Held) => {
    const el = h && hostEl.querySelector(h.selector);
    if (!el) return;
    const moved = offsetOf(el) - h!.offset;
    if (Math.abs(moved) >= 1) hostEl.scrollTop += moved;
  };
  // Re-pick the drawn days from where the pane is now. Settles in a
  // pass or two: drawing a day can move the pane, which re-picks.
  // The days to draw with the pane's top at `top` (content px).
  const windowFor = (top: number): { start: number; end: number } => {
    const at = offsets(props.groups);
    const n = props.groups.length;
    const pane = hostEl.clientHeight;
    const lo = top - pane;
    const hi = top + 2 * pane;
    let start = 0;
    while (start < n - 1 && at[start + 1]! <= lo) start++;
    let end = start;
    while (end < n - 1 && at[end + 1]! < hi) end++;
    return { start, end };
  };
  // Draw `next` (and `nextFar`), keeping what is at the top of the pane
  // where it is.
  const setWindow = (next: Span, nextFar: Span | null = null) => {
    const cur = span();
    const f = far();
    if (
      cur.start === next.start &&
      cur.end === next.end &&
      f?.start === nextFar?.start &&
      f?.end === nextFar?.end
    ) {
      return false;
    }
    const h = topRow();
    batch(() => {
      setWin(next);
      setFar(nextFar);
    });
    restore(h);
    return true;
  };
  // Columns: the days from a pane's width before the pane to one after.
  // No hold: the spacers are exact, so the drawn columns stay put.
  const setColumnWindow = (next: Span) => {
    const cur = win();
    if (cur.start === next.start && cur.end === next.end) return;
    setWin(next);
  };
  const columnWindowAt = (left: number): Span =>
    columnWindow(left, hostEl.clientWidth, props.groups.length);
  const updateWindow = () => {
    if (!hostEl?.isConnected || flying) return;
    if (cols) {
      setColumnWindow(columnWindowAt(hostEl.scrollLeft));
      return;
    }
    for (let pass = 0; pass < 4; pass++) {
      measureDrawn();
      if (!setWindow(windowFor(hostEl.scrollTop))) return;
    }
  };
  let frame = 0;
  const scheduleWindow = () => {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      updateWindow();
    });
  };
  onCleanup(() => cancelAnimationFrame(frame));

  // The days on screen, for the mini calendar: the first and last drawn
  // day section shown by at least `RANGE_MIN_SHOWN` (all of a shorter
  // one), Overdue skipped. Read off the DOM a frame after a scroll, a
  // resize or a change to the rows, flights included. A far flight draws
  // its destination as a second run straight after the pane's days (the
  // days between left out), so only the run of the first day counted is
  // read: the range steps from one run to the other as the seam passes,
  // never spanning the days skipped. Published only when it changes.
  let lastRange = "";
  const groupIndex = createMemo(
    () => new Map(props.groups.map((g, i) => [g.key, i] as const)),
  );
  const publishRange = () => {
    if (!props.onVisibleRange || !hostEl?.isConnected) return;
    const rs = runs();
    let run: Span | null = null;
    const box = hostEl.getBoundingClientRect();
    const lo = cols ? box.left : box.top;
    const hi = cols ? box.right : box.bottom;
    let from: string | null = null;
    let to: string | null = null;
    for (const el of hostEl.querySelectorAll<HTMLElement>("section[data-group]")) {
      const key = el.dataset.group!;
      if (key === OVERDUE_KEY) continue;
      const r = el.getBoundingClientRect();
      const start = cols ? r.left : r.top;
      const end = cols ? r.right : r.bottom;
      if (end <= lo) continue;
      if (start >= hi) break;
      const shown = Math.min(end, hi) - Math.max(start, lo);
      if (shown < Math.min(RANGE_MIN_SHOWN, end - start)) continue;
      if (rs.length > 1) {
        const i = groupIndex().get(key) ?? -1;
        run ??= rs.find((r) => i >= r.start && i <= r.end) ?? null;
        if (run && (i < run.start || i > run.end)) break;
      }
      from ??= key;
      to = key;
    }
    const range = from && to ? { from, to } : null;
    const sig = range ? `${range.from}/${range.to}` : "";
    if (sig === lastRange) return;
    lastRange = sig;
    props.onVisibleRange(range);
  };
  // Today's place, read off the DOM with the range: its section's box
  // against the pane, else (not drawn) its index against the drawn days.
  let lastPlace: TodayPlace | null = null;
  const publishTodayPlace = () => {
    if (!props.onTodayPlace || !hostEl?.isConnected) return;
    const i = anchorIndex();
    const g = props.groups[i];
    let place: TodayPlace = "here";
    if (g) {
      const el = hostEl.querySelector<HTMLElement>(
        `section[data-group="${CSS.escape(g.key)}"]`,
      );
      if (el) {
        const box = hostEl.getBoundingClientRect();
        const r = el.getBoundingClientRect();
        const lo = cols ? box.left : box.top;
        const hi = cols ? box.right : box.bottom;
        const start = cols ? r.left : r.top;
        const end = cols ? r.right : r.bottom;
        const min = Math.min(RANGE_MIN_SHOWN, end - start);
        if (end - lo < min) place = "before";
        else if (hi - start < min) place = "after";
      } else {
        place = i < span().start ? "before" : "after";
      }
    }
    if (place === lastPlace) return;
    lastPlace = place;
    props.onTodayPlace(place);
  };
  let rangeFrame = 0;
  const scheduleRange = () => {
    if (rangeFrame) return;
    rangeFrame = requestAnimationFrame(() => {
      rangeFrame = 0;
      publishRange();
      publishTodayPlace();
    });
  };
  createEffect(() => {
    void props.groups;
    scheduleRange();
  });
  onCleanup(() => {
    cancelAnimationFrame(rangeFrame);
    if (lastRange) props.onVisibleRange?.(null);
  });

  onMount(() => {
    const ro = new ResizeObserver(() => {
      scheduleWindow();
      scheduleRange();
    });
    ro.observe(hostEl);
    onCleanup(() => ro.disconnect());
  });
  // Draw day `i` at the top of the pane, then fill in around it: for a
  // jump to a day that is not drawn (a far row, back to Today).
  const jumpToDay = (i: number) => {
    batch(() => {
      setWin({ start: i, end: i });
      setFar(null);
    });
    hostEl.scrollTop = spacers().top;
    updateWindow();
  };
  const ensureDrawn = (i: number) => {
    const { start, end } = span();
    if (i < start || i > end) jumpToDay(i);
  };

  const putAnchorAtTop = () => {
    if (cols) {
      // The anchor's column at the left edge (the browser clamps when
      // the days from there on are narrower than the pane).
      const left = anchorIndex() * AGENDA_COL_PITCH;
      setColumnWindow(columnWindowAt(left));
      hostEl.scrollLeft = left;
      updateWindow();
      put = hostEl.scrollLeft;
      return;
    }
    ensureDrawn(anchorIndex());
    const el = anchorEl();
    // Not past the end: a page shorter than the pane from Today on
    // rests on its last row (the browser clamps).
    if (el) hostEl.scrollTop += offsetOf(el);
    updateWindow();
    const again = anchorEl();
    if (again) hostEl.scrollTop += offsetOf(again);
    put = hostEl.scrollTop;
  };
  // A press is the user's scroll: it does not re-arm the opening hold.
  //
  // The scroll is the browser's smooth one, which heads for a fixed
  // scrollTop and is cancelled by any write to it. So everything it will
  // pass is drawn and measured before it starts, and the window holds
  // still until it lands (`flying`): no days drawn or dropped, no hold
  // putting the top row back. A change to the rows mid-flight re-aims it
  // instead.
  //
  // A target that is not drawn is faked near: its days are drawn as a
  // second run (`far`) straight after the days in the pane (before them,
  // when it is above), the days between left out, so the scroll is about
  // a pane over real rows. On landing the true layout goes back in under
  // the hold: the skipped days come back off screen (above the pane after
  // a scroll down, below it after one up), and only the scrollbar moves.
  let flying = false;
  let flightTarget: (() => Element | null) | null = null;
  let landTimer = 0;
  // Back to one run: the one under the top of the pane, which is the far
  // one after a whole flight, and either if the user took over mid-way.
  const land = () => {
    flying = false;
    flightTarget = null;
    clearTimeout(landTimer);
    if (far() && hostEl?.isConnected) {
      const rs = runs();
      const box = hostEl.getBoundingClientRect();
      const at = document.elementFromPoint(box.left + 24, box.top + 12);
      const key = at?.closest<HTMLElement>("[data-group]")?.dataset.group;
      const k = props.groups.findIndex((g) => g.key === key);
      const keep = rs.find((r) => k >= r.start && k <= r.end) ?? rs[rs.length - 1]!;
      setWindow(keep, null);
    }
    updateWindow();
    scheduleRange();
  };
  // `scrollend` is the signal where there is one; a quiet spell is the
  // fallback, and covers a scroll that had nowhere to go.
  const armLanding = () => {
    clearTimeout(landTimer);
    landTimer = window.setTimeout(land, 150);
  };
  onCleanup(() => clearTimeout(landTimer));
  const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)");
  // Where the flight is headed, as the browser will clamp it.
  let aimTop = 0;
  const aim = (el: Element) => {
    const max = hostEl.scrollHeight - hostEl.clientHeight;
    aimTop = Math.max(0, Math.min(max, hostEl.scrollTop + offsetOf(el)));
    hostEl.scrollTo({
      top: aimTop,
      behavior: reducedMotion?.matches ? "instant" : "smooth",
    });
  };
  // Draw day `i` as the far run beside the days in the pane: from `i`
  // on for two panes when it is below, from `i` to the pane's first day
  // (at most a pane of it) when it is above. The window is cut to the
  // pane on that side, so the run starts where the pane ends. A pass
  // again once the run is drawn and measured.
  const fakeNear = (i: number) => {
    const n = props.groups.length;
    const pane = hostEl.clientHeight;
    // One run still (a press lands any flight first), so scrollTop is
    // true page position. The days in the pane: `vs` .. `ve`.
    measureDrawn();
    let at = offsets(props.groups);
    const top = hostEl.scrollTop;
    let vs = 0;
    while (vs < n - 1 && at[vs + 1]! <= top) vs++;
    let ve = vs;
    while (ve < n - 1 && at[ve + 1]! < top + pane) ve++;
    const cur = span();
    for (let pass = 0; pass < 2; pass++) {
      if (pass > 0) {
        measureDrawn();
        at = offsets(props.groups);
      }
      let fe = i;
      if (i > ve) {
        while (fe < n - 1 && at[fe + 1]! < at[i]! + 2 * pane) fe++;
        setWindow({ start: cur.start, end: ve }, { start: i, end: fe });
      } else {
        while (fe < vs - 1 && at[fe + 1]! < at[i]! + pane) fe++;
        setWindow({ start: vs, end: cur.end }, { start: i, end: fe });
      }
    }
  };
  const scrollToGroup = (i: number, find: () => Element | null) => {
    release();
    if (cols) {
      // The column's place is known, and the window follows the scroll
      // as it goes. Smooth over a few panes; a far day is a jump.
      const left = i * AGENDA_COL_PITCH;
      const near = Math.abs(left - hostEl.scrollLeft) <= 3 * hostEl.clientWidth;
      hostEl.scrollTo({
        left,
        behavior: near && !reducedMotion?.matches ? "smooth" : "instant",
      });
      return;
    }
    const { start, end } = span();
    if (i < start || i > end) {
      fakeNear(i);
      const el = find();
      if (!el) return;
      measureDrawn();
      flying = true;
      flightTarget = find;
      armLanding();
      aim(el);
      return;
    }
    let el = find();
    if (!el) return;
    // Draw every day from here to a pane either side of where it lands.
    // Days drawn for the first time measure differently from their
    // estimate, which moves the landing, so a pass or two.
    for (let pass = 0; pass < 4; pass++) {
      measureDrawn();
      const there = windowFor(hostEl.scrollTop + offsetOf(el));
      const cur = span();
      const next = {
        start: Math.min(cur.start, there.start),
        end: Math.max(cur.end, there.end),
      };
      if (!setWindow(next)) break;
      el = find();
      if (!el) return;
    }
    measureDrawn();
    flying = true;
    flightTarget = find;
    armLanding();
    aim(el);
  };
  const scrollToToday = () => scrollToGroup(anchorIndex(), anchorEl);
  // Day keys are `YYYY-MM-DD`, so they order as strings; Overdue's key
  // is not a day and is skipped.
  const dayIndex = (day: string): number => {
    const groups = props.groups;
    const exact = groups.findIndex((g) => g.key === day);
    if (exact >= 0) return exact;
    const next = groups.findIndex((g) => g.key !== OVERDUE_KEY && g.key > day);
    if (next >= 0) return next;
    for (let j = groups.length - 1; j >= 0; j--) {
      if (groups[j]!.key !== OVERDUE_KEY) return j;
    }
    return -1;
  };
  const scrollToDay = (day: string) => {
    const i = dayIndex(day);
    if (i < 0) return;
    const key = props.groups[i]!.key;
    scrollToGroup(i, () =>
      hostEl.querySelector(`[data-group="${CSS.escape(key)}"]`),
    );
  };
  // Where the opening scroll left the pane: a scroll to anywhere else
  // is the user's (the scrollbar, Page Down), and ends it.
  let put = 0;
  // Also the end of a flight: a wheel, touch or click during one is
  // the user taking over.
  const release = () => {
    anchoring = false;
    if (flying) land();
  };
  const onScroll = () => {
    scheduleRange();
    if (flying) {
      armLanding();
      return;
    }
    const at = cols ? hostEl.scrollLeft : hostEl.scrollTop;
    if (anchoring && Math.abs(at - put) > 1) release();
    scheduleWindow();
  };
  onMount(() => {
    // Only at the destination: the instant scrollTop write that put the
    // far run in (the hold) has its own `scrollend`, which arrives after
    // the flight has set off. A flight stopped short lands on the quiet
    // spell instead.
    const onEnd = () => {
      if (flying && Math.abs(hostEl.scrollTop - aimTop) < 2) land();
    };
    hostEl.addEventListener("scrollend", onEnd);
    onCleanup(() => hostEl.removeEventListener("scrollend", onEnd));
  });
  // A computed runs ahead of the render effects that patch the rows in,
  // so the DOM read here is still the old one.
  createComputed(() => {
    void props.groups;
    held = cols || anchoring || flying || !hostEl?.isConnected ? null : topRow();
  });
  // Columns: the day keys as last drawn, to find where the column at the
  // pane's left edge went when the days change.
  let colKeys: string[] = [];
  // The window moves in a microtask: a signal written inside an effect
  // waits for the effect to end before the DOM has it, and the window
  // reads the DOM straight after its writes.
  createEffect(() => {
    void props.groups;
    if (cols) {
      const keys = props.groups.map((g) => g.key);
      const was = colKeys;
      colKeys = keys;
      queueMicrotask(() => {
        if (!hostEl?.isConnected) return;
        if (anchoring) return putAnchorAtTop();
        // A day added or dropped before the pane shifts every column
        // after it by a pitch: scroll by as much.
        const iOld = Math.floor(hostEl.scrollLeft / AGENDA_COL_PITCH);
        const key = was[iOld];
        const iNew = key === undefined ? -1 : keys.indexOf(key);
        if (iNew >= 0 && iNew !== iOld) {
          hostEl.scrollLeft += (iNew - iOld) * AGENDA_COL_PITCH;
        }
        updateWindow();
      });
      return;
    }
    if (anchoring) {
      queueMicrotask(() => {
        if (anchoring) putAnchorAtTop();
      });
      return;
    }
    if (flying) {
      // A far run is indexed into the old groups: give it up.
      queueMicrotask(() => {
        if (!flying) return;
        if (far()) return land();
        const el = flightTarget?.();
        if (el) aim(el);
      });
      return;
    }
    restore(held);
    queueMicrotask(updateWindow);
  });

  // The selected day's row of the item, else its first row; its day is
  // drawn first if it is not.
  const scrollToKey = (id: string) => {
    release();
    const day = indexed.has(id) ? (selDay() ?? undefined) : firstDayOf().get(id);
    let i = day === undefined ? -1 : props.groups.findIndex((g) => g.key === day);
    if (i < 0) {
      i = props.groups.findIndex((g) => g.rows.some((r) => r.item.id === id));
    }
    if (i < 0) return;
    if (cols) {
      const { start, end } = span();
      if (i < start || i > end) {
        setColumnWindow({
          start: Math.max(0, i - 1),
          end: Math.min(props.groups.length - 1, i + 1),
        });
      }
    } else ensureDrawn(i);
    const key = `[data-key="${CSS.escape(id)}"]`;
    const el =
      (day && hostEl.querySelector(`${key}[data-day="${CSS.escape(day)}"]`)) ||
      hostEl.querySelector(key);
    // Columns: down the row's column and sideways to it.
    el?.scrollIntoView({ block: "nearest", inline: "nearest" });
    if (cols) updateWindow();
  };

  props.ref?.({
    focus: () => hostEl.focus({ preventScroll: true }),
    scrollToKey,
    scrollToToday,
    scrollToDay,
  });

  // Autofocus on entry, as the list's Dnd does, so the keys act here
  // straight away whichever way the view was reached (nav click, digit,
  // [ / ], a link). Deferred like the Dnd's, and with the same guard: an
  // editor the user has just landed in keeps focus.
  onMount(() => {
    queueMicrotask(() => {
      const ae = document.activeElement;
      if (
        ae instanceof HTMLElement &&
        (ae.isContentEditable ||
          ae.tagName === "INPUT" ||
          ae.tagName === "TEXTAREA")
      ) {
        return;
      }
      hostEl.focus({ preventScroll: true });
    });
  });

  // Keys land on the host (rows are not focusable), so they bubble up from
  // nothing else. ⌘/Ctrl+↑/↓ reorders a list; the agenda is ordered by
  // date, so it is left alone here.
  // With nothing selected ↓ starts at the top of Overdue / Today, not a
  // year back, and ↑ at the last row above it. ⌘/Ctrl+A selects every
  // row of the selection's day (the selection's order is that day), and
  // with nothing selected does nothing: the agenda reaches a year back,
  // so "all of it" is never what is meant. Swallowed either way so the
  // browser does not select the page's text.
  //
  // Columns: plain ← / → move the selection to the nearest day that way
  // with rows, at the same row index (that day's last row when it is
  // shorter), as the board's lanes do. No wrap. With nothing selected →
  // enters at Overdue / Today's first row and ← at the day before it.
  const stepDay = (dir: -1 | 1) => {
    const groups = props.groups;
    const sel = props.selection.hasSelection();
    const at = sel ? groups.findIndex((g) => g.key === selDay()) : -1;
    const top = sel ? props.selection.getSelectionTop() : null;
    const row =
      at < 0 || top === null
        ? 0
        : Math.max(0, groups[at]!.rows.findIndex((r) => r.item.id === String(top)));
    // From outside, the scan starts on the anchor day (→) or before it.
    const from = at >= 0 ? at : dir === 1 ? anchorIndex() - 1 : anchorIndex();
    const next = neighbourDayRow(
      groups.map((g) => g.rows.length),
      from,
      row,
      dir,
    );
    if (!next) return;
    const g = groups[next.day]!;
    const id = g.rows[next.row]!.item.id;
    release();
    useDay(g.key);
    props.selection.selectOnly(id);
    scrollToKey(id);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (
      cols &&
      (e.key === "ArrowLeft" || e.key === "ArrowRight") &&
      !(e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) &&
      // A row's context menu portals out, but Solid routes its keydown
      // back through here, and ← / → there belong to its submenus.
      !isOverlayOpen()
    ) {
      e.preventDefault();
      stepDay(e.key === "ArrowRight" ? 1 : -1);
      return;
    }
    const action = mapDndKeyEvent(e);
    if (action.type === "ignore" || action.type === "move-selection") return;
    e.preventDefault();
    if (action.type === "select-all") {
      if (props.selection.hasSelection()) props.selection.selectAll();
      return;
    }
    release();
    if (
      (action.type === "navigate" || action.type === "extend") &&
      !props.selection.hasSelection()
    ) {
      const rows = pageRows();
      const past = new Set<string>();
      for (const g of props.groups) if (g.urgency === "past") past.add(g.key);
      const first = rows.findIndex((r) => !past.has(r.day));
      const at =
        action.direction === "down"
          ? first
          : first > 0
            ? first - 1
            : rows.length - 1;
      const row = rows[at];
      if (row === undefined) return;
      useDay(row.day);
      props.selection.selectOnly(row.id);
      scrollToKey(row.id);
      return;
    }
    // Plain ↑ / ↓ go row by row down the page, crossing into the next
    // day; shift ↑ / ↓ extend within the day and stop at its edge.
    if (action.type === "navigate") {
      const down = action.direction === "down";
      const from = down
        ? props.selection.getSelectionBottom()
        : props.selection.getSelectionTop();
      const rows = pageRows();
      const fromId = from === null ? null : String(from);
      const at = rows.findIndex((r) => r.id === fromId && r.day === selDay());
      const next = at < 0 ? undefined : rows[at + (down ? 1 : -1)];
      if (next) {
        useDay(next.day);
        props.selection.selectOnly(next.id);
        scrollToKey(next.id);
      }
      if (at >= 0) return;
    }
    const target = applySelectionKeyAction(props.selection, action, order(), {
      multi: true,
    });
    if (target !== null) scrollToKey(String(target));
  };

  // Columns: a vertical wheel over a day's header scrolls the days
  // sideways, since a mouse wheel in a column only scrolls its rows. A
  // sideways gesture (a trackpad, shift + wheel) is the browser's own,
  // and ctrl + wheel is its zoom.
  const onHeaderWheel = (e: WheelEvent) => {
    if (e.ctrlKey || e.shiftKey) return;
    if (e.deltaY === 0 || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    const t = e.target as Element | null;
    if (!t?.closest(".upcoming-day-header")) return;
    e.preventDefault();
    // Lines (Firefox's mouse wheel) and pages, in px.
    const unit =
      e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? hostEl.clientWidth : 1;
    hostEl.scrollLeft += e.deltaY * unit;
  };
  onMount(() => {
    if (!cols) return;
    // Not passive: it stops the column scrolling its rows.
    hostEl.addEventListener("wheel", onHeaderWheel, { passive: false });
    onCleanup(() => hostEl.removeEventListener("wheel", onHeaderWheel));
  });

  // Mouse mirrors the listbox: click selects, shift extends, ⌘/Ctrl
  // toggles, a click on no row clears (the list's `clearOnBlankClick`).
  // The checkbox is its own control. On touch a tap opens instead.
  const rowFromEvent = (e: MouseEvent): HTMLElement | null => {
    const t = e.target as HTMLElement | null;
    const row = t?.closest<HTMLElement>("[data-key]");
    return row && hostEl.contains(row) ? row : null;
  };
  const rowIdFromEvent = (e: MouseEvent): string | null =>
    rowFromEvent(e)?.dataset.key ?? null;
  // Drag to reschedule (`spec/calendar-plan.md` "Reschedule"): a mouse
  // press on a row that moves past a few px picks up the selection (the
  // row is selected first if it is not), the list view's overlay stack
  // follows the pointer, and the day section under it is the target
  // (any day, past ones included; not Overdue). The drop writes
  // `dropOnDay` for each item as one undo step. Escape cancels. Mouse
  // only.
  const DRAG_START_PX = 4;
  const overlay = new DndDragOverlay(3);
  let autoscroll: DndAutoscroll | null = null;
  let press: { x: number; y: number; id: string; day: string; el: HTMLElement } | null =
    null;
  let dragIds: string[] | null = null;
  let lastPointer = { x: 0, y: 0 };
  let suppressClick = false;
  const [dropDay, setDropDay] = createSignal<string | null>(null);
  const [dragging, setDragging] = createSignal<ReadonlySet<string>>(new Set());

  const dayUnder = (x: number, y: number): string | null => {
    const at = document.elementFromPoint(x, y);
    if (!at || !hostEl.contains(at)) return null;
    const key = at.closest<HTMLElement>("[data-group]")?.dataset.group;
    return key && key !== OVERDUE_KEY ? key : null;
  };
  const onRowMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || e.shiftKey || (isMac ? e.metaKey : e.ctrlKey)) return;
    if (props.openOnTap?.()) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest("input, button, a")) return;
    const row = rowFromEvent(e);
    // A later day of a span is not a handle, whatever else is selected.
    if (!row || row.dataset.spanTail !== undefined) return;
    press = {
      x: e.clientX,
      y: e.clientY,
      id: row.dataset.key!,
      day: row.dataset.day!,
      el: row,
    };
    document.addEventListener("mousemove", onDocMouseMove);
    document.addEventListener("mouseup", onDocMouseUp);
  };
  const startDrag = () => {
    const p = press!;
    if (p.day !== selDay() || !props.selection.getSelectedKeySet().has(p.id)) {
      useDay(p.day);
      props.selection.selectOnly(p.id);
    }
    // The selection's rows in the grabbed day, less its span tails: a
    // later day of a span stays put when its neighbours are dragged.
    const rowsHere = new Map(
      (props.groups.find((g) => g.key === p.day)?.rows ?? []).map((r) => [
        r.item.id,
        r,
      ]),
    );
    const ids = props.selection.getSelectedKeys().map(String).filter((id) => {
      const r = rowsHere.get(id);
      return r !== undefined && !isSpanTail(r, p.day);
    });
    dragIds = ids;
    setDragging(new Set(ids));
    release();
    window.getSelection()?.removeAllRanges();
    // The grabbed row leads the stack, then the other dragged rows of the
    // day.
    const els: HTMLElement[] = [p.el];
    const dayAttr = `[data-day="${CSS.escape(p.day)}"]`;
    for (const id of ids) {
      if (id === p.id) continue;
      const el = hostEl.querySelector<HTMLElement>(
        `[data-key="${CSS.escape(id)}"]${dayAttr}`,
      );
      if (el) els.push(el);
    }
    overlay.start(
      els,
      ids.length,
      p.x,
      p.y,
      p.el.offsetHeight,
      p.el.offsetWidth,
      p.el,
      0,
      4,
    );
    autoscroll = new DndAutoscroll(hostEl, 48, false, undefined, undefined, cols ? "x" : "y");
    document.addEventListener("keydown", onDragKeyDown, true);
  };
  const onDocMouseMove = (e: MouseEvent) => {
    if (!press) return;
    if (!dragIds) {
      if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < DRAG_START_PX) {
        return;
      }
      startDrag();
    }
    e.preventDefault();
    lastPointer = { x: e.clientX, y: e.clientY };
    overlay.updatePosition(e.clientX, e.clientY);
    autoscroll?.update(e.clientX, e.clientY);
    setDropDay(dayUnder(e.clientX, e.clientY));
  };
  const endDrag = (commit: boolean) => {
    document.removeEventListener("mousemove", onDocMouseMove);
    document.removeEventListener("mouseup", onDocMouseUp);
    document.removeEventListener("keydown", onDragKeyDown, true);
    const ids = dragIds;
    const day = dropDay();
    press = null;
    dragIds = null;
    overlay.stop();
    autoscroll?.stop();
    autoscroll = null;
    setDropDay(null);
    setDragging(new Set<string>());
    if (!ids) return;
    // The click that follows the mouseup is the drag's, not a select.
    suppressClick = true;
    setTimeout(() => (suppressClick = false), 0);
    if (!commit || day === null) return;
    const writes = ids.flatMap((id) => {
      const it = flat().find((r) => r.item.id === id)?.item;
      const when = it ? dropOnDay(it, day) : null;
      return when ? [{ id, when }] : [];
    });
    if (writes.length === 0) return;
    // The selection follows the moved items to the day they land on;
    // one Solid batch, so the rows change once with every item moved.
    landingDay = day;
    batch(() =>
      props.app.withActionBatch(() => {
        for (const w of writes) props.app.setItemWhen(w.id, w.when);
      }),
    );
  };
  const onDocMouseUp = () => endDrag(true);
  const onDragKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    endDrag(false);
  };
  // Autoscroll moves the days under a still pointer.
  const onDragScroll = () => {
    if (dragIds) setDropDay(dayUnder(lastPointer.x, lastPointer.y));
  };
  onCleanup(() => {
    if (press || dragIds) endDrag(false);
  });

  const onClick = (e: MouseEvent) => {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    const t = e.target as HTMLElement | null;
    if (t?.closest("input")) return;
    // A day header's "+" (here, so the host's click handling runs once).
    const addDay = t?.closest<HTMLElement>("[data-add-day]")?.dataset.addDay;
    if (addDay) {
      props.onAddOnDay?.(addDay);
      return;
    }
    const row = rowFromEvent(e);
    const id = row?.dataset.key ?? null;
    if (id === null) {
      if (props.selection.hasSelection()) props.selection.clear();
      return;
    }
    if (props.openOnTap?.()) {
      props.onOpen(id);
      return;
    }
    const mod = isMac ? e.metaKey : e.ctrlKey;
    // Shift and ⌘ build on the selection within its day; on another day
    // they start afresh there, as a plain click does.
    const day = row!.dataset.day!;
    if (day !== selDay() || !props.selection.hasSelection()) {
      useDay(day);
      props.selection.selectOnly(id);
    } else if (e.shiftKey) props.selection.extendActive(id);
    else if (mod) props.selection.toggleItem(id);
    else props.selection.selectOnly(id);
  };
  const onDblClick = (e: MouseEvent) => {
    const t = e.target as HTMLElement | null;
    if (t?.closest("input")) return;
    const id = rowIdFromEvent(e);
    if (id !== null) props.onOpen(id);
  };

  // A day takes an Add from today on: Overdue has no day to plan
  // onto, and a day gone by is not one to plan.
  const canAdd = (g: DayGroup) =>
    g.urgency === "today" || g.urgency === "future";
  // Every day heads with the same long-form date ("Sat 24 Sept"); Today
  // and Tomorrow lead with their name ("Today - Sat 24 Sept") so the eye
  // lands on them without the other days changing shape as they
  // approach. Overdue has no day: its rows each carry their own date.
  const dayHeading = (g: DayGroup): string => {
    if (g.urgency === "overdue") return g.label;
    const [y, mo, d] = g.key.split("-").map(Number);
    if (!y || !mo || !d) return g.label;
    const today = todayStamp(nowMs());
    // The year once it is not this one: the past reaches a year back.
    const date = new Intl.DateTimeFormat(locale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
      year: g.key.slice(0, 4) === today.slice(0, 4) ? undefined : "numeric",
    }).format(new Date(y, mo - 1, d));
    if (g.key === today) return `${m().calendar.today} - ${date}`;
    if (g.key === addDaysToStamp(today, 1)) {
      return `${m().calendar.tomorrow} - ${date}`;
    }
    return date;
  };

  // The end clock of a timed span, on the row of its last day only.
  const spanEnd = (r: DayRow): string | null => {
    if (!r.span || r.span.day !== r.span.total || !r.item.duration) return null;
    const start = whenTime(r.item.when!);
    if (!start) return null;
    return formatClockTime(endTimeOf(start, r.item.duration), locale());
  };

  // Start time only: the agenda is a glance at the day, the end sits on
  // the task surface. An all-day event says so in the lane; a task with
  // no time says nothing (it is for the day, not something that fills
  // it). A multi-day span
  // reads by what the day holds of it: every day of an all-day span is
  // "All day"; a timed one shows its start clock on the first day, "All
  // day" on the days it runs clean through, and its end clock on the last
  // ("until 03:00"); again a task's lane stays empty where an event's
  // would say "All day". Which
  // day of the span a row is sits on the sub line (`spanDay`), not here.
  // A task in Overdue by its `when` shows the day it was planned for
  // ("7 Sept"): Overdue has no day header to carry it.
  const timeLabel = (r: DayRow): string => {
    if (r.tone === "overdue") {
      return (
        formatDayLabel(
          whenDay(r.item.when!),
          todayStamp(nowMs()),
          m().calendar,
          locale(),
        ) ?? ""
      );
    }
    const allDay = isEvent(r.item) ? m().when.allDay : "";
    const start = whenTime(r.item.when!);
    if (!start) return allDay;
    if (r.span && r.span.day > 1) {
      const end = spanEnd(r);
      return end ? m().upcoming.spanUntil(end) : allDay;
    }
    return formatClockTime(start, locale());
  };

  // "Day 2" under the title of every row of a multi-day span, the first
  // day included. Not in Overdue, where a span is one row, and not on a
  // timed span under 24 hours: an evening that runs past midnight is one
  // occasion, and its rows already read "22:00" and "until 03:00".
  const spanDayLabel = (r: DayRow): string | null => {
    if (!r.span || r.tone === "overdue") return null;
    if (whenTime(r.item.when!) && (r.item.duration ?? 0) < 1440) return null;
    return m().upcoming.spanDay(r.span.day);
  };

  // "in 2h 15m" under a timed row's time later today: how soon it
  // starts, read off the shared minute tick so it counts down in place. Only today's
  // open rows with a start still ahead; the minute is rounded up so a
  // row never reads "in 0m" before its time.
  const startsIn = (r: DayRow): string | null => {
    if (isClosed(r.item)) return null;
    const start = whenStartMs(r.item.when!);
    if (start === null) return null;
    const now = nowMs();
    if (todayStamp(now) !== todayStamp(start)) return null;
    const diff = start - now;
    if (diff <= 0) return null;
    return m().upcoming.startsIn(formatDurationShort(Math.ceil(diff / 60_000)));
  };

  return (
    <div
      ref={hostEl}
      class="upcoming"
      data-layout={cols ? "columns" : undefined}
      style={
        cols
          ? {
              "--agenda-col-w": `${AGENDA_COL_WIDTH}px`,
              "--agenda-col-gap": `${AGENDA_COL_GAP}px`,
            }
          : undefined
      }
      role="listbox"
      aria-multiselectable="true"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onClick={onClick}
      onDblClick={onDblClick}
      onScroll={() => {
        onScroll();
        onDragScroll();
      }}
      onMouseDown={onRowMouseDown}
      onWheel={release}
      onTouchMove={release}
      onPointerDown={release}
    >
      <div
        class="upcoming-spacer"
        style={cols ? { width: `${spacers().top}px` } : { height: `${spacers().top}px` }}
      />
      <For each={shown()}>
        {(g) => (
          <section
            class="upcoming-day"
            role="group"
            aria-label={dayHeading(g)}
            data-urgency={g.urgency}
            data-group={g.key}
            data-drop-target={dropDay() === g.key ? "" : undefined}
          >
            <header class="upcoming-day-header">
              <h2 class="upcoming-day-label">{dayHeading(g)}</h2>
              <Show when={props.onAddOnDay && canAdd(g)}>
                <button
                  type="button"
                  class="upcoming-day-add"
                  tabIndex={-1}
                  data-add-day={g.key}
                  aria-label={m().upcoming.addOnDay(dayHeading(g))}
                  title={m().common.add}
                  innerHTML={plusSvg}
                />
              </Show>
              <Show when={weeks().get(g.key)}>
                {(n) => (
                  <span class="upcoming-week-label">
                    {m().upcoming.week(n())}
                  </span>
                )}
              </Show>
            </header>
            <For each={g.rows}>
                {(r) => {
                  const selected = createMemo(() => isLit(r.item.id, g.key));
                  // Starred (pinned to Focus): a star follows the title.
                  const starred = createMemo(() =>
                    props.app.isFocused(r.item.id),
                  );
                  const assignee = () =>
                    isClosed(r.item) ? null : (props.assigneeOf?.(r.item) ?? null);
                  return (
                  <ItemContextMenu
                    item={() => r.item}
                    app={props.app}
                    selection={props.selection}
                    viewKind="upcoming"
                    duplicateBlock={props.duplicateBlock}
                    copyBlock={props.copyBlock}
                    onOpen={props.onOpen}
                    onReveal={props.onReveal}
                    onMoveToList={props.onMoveToList}
                    listLabel={props.listLabel}
                  >
                  <ContextMenu.Trigger
                    class="upcoming-row"
                    role="option"
                    data-key={r.item.id}
                    data-day={g.key}
                    aria-selected={selected() ? "true" : "false"}
                    data-tone={r.tone}
                    data-dragging={dragging().has(r.item.id) ? "" : undefined}
                    data-span-tail={isSpanTail(r, g.key) ? "" : undefined}
                    // `data-done`, not `data-closed`: the menu trigger
                    // sets its own `data-closed` while the menu is shut.
                    data-done={isClosed(r.item) ? "" : undefined}
                  >
                    {/* The check leads the row; the fixed-width time
                        lane sits in source order here but CSS `order`
                        moves it to the row's right edge, kept even
                        when empty so the times line up down the day. */}
                    <span class="upcoming-row-check">
                      {/* An event's own row has no box while it is
                          open (nothing asks for the tick) or once its
                          day has gone by (it is a record of what was
                          on): an upright pill holds the slot, or a
                          bare cross on a cancelled one. Done or
                          cancelled from today on, it shows the box,
                          ticked or crossed, and unticks as usual. */}
                      <Show
                        when={
                          !isEvent(r.item) ||
                          (isClosed(r.item) && g.urgency !== "past")
                        }
                        fallback={
                          <span
                            class="upcoming-row-event-mark"
                            aria-hidden="true"
                            innerHTML={isCancelled(r.item) ? crossSvg : EVENT_PILL}
                          />
                        }
                      >
                        <input
                          type="checkbox"
                          class="task-check"
                          checked={isClosed(r.item)}
                          data-cancelled={isCancelled(r.item) ? "" : undefined}
                          aria-label={m().workspace.markDone}
                          onPointerDown={(e) => pressCheck(e.currentTarget)}
                          onChange={(e) =>
                            props.app.setDone(r.item.id, e.currentTarget.checked)
                          }
                        />
                      </Show>
                    </span>
                    <span
                      class="upcoming-row-time"
                      data-empty={timeLabel(r) === "" ? "" : undefined}
                    >
                      <span class="upcoming-row-clock">{timeLabel(r)}</span>
                      <Show when={startsIn(r)}>
                        {(label) => (
                          <span class="upcoming-row-soon">{label()}</span>
                        )}
                      </Show>
                    </span>
                    <span class="upcoming-row-body">
                      <span class="upcoming-row-title">
                        <span class="upcoming-row-text">{r.item.text}</span>
                        <Show when={starred()}>
                          <span
                            class="upcoming-row-star"
                            title={m().focus.badge}
                            aria-label={m().focus.badge}
                            innerHTML={starFilledSvg}
                          />
                        </Show>
                      </span>
                      {/* Sub line under the title: which day of a
                          multi-day span this row is ("Day 2"), the shared
                          workspace a foreign row comes from (people glyph
                          + name), the list the item is in, then the place.
                          Home rows carry no workspace marker, and an item
                          with no list (it lives on the calendar alone)
                          no list. An open item's assignee leads the
                          line as a small disc (shared workspaces only,
                          as list rows). */}
                      <Show
                        when={
                          assignee() ||
                          spanDayLabel(r) ||
                          props.workspaceLabel(r.item.docId) ||
                          r.item.listId !== "inbox" ||
                          r.item.place
                        }
                      >
                        <span class="upcoming-row-sub">
                          <Show when={assignee()}>
                            {(a) => (
                              <Avatar
                                accountId={a().accountId}
                                email={a().email}
                                title={m().workspace.assignedTo(
                                  a().email ?? m().sharing.formerMember,
                                )}
                              />
                            )}
                          </Show>
                          <Show when={spanDayLabel(r)}>
                            {(label) => (
                              <span class="upcoming-row-place upcoming-row-span">
                                {label()}
                              </span>
                            )}
                          </Show>
                          <Show when={props.workspaceLabel(r.item.docId)}>
                            {(name) => (
                              <span class="upcoming-row-place upcoming-row-workspace" title={name()}>
                                <span
                                  class="upcoming-row-place-icon"
                                  innerHTML={usersSvg}
                                />
                                <span class="upcoming-row-place-label">{name()}</span>
                              </span>
                            )}
                          </Show>
                          <Show
                            when={
                              r.item.listId !== "inbox" &&
                              props.listLabel(r.item.listId, r.item.docId)
                            }
                          >
                            {(name) => (
                              <span class="upcoming-row-place" title={name()}>
                                {/* The nav's glyphs: the list's chosen
                                    emoji, else the default file mark. */}
                                <Show
                                  when={props.listIcon(r.item.listId, r.item.docId)}
                                  fallback={
                                    <span
                                      class="upcoming-row-place-icon"
                                      innerHTML={fileSvg}
                                    />
                                  }
                                >
                                  {(icon) => (
                                    <span class="upcoming-row-place-icon upcoming-row-list-emoji">
                                      {icon()}
                                    </span>
                                  )}
                                </Show>
                                <span class="upcoming-row-place-label">{name()}</span>
                              </span>
                            )}
                          </Show>
                          <Show when={r.item.place}>
                            {(p) => (
                              <span
                                class="upcoming-row-place"
                                title={p().address ?? p().label}
                              >
                                <span
                                  class="upcoming-row-place-icon"
                                  innerHTML={mapPinSvg}
                                />
                                <span class="upcoming-row-place-label">
                                  {p().label}
                                </span>
                              </span>
                            )}
                          </Show>
                        </span>
                      </Show>
                    </span>
                  </ContextMenu.Trigger>
                  </ItemContextMenu>
                  );
                }}
            </For>
            {/* An empty day keeps a blank row-height slot, room to drop
                a dragged row onto (`data-drop-day`); a day with rows has
                them to drop onto. It says the day is empty ("Nothing
                scheduled", Today's "Nothing due today"). Overdue is never empty. */}
            <Show
              keyed
              when={props.draft?.()?.day === g.key ? props.draft!()!.key : null}
            >
              {(_key) => (
                <AgendaDraftRow
                  placeholder={m().workspace.newItemPlaceholder}
                  onInput={(t) => props.onDraftInput?.(t)}
                  onSettle={(t, how) => props.onDraftSettle?.(t, how)}
                />
              )}
            </Show>
            <Show
              when={
                g.rows.length === 0 &&
                g.urgency !== "overdue" &&
                props.draft?.()?.day !== g.key
              }
            >
              <div
                class="upcoming-day-slot"
                data-drop-day={g.key}
                aria-hidden="true"
              >
                <span class="upcoming-empty-day">
                  {g.urgency === "today"
                    ? m().upcoming.emptyToday
                    : m().upcoming.emptyDay}
                </span>
              </div>
            </Show>
          </section>
        )}
      </For>
      <div
        class="upcoming-spacer"
        style={
          cols ? { width: `${spacers().bottom}px` } : { height: `${spacers().bottom}px` }
        }
      />
    </div>
  );
}

/** The agenda's draft row: a plain text input laid out as a task row
 *  (an empty box in the check lane, the title in the body), at the end
 *  of its day. It takes the caret on mount and settles exactly once:
 *  Enter, Escape or ↑ from the keyboard, or focus leaving it (which
 *  includes the owner ending it from outside, a pane handoff, after
 *  which the late blur is ignored). Keys stop here, so the agenda's
 *  listbox and the global shortcuts never see them. */
function AgendaDraftRow(props: {
  placeholder: string;
  onInput: (text: string) => void;
  onSettle: (text: string, how: "enter" | "escape" | "blur") => void;
}) {
  let input!: HTMLInputElement;
  let settled = false;
  const settle = (how: "enter" | "escape" | "blur") => {
    if (settled) return;
    settled = true;
    props.onSettle(input.value.trim(), how);
  };
  onMount(() => {
    input.focus({ preventScroll: true });
    input.closest(".upcoming-row")?.scrollIntoView({ block: "nearest" });
  });
  onCleanup(() => {
    settled = true;
  });
  return (
    <div class="upcoming-row upcoming-draft-row">
      <span class="upcoming-row-check">
        <input type="checkbox" class="task-check" disabled tabIndex={-1} aria-hidden="true" />
      </span>
      <span class="upcoming-row-time" data-empty="" />
      <span class="upcoming-row-body">
        <span class="upcoming-row-title">
          <input
            ref={input}
            type="text"
            class="upcoming-draft-input"
            placeholder={props.placeholder}
            aria-label={props.placeholder}
            onInput={(e) => props.onInput(e.currentTarget.value)}
            on:blur={() => settle("blur")}
            on:keydown={(e) => {
              e.stopPropagation();
              if (e.isComposing) return;
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                settle("enter");
              } else if (e.key === "Escape" || e.key === "ArrowUp") {
                e.preventDefault();
                settle("escape");
              }
            }}
          />
        </span>
      </span>
    </div>
  );
}
