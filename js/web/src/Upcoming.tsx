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
// is a task row without the list label: the checkbox is there for anyone
// who wants the record, but nothing asks for it, the event just goes by.
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
import type { DayGroup, DayRow } from "./dayGroups.ts";
import {
  applySelectionKeyAction,
  mapDndKeyEvent,
  type DndSelection,
} from "./dnd";
import {
  addDaysToStamp,
  formatClockTime,
  nowMs,
  todayStamp,
  whenTime,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import mapPinSvg from "./icons/map-pin.svg?raw";
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
  onOpen: (id: string) => void;
  /** Touch: a tap opens the row (there is no pane to follow a selection). */
  openOnTap?: () => boolean;
  ref?: (handle: UpcomingImperative) => void;
  /** Context menu wiring, as the list rows take it (`ItemContextMenu`). */
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  onSetDeadline?: (id: string) => void;
  onSetWhen?: (id: string) => void;
  onReveal?: (id: string, where: "list" | "focus") => void;
  onMoveToList?: (ids: readonly string[]) => void;
}) {
  const { m, locale } = useAppI18n();

  let hostEl!: HTMLDivElement;
  const rowEls = new Map<string, HTMLElement>();

  // The selection's order is every day's rows in display order. An item
  // is placed on one day only, so ids are unique down the agenda. Fresh
  // array per change: `updateOrder` is identity-guarded.
  const order = createMemo<string[]>(() =>
    props.groups.flatMap((g) => g.rows.map((r) => r.item.id)),
  );
  createEffect(() => props.selection.updateOrder(order()));

  // Selected set as a signal so only rows whose membership flips re-render
  // their chrome. `equals: false`: the selection mutates in place.
  const [selTick, setSelTick] = createSignal(0, { equals: false });
  onCleanup(props.selection.onChange(() => setSelTick((n) => n + 1)));
  const selectedSet = createMemo(() => {
    selTick();
    return props.selection.getSelectedKeySet();
  });

  const scrollToKey = (id: string) => {
    rowEls.get(id)?.scrollIntoView({ block: "nearest" });
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
  const rowIdFromEvent = (e: MouseEvent): string | null => {
    const t = e.target as HTMLElement | null;
    const row = t?.closest<HTMLElement>("[data-key]");
    return row && hostEl.contains(row) ? (row.dataset.key ?? null) : null;
  };
  const onClick = (e: MouseEvent) => {
    const t = e.target as HTMLElement | null;
    if (t?.closest("input")) return;
    const id = rowIdFromEvent(e);
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
  // here on its deadline says "Due".
  const timeLabel = (r: DayRow): string => {
    if (r.placedBy === "deadline") return m().upcoming.due;
    if (r.placedBy !== "when") return "";
    const start = whenTime(r.item.when!);
    if (!start) return m().when.allDay;
    return formatClockTime(start, locale());
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
                  const selected = createMemo(() => selectedSet().has(r.item.id));
                  return (
                  <ItemContextMenu
                    item={() => r.item}
                    app={props.app}
                    selection={props.selection}
                    viewKind="upcoming"
                    duplicateBlock={props.duplicateBlock}
                    copyBlock={props.copyBlock}
                    onOpen={props.onOpen}
                    onSetDeadline={props.onSetDeadline}
                    onSetWhen={props.onSetWhen}
                    onReveal={props.onReveal}
                    onMoveToList={props.onMoveToList}
                    listLabel={props.listLabel}
                  >
                  <ContextMenu.Trigger
                    ref={(el: HTMLElement) => {
                      rowEls.set(r.item.id, el);
                      onCleanup(() => {
                        if (rowEls.get(r.item.id) === el) rowEls.delete(r.item.id);
                      });
                    }}
                    class="upcoming-row"
                    role="option"
                    data-key={r.item.id}
                    aria-selected={selected() ? "true" : "false"}
                    data-tone={r.tone}
                    // `data-done`, not `data-closed`: the menu trigger
                    // sets its own `data-closed` while the menu is shut.
                    data-done={isClosed(r.item) ? "" : undefined}
                  >
                    {/* The check leads the row, then a fixed-width time
                        lane kept even when empty so the titles line up
                        down the day. */}
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
                    <span class="upcoming-row-meta">
                      {/* No label is the event's mark: every task row
                          carries its list, an event has none to show. */}
                      <Show when={!isEvent(r.item)}>
                        <span
                          class="badge row-list"
                          title={props.listLabel(r.item.listId, r.item.docId)}
                        >
                          <span class="row-list-name">
                            {props.listLabel(r.item.listId, r.item.docId)}
                          </span>
                        </span>
                      </Show>
                    </span>
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
