// Day grouping behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): every Open item carrying a `when` or a `deadline`, placed
// once on the earlier of the two days. An overdue deadline leads in an
// Overdue group so what is owed is visible at a glance and Today reads as
// today's plan; the group only exists while something is overdue. A past
// `when` has gone by like an event: it places nothing, and an item with
// only that is not selected for this view. Closed items (done or
// cancelled) keep their calendar day via `when` only ("happens on"
// outlives the tick or the cross, "owed by" does not): they keep their
// slot on that day, never in Overdue and never placed by deadline. Binned
// items never appear. Events
// (`spec/events-plan.md`) place by the same rules; an Open event with no
// date at all has no day, and no list view to fall back on, so it gets an
// Unscheduled group between Overdue and Today. A `when` that runs for
// days (`duration` beside it, `spec/calendar-plan.md` "Field:
// `duration`") puts the item on every day it covers, from today onward,
// so a span that started yesterday is still on today's page. Pure so it
// can be unit-tested without a DOM (`test/dayGroups.test.ts`).

import {
  addDaysToStamp,
  formatDeadlineBadge,
  spanEndOffset,
  whenDay,
} from "./format.tsx";
import {
  isBinned,
  isClosed,
  isEvent,
  isOpen,
  type ItemView,
} from "./sync/store.ts";

/** Most urgent first when comparing. */
export type DayTone = "overdue" | "warning" | "neutral";

export interface DayRow {
  item: ItemView;
  /** Which field put the row on this day; `none` in Unscheduled. */
  placedBy: "when" | "deadline" | "none";
  tone: DayTone;
  /** Set on every row of a `when` that covers more than one day: which
   *  day of the span this row is (1-based) and how many there are. The
   *  same item then appears on several consecutive days; `day` tells a
   *  continuation row from the first. Absent on single-day rows. */
  span?: { day: number; total: number };
}

/** The days a `when` covers, as `YYYY-MM-DD` stamps: the start day and
 *  the last day, with `total` the count between them inclusive. One day
 *  without a `duration`; otherwise the end lands `spanEndOffset` days
 *  past the start (whole days beside an all-day start, the clock end
 *  beside a timed one, so an overnight 23:00 + 2h covers two days). The
 *  one place the span is derived from the registers, for every surface
 *  that lays items out on days to share. */
export function whenSpan(
  when: string,
  duration: number | null | undefined,
): { start: string; end: string; total: number } {
  const start = whenDay(when);
  const offset = spanEndOffset(when, duration);
  return {
    start,
    end: offset > 0 ? addDaysToStamp(start, offset) : start,
    total: offset + 1,
  };
}

/** Group key of the Overdue group, which has no day of its own. */
export const OVERDUE_KEY = "overdue";
/** Group key of the Unscheduled group: Open events with no date. */
export const UNSCHEDULED_KEY = "unscheduled";

export interface DayGroup {
  /** Group key: the `YYYY-MM-DD` stamp of the day, `OVERDUE_KEY`, or
   *  `UNSCHEDULED_KEY`. */
  key: string;
  label: string;
  urgency: "overdue" | "unscheduled" | "today" | "future";
  rows: DayRow[];
}

export interface DayGroupLabels {
  overdue: string;
  unscheduled: string;
  today: string;
  tomorrow: string;
}

/** An Open event with neither a `when` nor a `deadline`. A past `when` is
 *  not unscheduled: it went by. */
const isUnscheduledEvent = (it: ItemView): boolean =>
  isEvent(it) && isOpen(it) && !it.when && !it.deadline;

/** Bucket `items` by placement day. `today` is the local `YYYY-MM-DD`
 *  stamp everything is judged against. An Overdue group leads when any
 *  deadline is past, oldest first, then `createdAt`. An Unscheduled
 *  group follows when any Open event has no date, oldest first. Today is
 *  always present after them, empty if nothing is due, so the surface
 *  anchors on the current day.
 *
 *  Within a day, rows order by the raw string of the placing field
 *  (all-day ahead of timed), then `createdAt`; ticking a row does not
 *  move it.
 *
 *  A row placed by a `when` that covers several days is repeated on each
 *  day of the span from today onward (`span` on every copy), so the
 *  same id can occur more than once down the groups; a flat reading of
 *  the rows dedupes by id. A row placed by its deadline is never
 *  repeated: the deadline is one day. */
export function groupByDay(
  items: Iterable<ItemView>,
  today: string,
  labels: DayGroupLabels,
  locale: string,
): DayGroup[] {
  const placed: { day: string; raw: string; row: DayRow }[] = [];
  const overdueRows: { raw: string; row: DayRow }[] = [];
  const unscheduled: ItemView[] = [];
  for (const it of items) {
    if (isUnscheduledEvent(it)) {
      unscheduled.push(it);
      continue;
    }
    if (isBinned(it) || (!it.when && !it.deadline)) continue;
    // Past days are not rendered here, so a `when` that is over places
    // nothing, done or not. A span still running is placed from today:
    // `wDay` is its first day on the page, the start or today, whichever
    // is later.
    const span = it.when ? whenSpan(it.when, it.duration) : null;
    const wDay =
      span && span.end >= today
        ? span.start >= today
          ? span.start
          : today
        : null;
    const dDay = it.deadline ?? null;
    // Every day of the span from `wDay` to its end, each row carrying its
    // position in the span when there is more than one day.
    const placeWhen = (tone: DayTone) => {
      const first = span!.start;
      for (let day = wDay!; day <= span!.end; day = addDaysToStamp(day, 1)) {
        const row: DayRow = { item: it, placedBy: "when", tone };
        if (span!.total > 1) {
          // Days since the start, +1 for the 1-based label.
          let n = 1;
          for (let d = first; d < day; d = addDaysToStamp(d, 1)) n++;
          row.span = { day: n, total: span!.total };
        }
        placed.push({ day, raw: it.when!, row });
      }
    };
    if (isClosed(it)) {
      // A ticked or crossed item stays put on its `when` day, unjudged:
      // the day still records what happened, or what was called off. Its
      // deadline is settled and places nothing; Overdue is Open-only.
      if (wDay === null) continue;
      placeWhen("neutral");
      continue;
    }
    // An overdue deadline is owed now whatever `when` says: it leaves the
    // day ladder for the Overdue group (red).
    if (dDay !== null && dDay < today) {
      overdueRows.push({
        raw: dDay,
        row: { item: it, placedBy: "deadline", tone: "overdue" },
      });
      continue;
    }
    // Only a past `when`: over, nothing to place.
    if (wDay === null && dDay === null) continue;
    // What is left is today or later: place on the earlier. `when` wins a
    // tie: it is the "happens on" date and renders first.
    const tone: DayTone = dDay === today ? "warning" : "neutral";
    if (wDay && (!dDay || wDay <= dDay)) {
      placeWhen(tone);
    } else {
      placed.push({
        day: dDay!,
        raw: dDay!,
        row: { item: it, placedBy: "deadline", tone },
      });
    }
  }
  const byRawThenCreated = (
    a: { raw: string; row: DayRow },
    b: { raw: string; row: DayRow },
  ) =>
    a.raw.localeCompare(b.raw) || a.row.item.createdAt - b.row.item.createdAt;
  overdueRows.sort(byRawThenCreated);
  placed.sort((a, b) => a.day.localeCompare(b.day) || byRawThenCreated(a, b));

  const out: DayGroup[] = [];
  if (overdueRows.length > 0) {
    out.push({
      key: OVERDUE_KEY,
      label: labels.overdue,
      urgency: "overdue",
      rows: overdueRows.map((p) => p.row),
    });
  }
  if (unscheduled.length > 0) {
    unscheduled.sort(
      (a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id),
    );
    out.push({
      key: UNSCHEDULED_KEY,
      label: labels.unscheduled,
      urgency: "unscheduled",
      rows: unscheduled.map((item) => ({
        item,
        placedBy: "none",
        tone: "neutral",
      })),
    });
  }
  out.push({ key: today, label: labels.today, urgency: "today", rows: [] });
  for (const p of placed) {
    const last = out[out.length - 1]!;
    if (last.key === p.day) {
      last.rows.push(p.row);
      continue;
    }
    // Day labels reuse the deadline badge's Tomorrow / weekday / compact
    // date rules, judged against today.
    const info = formatDeadlineBadge(p.day, today, labels, locale);
    out.push({
      key: p.day,
      label: info?.label ?? p.day,
      urgency: "future",
      rows: [p.row],
    });
  }
  return out;
}

/** The Upcoming nav entry's badge: how many Open rows need the user today,
 *  and the most urgent tone among them. */
export interface AttentionBadge {
  count: number;
  tone: DayTone;
}

/** Count what is owed or on today: Open items with an overdue deadline, a
 *  deadline today, or a `when` on today, a span covering it included.
 *  Tone is the most urgent counted: overdue, else warning (a deadline
 *  today), else neutral. A past `when` has gone by like an event and
 *  counts for nothing; future days are the view's business, not the
 *  badge's. Unscheduled events count too, at neutral tone: the calendar
 *  is the only place they show. */
export function attentionBadge(
  items: Iterable<ItemView>,
  today: string,
): AttentionBadge {
  let count = 0;
  let overdue = false;
  let warning = false;
  for (const it of items) {
    if (!isOpen(it)) continue;
    const dDay = it.deadline ?? null;
    if (dDay !== null && dDay < today) {
      count++;
      overdue = true;
    } else if (dDay === today) {
      count++;
      warning = true;
    } else if (it.when && coversToday(it.when, it.duration, today)) {
      count++;
    } else if (isUnscheduledEvent(it)) {
      count++;
    }
  }
  return { count, tone: overdue ? "overdue" : warning ? "warning" : "neutral" };
}

const coversToday = (
  when: string,
  duration: number | null | undefined,
  today: string,
): boolean => {
  const span = whenSpan(when, duration);
  return span.start <= today && today <= span.end;
};
