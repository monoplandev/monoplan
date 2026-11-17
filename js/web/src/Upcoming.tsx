// The Upcoming view: the agenda. Every Open item with a planned date or a
// deadline, bucketed by day (`dayGroups.ts`, grouped by the Workspace so
// the same rows feed its selection-scoped shortcuts), Today always
// anchored so the surface reads as "the days ahead" even when nothing is
// due. Overdue deadlines sit in an Overdue section above Today, and so
// does an Open task whose `when` has passed; an event's past `when` is
// over and places nothing here. Rows carry no date pills: the
// day header is the date, and the task surface shows the rest. The
// checkbox leads each row, then a fixed-width lane carries the start time
// ("All day" on an event with no time, nothing on a task with none,
// "Due" on a deadline, or the planned date on a task that
// slipped into Overdue, which has no day header to say it); the place is
// a muted line under the title.
// A deadline is a special kind of date here: its own row on the day it
// is owed, a timer glyph and "Due" in the time lane, beside whatever row
// the item's `when` gives it on another day. Where both fall on one day
// the `when` row stands for both and carries the "Due" mark after its
// title.
// Rows tick off in place and stay where they are on their `when` day once
// done or cancelled, muted (tick or cross), so the day still records what
// happened and what was called off.
//
// Events (`spec/events-plan.md`) live here and nowhere else. An event row
// looks like a task row: the checkbox is there for anyone who wants the
// record, but nothing asks for it, the event just goes by. No row carries
// its list label; the context menu and task surface name it.
//
// Selection and keys: the agenda is a listbox in the list view's sense
// (click selects, the side pane follows, Enter / double-click opens, the
// item shortcuts act on the selection) but not a `Dnd`: day sections are
// the point, rows are not one height, and the flat virtualised list can't
// host group headers. So it runs the Workspace's `DndSelection` over the
// rows of every day concatenated, and answers the same keys through the
// shared `applySelectionKeyAction`, which makes ↑ / ↓ cross day boundaries
// for free. Nothing is virtualised. Drag-to-reschedule is deferred
// (`spec/calendar-plan.md`).
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

import {
  createComputed,
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import { ItemContextMenu } from "./ItemContextMenu.tsx";
import { RowMenuButton } from "./Row.tsx";
import {
  agendaItems,
  isDueRow,
  type DayGroup,
  type DayRow,
} from "./dayGroups.ts";
import {
  applySelectionKeyAction,
  mapDndKeyEvent,
  type DndSelection,
} from "./dnd";
import {
  addDaysToStamp,
  endTimeOf,
  formatClockTime,
  formatDeadlineBadge,
  formatDurationShort,
  nowMs,
  todayStamp,
  whenDay,
  whenStartMs,
  whenTime,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import mapPinSvg from "./icons/map-pin.svg?raw";
import starFilledSvg from "./icons/star-filled.svg?raw";
import timerSvg from "./icons/timer.svg?raw";
import usersSvg from "./icons/users.svg?raw";
import {
  isCancelled,
  isClosed,
  isEvent,
  type ItemActions,
} from "./sync/store.ts";

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
}

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
  /** The agenda merges every workspace, so a row from a shared one
   *  (anything but Home) names it under the title with the shared
   *  people glyph; null for Home rows, which carry no marker. */
  workspaceLabel: (docId: string) => string | null;
  onOpen: (id: string) => void;
  /** Touch: a tap opens the row. */
  openOnTap?: () => boolean;
  ref?: (handle: UpcomingImperative) => void;
  /** Context menu wiring, as the list rows take it (`ItemContextMenu`). */
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  onReveal?: (id: string, where: "list" | "focus") => void;
  onMoveToList?: (ids: readonly string[]) => void;
  /** Open the item on its deadline band, calendar showing: the row
   *  menu's "Set deadline…". */
  onSetDeadline?: (id: string) => void;
}) {
  const { m, locale } = useAppI18n();

  let hostEl!: HTMLDivElement;

  // The selection's order is every day's rows in display order, each id
  // once (`agendaItems`): a multi-day span repeats its item on each day
  // it covers (`DayRow.span`), and selecting any of those rows selects
  // the item. Fresh array per change: `updateOrder` is identity-guarded.
  const flat = createMemo(() => agendaItems(props.groups));
  const order = createMemo<string[]>(() => flat().map((r) => r.item.id));
  createEffect(() => props.selection.updateOrder(order()));

  // Selected set as a signal so only rows whose membership flips re-render
  // their chrome. `equals: false`: the selection mutates in place.
  const [selTick, setSelTick] = createSignal(0, { equals: false });
  onCleanup(props.selection.onChange(() => setSelTick((n) => n + 1)));
  const selectedSet = createMemo(() => {
    selTick();
    return props.selection.getSelectedKeySet();
  });

  // Which row of an item on several days (a multi-day span, a date and
  // a deadline) draws the selection: the one the pointer picked, else
  // the item's place in the order (its earliest day from today on, or
  // in the past when it is only there). The selection itself is by item
  // id (one item, however many rows it has), so this is chrome only; a
  // row is lit when its item is selected and it is on that item's chosen
  // day.
  const [pickedDay, setPickedDay] = createSignal<Map<string, string>>(
    new Map(),
  );
  const firstDayOf = createMemo(
    () => new Map(flat().map((r) => [r.item.id, r.group])),
  );
  const litDay = (id: string): string | undefined =>
    pickedDay().get(id) ?? firstDayOf().get(id);
  // Every row on the page in display order, an item's several rows
  // included: what plain ↑ / ↓ step through.
  const pageRows = createMemo(() =>
    props.groups.flatMap((g) =>
      g.rows.map((r) => ({ id: r.item.id, day: g.key })),
    ),
  );
  // A picked row only lives while its item stays selected; other paths
  // fall back to the first copy.
  createEffect(() => {
    const sel = selectedSet();
    const picked = pickedDay();
    if ([...picked.keys()].every((id) => sel.has(id))) return;
    const next = new Map(picked);
    for (const id of picked.keys()) if (!sel.has(id)) next.delete(id);
    setPickedDay(next);
  });

  // Opening on Overdue / Today, and holding the page still under a
  // change (see the header comment).
  let anchoring = true;
  const offsetOf = (el: Element): number =>
    el.getBoundingClientRect().top - hostEl.getBoundingClientRect().top;
  // Overdue comes first in the DOM when it is there.
  const anchorEl = () =>
    hostEl.querySelector('[data-urgency="overdue"], [data-urgency="today"]');
  const putAnchorAtTop = () => {
    const el = anchorEl();
    // Not past the end: a page shorter than the pane from Today on
    // rests on its last row (the browser clamps).
    if (el) hostEl.scrollTop += offsetOf(el);
    put = hostEl.scrollTop;
  };
  // A press is the user's scroll: it does not re-arm the opening hold.
  const scrollToToday = () => {
    release();
    const el = anchorEl();
    if (!el) return;
    hostEl.scrollTo({
      top: hostEl.scrollTop + offsetOf(el),
      behavior: "smooth",
    });
  };
  // Where the opening scroll left the pane: a scroll to anywhere else
  // is the user's (the scrollbar, Page Down), and ends it.
  let put = 0;
  const release = () => {
    anchoring = false;
  };
  const onScroll = () => {
    if (anchoring && Math.abs(hostEl.scrollTop - put) > 1) release();
  };
  // The row (or day header) at the top of the pane and where it sits,
  // read before a change reaches the DOM and put back after it.
  let held: { selector: string; offset: number } | null = null;
  const topRow = (): typeof held => {
    const box = hostEl.getBoundingClientRect();
    const at = document.elementFromPoint(box.left + 24, box.top + 2);
    if (!at || !hostEl.contains(at)) return null;
    const row = at.closest<HTMLElement>("[data-key]");
    const day = at.closest<HTMLElement>("[data-group]");
    if (!day || !hostEl.contains(day)) return null;
    const selector = row
      ? `[data-key="${CSS.escape(row.dataset.key!)}"][data-day="${CSS.escape(row.dataset.day!)}"]`
      : `[data-group="${CSS.escape(day.dataset.group!)}"]`;
    return { selector, offset: offsetOf(row ?? day) };
  };
  // A computed runs ahead of the render effects that patch the rows in,
  // so the DOM read here is still the old one.
  createComputed(() => {
    void props.groups;
    held = anchoring || !hostEl?.isConnected ? null : topRow();
  });
  createEffect(() => {
    void props.groups;
    if (anchoring) {
      putAnchorAtTop();
      return;
    }
    const el = held && hostEl.querySelector(held.selector);
    if (!el) return;
    const moved = offsetOf(el) - held!.offset;
    if (Math.abs(moved) >= 1) hostEl.scrollTop += moved;
  });

  // The copy that draws the selection (`litDay`), else the first row
  // carrying the id in DOM order.
  const scrollToKey = (id: string) => {
    release();
    const key = `[data-key="${CSS.escape(id)}"]`;
    const day = litDay(id);
    const el =
      (day && hostEl.querySelector(`${key}[data-day="${CSS.escape(day)}"]`)) ||
      hostEl.querySelector(key);
    el?.scrollIntoView({ block: "nearest" });
  };

  props.ref?.({
    focus: () => hostEl.focus({ preventScroll: true }),
    scrollToKey,
    scrollToToday,
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
  // year back, and ↑ at the last row above it.
  const onKeyDown = (e: KeyboardEvent) => {
    const action = mapDndKeyEvent(e);
    if (action.type === "ignore" || action.type === "move-selection") return;
    e.preventDefault();
    release();
    if (
      (action.type === "navigate" || action.type === "extend") &&
      !props.selection.hasSelection()
    ) {
      const rows = flat();
      const past = new Set<string>();
      for (const g of props.groups) if (g.urgency === "past") past.add(g.key);
      const first = rows.findIndex((r) => !past.has(r.group));
      const at =
        action.direction === "down"
          ? first
          : first > 0
            ? first - 1
            : rows.length - 1;
      const id = rows[at]?.item.id;
      if (id === undefined) return;
      props.selection.selectOnly(id);
      scrollToKey(id);
      return;
    }
    // Plain ↑ / ↓ go row by row, so an item's second row (its deadline,
    // a later day of its span) is a stop like any other: the selection
    // is by item id, which alone would step straight past it. Shift
    // ranges stay by item.
    if (action.type === "navigate") {
      const down = action.direction === "down";
      const from = down
        ? props.selection.getSelectionBottom()
        : props.selection.getSelectionTop();
      const rows = pageRows();
      const fromId = from === null ? null : String(from);
      const day = fromId === null ? undefined : litDay(fromId);
      let at = rows.findIndex((r) => r.id === fromId && r.day === day);
      if (at < 0) at = rows.findIndex((r) => r.id === fromId);
      const next = at < 0 ? undefined : rows[at + (down ? 1 : -1)];
      if (next) {
        props.selection.selectOnly(next.id);
        setPickedDay(new Map([[next.id, next.day]]));
        scrollToKey(next.id);
      }
      if (at >= 0) return;
    }
    const target = applySelectionKeyAction(props.selection, action, order(), {
      multi: true,
    });
    if (target !== null) scrollToKey(String(target));
  };

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
  const onClick = (e: MouseEvent) => {
    const t = e.target as HTMLElement | null;
    if (t?.closest("input")) return;
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
    if (e.shiftKey) props.selection.extendActive(id);
    else if (mod) props.selection.toggleItem(id);
    else props.selection.selectOnly(id);
    const day = row?.dataset.day;
    if (day && props.selection.getSelectedKeySet().has(id)) {
      setPickedDay((prev) => new Map(prev).set(id, day));
    }
  };
  const onDblClick = (e: MouseEvent) => {
    const t = e.target as HTMLElement | null;
    if (t?.closest("input")) return;
    const id = rowIdFromEvent(e);
    if (id !== null) props.onOpen(id);
  };

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
    if (g.key === today) return `${m().deadline.today} - ${date}`;
    if (g.key === addDaysToStamp(today, 1)) {
      return `${m().deadline.tomorrow} - ${date}`;
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
  // it), and a row here on its deadline says "Due". A multi-day span
  // reads by what the day holds of it: every day of an all-day span is
  // "All day"; a timed one shows its start clock on the first day, "All
  // day" on the days it runs clean through, and its end clock on the last
  // ("until 03:00"); again a task's lane stays empty where an event's
  // would say "All day". Which
  // day of the span a row is sits on the sub line (`spanDay`), not here.
  // A task in Overdue by its `when` shows the day it was planned for
  // ("7 Sept"): Overdue has no day header to carry it.
  const timeLabel = (r: DayRow): string => {
    if (r.placedBy === "deadline") return m().upcoming.due;
    if (r.placedBy !== "when") return "";
    if (r.tone === "overdue") {
      const info = formatDeadlineBadge(
        whenDay(r.item.when!),
        todayStamp(nowMs()),
        m().deadline,
        locale(),
        { pastAsDate: true },
      );
      return info?.label ?? "";
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
    if (!r.span || r.placedBy !== "when" || r.tone === "overdue") return null;
    if (whenTime(r.item.when!) && (r.item.duration ?? 0) < 1440) return null;
    return m().upcoming.spanDay(r.span.day);
  };

  // "in 2h 15m" beside a timed row later today: how soon it starts, read
  // off the shared minute tick so it counts down in place. Only today's
  // open rows with a start still ahead; the minute is rounded up so a
  // row never reads "in 0m" before its time.
  const startsIn = (r: DayRow): string | null => {
    if (r.placedBy !== "when" || isClosed(r.item)) return null;
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
      role="listbox"
      aria-multiselectable="true"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onClick={onClick}
      onDblClick={onDblClick}
      onScroll={onScroll}
      onWheel={release}
      onTouchMove={release}
      onPointerDown={release}
    >
      <For each={props.groups}>
        {(g) => (
          <section
            class="upcoming-day"
            role="group"
            aria-label={dayHeading(g)}
            data-urgency={g.urgency}
            data-group={g.key}
          >
            <header class="upcoming-day-header">
              <h2 class="upcoming-day-label">{dayHeading(g)}</h2>
            </header>
            <Show
              when={g.rows.length > 0}
              fallback={
                <div class="upcoming-empty-day">{m().upcoming.emptyToday}</div>
              }
            >
              <For each={g.rows}>
                {(r) => {
                  const selected = createMemo(
                    () =>
                      selectedSet().has(r.item.id) &&
                      litDay(r.item.id) === g.key,
                  );
                  // Keeps the "…" button showing while its menu is up
                  // (`.upcoming-row[data-menu-open]`), as list rows do.
                  const [menuOpen, setMenuOpen] = createSignal(false);
                  // Starred (pinned to Focus): a star follows the title.
                  const starred = createMemo(() =>
                    props.app.isFocused(r.item.id),
                  );
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
                    onSetDeadline={props.onSetDeadline}
                    listLabel={props.listLabel}
                    onMenuOpenChange={setMenuOpen}
                  >
                  <ContextMenu.Trigger
                    class="upcoming-row"
                    role="option"
                    data-key={r.item.id}
                    data-day={g.key}
                    data-menu-open={menuOpen() ? "" : undefined}
                    aria-selected={selected() ? "true" : "false"}
                    data-tone={r.tone}
                    data-placed={r.placedBy}
                    // `data-done`, not `data-closed`: the menu trigger
                    // sets its own `data-closed` while the menu is shut.
                    data-done={isClosed(r.item) ? "" : undefined}
                  >
                    {/* The check leads the row; the fixed-width time
                        lane sits in source order here but CSS `order`
                        moves it to the row's right edge, kept even
                        when empty so the times line up down the day. */}
                    <span class="upcoming-row-check">
                      <input
                        type="checkbox"
                        class="task-check"
                        checked={isClosed(r.item)}
                        data-cancelled={isCancelled(r.item) ? "" : undefined}
                        aria-label={m().workspace.markDone}
                        onChange={(e) =>
                          props.app.setDone(r.item.id, e.currentTarget.checked)
                        }
                      />
                    </span>
                    <span class="upcoming-row-time">
                      <Show when={r.placedBy === "deadline"}>
                        <span
                          class="upcoming-row-due-icon"
                          aria-hidden="true"
                          innerHTML={timerSvg}
                        />
                      </Show>
                      {timeLabel(r)}
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
                          + name), then the place. Home rows carry no
                          workspace marker; the task surface names the
                          list. */}
                      <Show
                        when={
                          spanDayLabel(r) ||
                          props.workspaceLabel(r.item.docId) ||
                          r.item.place
                        }
                      >
                        <span class="upcoming-row-sub">
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
                    <span class="upcoming-row-meta">
                      {/* The item's deadline is this day too: the one
                          row says so. */}
                      <Show when={isDueRow(r, g.key)}>
                        <span class="upcoming-row-due">
                          <span
                            class="upcoming-row-due-icon"
                            aria-hidden="true"
                            innerHTML={timerSvg}
                          />
                          {m().upcoming.due}
                        </span>
                      </Show>
                      <Show when={startsIn(r)}>
                        {(label) => (
                          <span class="upcoming-row-soon">{label()}</span>
                        )}
                      </Show>
                    </span>
                    {/* Row menu, past the time lane at the right edge
                        (CSS `order`), as on list rows. */}
                    <RowMenuButton />
                  </ContextMenu.Trigger>
                  </ItemContextMenu>
                  );
                }}
              </For>
            </Show>
          </section>
        )}
      </For>
    </div>
  );
}
