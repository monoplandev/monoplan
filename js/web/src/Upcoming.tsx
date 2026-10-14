// The Upcoming view: the agenda. Every Open item with a planned date or a
// deadline, bucketed by day (`dayGroups.ts`), Today always anchored so the
// surface reads as "the days ahead" even when nothing is due. Overdue
// deadlines sit in an Overdue section above Today. A past `when` is over
// and places nothing here. Rows carry no date pills: the day header is the
// date, and the task surface shows the rest. The checkbox leads each row,
// then a fixed-width lane carries the start time ("All day", or "Due" on
// a deadline); the place is a muted line under the title.
// Rows tick off in place and stay where they are on their `when` day once
// done or cancelled, muted (tick or cross), so the day still records what
// happened and what was called off. Click opens the
// task surface (dialog or side panel).
//
// Events (`spec/events-plan.md`) live here and nowhere else. An event row
// is a task row without the list label: the checkbox is there for anyone
// who wants the record, but nothing asks for it, the event just goes by.
// Events with no date sit in an Unscheduled section between Overdue and
// Today so they are never out of sight.
//
// Deliberately not a `Dnd` listbox: day sections are the point, and the
// flat virtualised list can't host group headers. Drag-to-reschedule is
// deferred (`spec/calendar-plan.md`).

import { createMemo, For, Show } from "solid-js";
import { groupByDay, type DayGroup, type DayRow } from "./dayGroups.ts";
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
  type DocApp,
} from "./sync/store.ts";

export function Upcoming(props: {
  app: DocApp;
  /** Display label for a list id (Inbox is localized, others by name). */
  listLabel: (listId: string) => string;
  onOpen: (id: string) => void;
}) {
  const { m, locale } = useAppI18n();

  const groups = createMemo<DayGroup[]>(() =>
    groupByDay(
      Object.values(props.app.state.itemsById),
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

  // "09:00–10:00" for a timed row with a length, the start alone without
  // one, "All day" without a time. An overnight end reads as its clock
  // time, as the task dialog shows it.
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
    <div class="upcoming" tabIndex={-1}>
      <For each={groups()}>
        {(g) => (
          <section class="upcoming-day" data-urgency={g.urgency}>
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
                {(r) => (
                  <div
                    class="upcoming-row"
                    role="button"
                    tabIndex={-1}
                    data-tone={r.tone}
                    data-closed={isClosed(r.item) ? "" : undefined}
                    onClick={(e) => {
                      const t = e.target as HTMLElement | null;
                      if (t?.closest("input")) return;
                      props.onOpen(r.item.id);
                    }}
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
                          title={props.listLabel(r.item.listId)}
                        >
                          <span class="row-list-name">
                            {props.listLabel(r.item.listId)}
                          </span>
                        </span>
                      </Show>
                    </span>
                  </div>
                )}
              </For>
            </Show>
          </section>
        )}
      </For>
    </div>
  );
}
