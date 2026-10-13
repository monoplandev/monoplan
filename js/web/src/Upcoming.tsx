// The Upcoming view: the agenda. Every Open item with a planned date or a
// deadline, bucketed by day (`dayGroups.ts`), Today always anchored so the
// surface reads as "the days ahead" even when nothing is due. Overdue
// deadlines sit in an Overdue section above Today. A past `when` is over
// and places nothing here. Rows carry no date pills: the day header is the
// date, and the task surface shows the rest. The time span (or "All
// day") leads the title line; the place is a muted line under it.
// Rows tick off in place and stay where they are on their `when` day once
// done, muted, so the day still records what happened. Click opens the
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
  endTimeOf,
  formatClockTime,
  nowMs,
  todayStamp,
  whenTime,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import { isDone, isEvent, type DocApp } from "./sync/store.ts";

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
  // is the only one annotated, so the eye lands on it without the other
  // days changing shape as they approach. Overdue has no day: its rows
  // each carry their own date. Unscheduled has no day at all.
  const dayHeading = (g: DayGroup): string => {
    if (g.urgency === "overdue" || g.urgency === "unscheduled") return g.label;
    const [y, mo, d] = g.key.split("-").map(Number);
    if (!y || !mo || !d) return g.label;
    const date = new Intl.DateTimeFormat(locale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
    }).format(new Date(y, mo - 1, d));
    return g.urgency === "today" ? `${date} (${m().deadline.today})` : date;
  };

  // "09:00–10:00" for a timed row with a length, the start alone without
  // one, "All day" without a time. An overnight end reads as its clock
  // time, as the task dialog shows it.
  const timeLabel = (r: DayRow): string => {
    if (r.placedBy !== "when") return "";
    const start = whenTime(r.item.when!);
    if (!start) return m().when.allDay;
    const from = formatClockTime(start, locale());
    const d = r.item.duration;
    if (!d) return from;
    return `${from}\u2013${formatClockTime(endTimeOf(start, d), locale())}`;
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
                    data-done={isDone(r.item) ? "" : undefined}
                    onClick={(e) => {
                      const t = e.target as HTMLElement | null;
                      if (t?.closest("input")) return;
                      props.onOpen(r.item.id);
                    }}
                  >
                    <input
                      type="checkbox"
                      class="task-check"
                      checked={isDone(r.item)}
                      aria-label={m().workspace.markDone}
                      onChange={(e) =>
                        props.app.setDone(r.item.id, e.currentTarget.checked)
                      }
                    />
                    <span class="upcoming-row-body">
                      <span class="upcoming-row-title">
                        <Show when={timeLabel(r)}>
                          {(t) => <span class="upcoming-row-time">{t()}</span>}
                        </Show>
                        <span class="upcoming-row-text">{r.item.text}</span>
                      </span>
                      <Show when={r.item.place}>
                        {(p) => (
                          <span
                            class="upcoming-row-place"
                            title={p().address ?? p().label}
                          >
                            {p().label}
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
