// The Upcoming view: the agenda. Every Open item with a planned date or a
// deadline, bucketed by day (`dayGroups.ts`), Today always anchored so the
// surface reads as "the days ahead" even when nothing is due. Past dates
// of either kind sit in an Overdue section above Today, and are the only
// placing dates that badge (the day header carries everyone else's). The
// other field, when present, badges beside it so a row reads
// "Sat 13 · due 31 Oct". Timed rows lead with their time. Rows tick off in
// place and stay where they are on their `when` day once done, muted, so
// the day still records what happened. Click opens the task surface
// (dialog or side panel).
//
// Deliberately not a `Dnd` listbox: day sections are the point, and the
// flat virtualised list can't host group headers. Drag-to-reschedule is
// deferred (`spec/calendar-plan.md`).

import { createMemo, For, Show } from "solid-js";
import { groupByDay, type DayGroup, type DayRow } from "./dayGroups.ts";
import { DeadlineBadge } from "./DeadlineBadge.tsx";
import { formatWhenTime, nowMs, todayStamp } from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import { isDone, type DocApp } from "./sync/store.ts";
import { WhenBadge } from "./WhenBadge.tsx";

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
        today: m().deadline.today,
        tomorrow: m().deadline.tomorrow,
      },
      locale(),
    ),
  );

  // Every day heads with the same long-form date ("Sat 24 Sept"); Today
  // is the only one annotated, so the eye lands on it without the other
  // days changing shape as they approach. Overdue has no day: its rows
  // each carry their own date.
  const dayHeading = (g: DayGroup): string => {
    if (g.urgency === "overdue") return g.label;
    const [y, mo, d] = g.key.split("-").map(Number);
    if (!y || !mo || !d) return g.label;
    const date = new Intl.DateTimeFormat(locale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
    }).format(new Date(y, mo - 1, d));
    return g.urgency === "today" ? `${date} (${m().deadline.today})` : date;
  };

  // The placing field badges only in Overdue, where the header carries no
  // day; the other field always badges.
  const showPlacingWhen = (r: DayRow, g: DayGroup) =>
    r.placedBy === "when" && g.urgency === "overdue";
  const showPlacingDeadline = (r: DayRow, g: DayGroup) =>
    r.placedBy === "deadline" && g.urgency === "overdue";
  const timeLabel = (r: DayRow) =>
    r.placedBy === "when" ? formatWhenTime(r.item.when!, locale()) : "";

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
                    <Show when={timeLabel(r)}>
                      {(t) => <span class="upcoming-row-time">{t()}</span>}
                    </Show>
                    <span class="upcoming-row-text">{r.item.text}</span>
                    <span class="upcoming-row-meta">
                      <Show when={showPlacingWhen(r, g)}>
                        <WhenBadge when={r.item.when!} />
                      </Show>
                      <Show when={r.placedBy === "deadline" && r.item.when}>
                        {(w) => <WhenBadge when={w()} />}
                      </Show>
                      <Show when={showPlacingDeadline(r, g)}>
                        <DeadlineBadge deadline={r.item.deadline!} pastAsDate />
                      </Show>
                      <Show when={r.placedBy === "when" && r.item.deadline}>
                        {(d) => (
                          <DeadlineBadge
                            deadline={d()}
                            muted={isDone(r.item)}
                          />
                        )}
                      </Show>
                      <span
                        class="badge row-list"
                        title={props.listLabel(r.item.listId)}
                      >
                        <span class="row-list-name">
                          {props.listLabel(r.item.listId)}
                        </span>
                      </span>
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
