// The Upcoming view: the agenda. Every Open item with a planned date or a
// deadline, bucketed by day (`dayGroups.ts`, grouped by the Workspace so
// the same rows feed its selection-scoped shortcuts), Today always
// anchored so the surface reads as "the days ahead" even when nothing is
// due. Overdue deadlines sit in an Overdue section above Today. A past
// `when` is over and places nothing here. Rows carry no date pills: the
// day header is the date, and the task surface shows the rest. The
// checkbox leads each row, then a fixed-width lane carries the start time
// ("All day", or "Due" on a deadline); the place is a muted line under
// the title.
// Rows tick off in place and stay where they are on their `when` day once
// done or cancelled, muted (tick or cross), so the day still records what
// happened and what was called off.
//
// Events (`spec/events-plan.md`) live here and nowhere else. An event row
// looks like a task row: the checkbox is there for anyone who wants the
// record, but nothing asks for it, the event just goes by. No row carries
// its list label; the context menu and task surface name it.
// Events with no date sit in an Unscheduled section between Overdue and
// Today so they are never out of sight.
//
// Selection and keys: the agenda is a listbox in the list view's sense
// (click selects, the side pane follows, Enter / double-click opens, the
// item shortcuts act on the selection) but not a `Dnd`: day sections are
// the point, rows are not one height, and the flat virtualised list can't
// host group headers. So it runs the Workspace's `DndSelection` over the
// rows of every day concatenated, and answers the same keys through the
// shared `applySelectionKeyAction`, which makes ↑ / ↓ cross day boundaries
// for free. Nothing is virtualised: the agenda is bounded to overdue plus
// today onward. Drag-to-reschedule is deferred (`spec/calendar-plan.md`).

import {
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
import type { DayGroup, DayRow } from "./dayGroups.ts";
import {
  applySelectionKeyAction,
  mapDndKeyEvent,
  type DndSelection,
} from "./dnd";
import {
  addDaysToStamp,
  formatClockTime,
  formatDurationShort,
  nowMs,
  todayStamp,
  whenStartMs,
  whenTime,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import mapPinSvg from "./icons/map-pin.svg?raw";
import usersSvg from "./icons/users.svg?raw";
import {
  isCancelled,
  isClosed,
  type ItemActions,
} from "./sync/store.ts";

/** The slice of `DndImperative` the Workspace drives on this view. */
export interface UpcomingImperative {
  /** Move keyboard focus to the agenda so arrow keys act on it. Never
   *  scrolls (see `DndImperative.focus`). */
  focus(): void;
  /** Bring a row into view, if it is on the agenda. */
  scrollToKey(id: string): void;
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
  listLabel: (listId: string, docId?: string) => string;
  /** The agenda merges every workspace, so a row from a shared one
   *  (anything but Home) names it under the title with the shared
   *  people glyph; null for Home rows, which carry no marker. */
  workspaceLabel: (docId: string) => string | null;
  onOpen: (id: string) => void;
  /** Touch: a tap opens the row (there is no pane to follow a selection). */
  openOnTap?: () => boolean;
  ref?: (handle: UpcomingImperative) => void;
  /** Context menu wiring, as the list rows take it (`ItemContextMenu`). */
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  onReveal?: (id: string, where: "list" | "focus") => void;
  onMoveToList?: (ids: readonly string[]) => void;
}) {
  const { m, locale } = useAppI18n();

  let hostEl!: HTMLDivElement;

  // The selection's order is every day's rows in display order, each id
  // once: a multi-day span repeats its item on each day it covers
  // (`DayRow.span`), and selecting any of those rows selects the item,
  // so every copy lights. Fresh array per change: `updateOrder` is
  // identity-guarded.
  const order = createMemo<string[]>(() => {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const g of props.groups) {
      for (const r of g.rows) {
        if (seen.has(r.item.id)) continue;
        seen.add(r.item.id);
        out.push(r.item.id);
      }
    }
    return out;
  });
  createEffect(() => props.selection.updateOrder(order()));

  // Selected set as a signal so only rows whose membership flips re-render
  // their chrome. `equals: false`: the selection mutates in place.
  const [selTick, setSelTick] = createSignal(0, { equals: false });
  onCleanup(props.selection.onChange(() => setSelTick((n) => n + 1)));
  const selectedSet = createMemo(() => {
    selTick();
    return props.selection.getSelectedKeySet();
  });

  // Which copy of a multi-day span draws the selection: the one the
  // pointer picked, else the span's earliest day on the page. The
  // selection itself is by item id (one item, however many days it
  // covers), so this is chrome only; a copy is lit when its item is
  // selected and it is that item's chosen day.
  const [pickedDay, setPickedDay] = createSignal<Map<string, string>>(
    new Map(),
  );
  const firstDayOf = createMemo(() => {
    const out = new Map<string, string>();
    for (const g of props.groups) {
      for (const r of g.rows) {
        if (!out.has(r.item.id)) out.set(r.item.id, g.key);
      }
    }
    return out;
  });
  const litDay = (id: string): string | undefined =>
    pickedDay().get(id) ?? firstDayOf().get(id);
  // A pointer pick only lives while its item stays selected; keyboard
  // and other paths fall back to the first copy.
  createEffect(() => {
    const sel = selectedSet();
    const picked = pickedDay();
    if ([...picked.keys()].every((id) => sel.has(id))) return;
    const next = new Map(picked);
    for (const id of picked.keys()) if (!sel.has(id)) next.delete(id);
    setPickedDay(next);
  });

  // The first row carrying the id in DOM order: a span's earliest day.
  const scrollToKey = (id: string) => {
    hostEl
      .querySelector(`[data-key="${CSS.escape(id)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  };

  props.ref?.({
    focus: () => hostEl.focus({ preventScroll: true }),
    scrollToKey,
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
  const onKeyDown = (e: KeyboardEvent) => {
    const action = mapDndKeyEvent(e);
    if (action.type === "ignore" || action.type === "move-selection") return;
    e.preventDefault();
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
  // Unscheduled has no day at all.
  const dayHeading = (g: DayGroup): string => {
    if (g.urgency === "overdue" || g.urgency === "unscheduled") return g.label;
    const [y, mo, d] = g.key.split("-").map(Number);
    if (!y || !mo || !d) return g.label;
    const date = new Intl.DateTimeFormat(locale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
    }).format(new Date(y, mo - 1, d));
    const today = todayStamp(nowMs());
    if (g.key === today) return `${m().deadline.today} - ${date}`;
    if (g.key === addDaysToStamp(today, 1)) {
      return `${m().deadline.tomorrow} - ${date}`;
    }
    return date;
  };

  // Start time only: the agenda is a glance at the day, the end sits on
  // the task surface. An all-day item says so in the lane, and a row
  // here on its deadline says "Due". A multi-day span counts its days
  // instead ("Day 2/3"): on the first day too when it is all-day, since
  // the count says "all day" and more; a timed start keeps its clock.
  const timeLabel = (r: DayRow): string => {
    if (r.placedBy === "deadline") return m().upcoming.due;
    if (r.placedBy !== "when") return "";
    const start = whenTime(r.item.when!);
    if (r.span && (!start || r.span.day > 1)) {
      return m().upcoming.spanDay(r.span.day, r.span.total);
    }
    if (!start) return m().when.allDay;
    return formatClockTime(start, locale());
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
    >
      <For each={props.groups}>
        {(g) => (
          <section
            class="upcoming-day"
            role="group"
            aria-label={dayHeading(g)}
            data-urgency={g.urgency}
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
                      (!r.span || litDay(r.item.id) === g.key),
                  );
                  // Keeps the "…" button showing while its menu is up
                  // (`.upcoming-row[data-menu-open]`), as list rows do.
                  const [menuOpen, setMenuOpen] = createSignal(false);
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
                    <span class="upcoming-row-time">{timeLabel(r)}</span>
                    <span class="upcoming-row-body">
                      <span class="upcoming-row-title">
                        <span class="upcoming-row-text">{r.item.text}</span>
                      </span>
                      {/* Sub line under the title: the shared workspace a
                          foreign row comes from (people glyph + name),
                          then the place. Home rows carry no workspace
                          marker; the task surface names the list. */}
                      <Show when={props.workspaceLabel(r.item.docId) || r.item.place}>
                        <span class="upcoming-row-sub">
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
