// Day grouping behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): every Open item carrying a `when` or a `deadline`. The two
// dates place independently: the `when` puts the item on its day, and
// the deadline puts a row of its own on the day it is owed (`placedBy:
// "deadline"`, the special kind of date), so an item with both is on the
// calendar twice. One row per item per group, though: a deadline on a
// day the `when` already covers rides on that row. An overdue deadline
// leads in an Overdue group so what is owed is visible at a glance and
// Today reads as today's plan; the group only exists while something is
// overdue. A task whose `when` has passed while it is still Open joins
// that group too (it was planned and did not happen, so it is still to
// do). An event's past `when` has gone by: it places nothing, and an
// event with only that is not selected for this view. Closed items (done
// or cancelled) keep their calendar day via `when` only ("happens on"
// outlives the tick or the cross, "owed by" does not): they keep their
// slot on that day, never in Overdue and never placed by deadline. Binned
// items never appear. Events
// (`spec/events-plan.md`) place by the same rules; an event always has a
// `when`, so it always has a day. A `when` that runs for
// days (`duration` beside it, `spec/calendar-plan.md` "Field:
// `duration`") puts the item on every day it covers, from today onward,
// so a span that started yesterday is still on today's page.
//
// With a `floor` the days gone by are grouped too, back to that day and
// ahead of Overdue and Today, oldest first, as `agenda --past N` and the
// Mac and iOS Calendar do (`spec/cli.md` §Items): a past day holds what
// has a `when` on it and went by, an event, neutral. A closed task
// (2026-11-18) leaves its dates: closed today it is among Today's
// completed items, closed earlier it is off the calendar (the Done view
// has it); an event alone keeps its `when` once closed. An Open task with a
// past `when` is in Overdue only, and a deadline never places in the
// past. Pure so it can be unit-tested without a DOM
// (`test/dayGroups.test.ts`).

import {
  addDaysToStamp,
  formatDeadlineBadge,
  spanCoverOffset,
  todayStamp,
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
  /** What put the row on this day: one of its two dates, or, for a
   *  closed task, the day it was closed (`done`). */
  placedBy: "when" | "deadline" | "done";
  tone: DayTone;
  /** Set on every row of a `when` that covers more than one day: which
   *  day of the span this row is (1-based) and how many there are. The
   *  same item then appears on several consecutive days; `day` tells a
   *  continuation row from the first. Absent on single-day rows. */
  span?: { day: number; total: number };
}

/** The days a `when` covers, as `YYYY-MM-DD` stamps: the start day and
 *  the last day, with `total` the count between them inclusive. One day
 *  without a `duration`; otherwise the end lands `spanCoverOffset` days
 *  past the start (whole days beside an all-day start, the clock end
 *  beside a timed one, so an overnight 23:00 + 2h covers two days; an
 *  end at 00:00 exactly is the end of the day before, not a day more). The
 *  one place the span is derived from the registers, for every surface
 *  that lays items out on days to share. */
export function whenSpan(
  when: string,
  duration: number | null | undefined,
): { start: string; end: string; total: number } {
  const start = whenDay(when);
  const offset = spanCoverOffset(when, duration);
  return {
    start,
    end: offset > 0 ? addDaysToStamp(start, offset) : start,
    total: offset + 1,
  };
}

/** How far back the Upcoming view reads: a year of days gone by, as the
 *  Mac and iOS Calendar do (`agendaPastDays`). */
export const AGENDA_PAST_DAYS = 365;

/** Group key of the Overdue group, which has no day of its own. */
export const OVERDUE_KEY = "overdue";

export interface DayGroup {
  /** Group key: the `YYYY-MM-DD` stamp of the day, or `OVERDUE_KEY`. */
  key: string;
  label: string;
  /** `past`: a day before today, there only when the grouping was given
   *  a floor. */
  urgency: "past" | "overdue" | "today" | "future";
  rows: DayRow[];
}

export interface DayGroupLabels {
  overdue: string;
  today: string;
  tomorrow: string;
}

/** An Open task (anything not marked an event) whose `when` is over:
 *  its whole span ended before `today`. Planned, not done, so it is
 *  overdue. An event in the same position simply went by. */
export const isSlippedTask = (it: ItemView, today: string): boolean =>
  isOpen(it) &&
  !isEvent(it) &&
  !!it.when &&
  whenSpan(it.when, it.duration).end < today;

/** Where a row sits among its day's rows (`spec/calendar-plan.md`
 *  "Within a day"): all-day events lead (0), they are the day's
 *  backdrop; then what has a clock time, tasks and events alike (1);
 *  then what is for the day at no particular time, an untimed task or a
 *  deadline's row (2). Mirrors `day_band` in the CLI's `agenda.rs`. */
export const dayBand = (row: DayRow): 0 | 1 | 2 => {
  if (row.placedBy !== "when") return 2;
  if ((row.item.when?.length ?? 0) > 10) return 1;
  return isEvent(row.item) ? 0 : 2;
};

/** A calendar item that leaves its dates once closed: a task (done or
 *  cancelled) is on the day it was closed, not the day it was planned
 *  for. An event stays on its `when`: it happened then whatever was
 *  ticked. The flag only means something beside a `when`. */
export const isClosedTask = (it: ItemView): boolean =>
  isClosed(it) && !(isEvent(it) && !!it.when);

/** Bucket `items` by placement day. `today` is the local `YYYY-MM-DD`
 *  stamp everything is judged against. An Overdue group leads when any
 *  deadline is past or any Open task's `when` is over, oldest first by
 *  the date that put the row there, then `createdAt`. Today is always
 *  present after it, empty if nothing is due, so the surface
 *  anchors on the current day.
 *
 *  Within a day, rows order in three bands (`dayBand`): all-day events,
 *  timed rows by the clock, then untimed tasks and deadline rows. Inside
 *  a band, the raw string of the placing field, then `createdAt`.
 *  Overdue has no bands.
 *
 *  A closed task (`isClosedTask`) leaves its dates: closed today
 *  (`lifecycleAt`, a day ahead read as today) it is one of Today's rows,
 *  placed by `done`, after the others in the order they were closed;
 *  closed on a day gone by it is not on the agenda at all. `lingers` holds one just closed where it was, placed as if it
 *  were still open, for the beat before it moves. A closed event stays
 *  on its `when`.
 *
 *  A row placed by a `when` that covers several days is repeated on each
 *  day of the span from today onward (`span` on every copy), and an item
 *  with both dates has its deadline row besides, so the same id can
 *  occur more than once down the groups; a flat reading of the rows
 *  dedupes by id. The deadline is one day, and never a second row in a
 *  group the item is already in: on a day its `when` covers, that row is
 *  the one (`isDueRow`), and with both dates behind it the deadline row
 *  is the one in Overdue.
 *
 *  `floor` (a `YYYY-MM-DD` stamp before `today`) adds the days gone by
 *  from that day on, each non-empty one a `past` group ahead of Overdue
 *  and Today, oldest first: events on the days their `when` covered.
 *  An item can then sit on a past day and again from
 *  today on (a span still running, or an event that went by with a
 *  deadline ahead). Without it no past day is grouped. */
export function groupByDay(
  items: Iterable<ItemView>,
  today: string,
  labels: DayGroupLabels,
  locale: string,
  floor?: string,
  lingers: (item: ItemView) => boolean = () => false,
): DayGroup[] {
  const yesterday = addDaysToStamp(today, -1);
  const placed: { day: string; raw: string; row: DayRow }[] = [];
  const overdueRows: { raw: string; row: DayRow }[] = [];
  for (const it of items) {
    if (isBinned(it) || (!it.when && !it.deadline)) continue;
    const closedTask = isClosedTask(it);
    if (closedTask && !lingers(it)) {
      // Among today's completed items if it was closed today (or on a
      // clock ahead of ours), whatever its dates say. Closed on a day
      // gone by, it is off the calendar: the Done view is its record.
      if (todayStamp(it.lifecycleAt) < today) continue;
      placed.push({
        day: today,
        raw: String(it.lifecycleAt).padStart(16, "0"),
        row: { item: it, placedBy: "done", tone: "neutral" },
      });
      continue;
    }
    // From today on, a `when` that is over places nothing, done or not.
    // A span still running is placed from today: `wDay` is its first day
    // there, the start or today, whichever is later.
    const span = it.when ? whenSpan(it.when, it.duration) : null;
    const wDay =
      span && span.end >= today
        ? span.start >= today
          ? span.start
          : today
        : null;
    const dDay = it.deadline ?? null;
    // Every day of the span from `from` to `to`, each row carrying its
    // position in the span when there is more than one day.
    // `dueDay`: the deadline's day when it is today, whose row reads as
    // due (warning) for carrying the deadline too.
    const placeWhen = (from: string, to: string, dueDay?: string) => {
      const first = span!.start;
      for (let day = from; day <= to; day = addDaysToStamp(day, 1)) {
        const row: DayRow = {
          item: it,
          placedBy: "when",
          tone: day === dueDay ? "warning" : "neutral",
        };
        if (span!.total > 1) {
          // Days since the start, +1 for the 1-based label.
          let n = 1;
          for (let d = first; d < day; d = addDaysToStamp(d, 1)) n++;
          row.span = { day: n, total: span!.total };
        }
        placed.push({ day, raw: it.when!, row });
      }
    };
    // The days gone by: what happened on them (an event), on each day
    // the `when` covered before today and not before the floor. The rest
    // of the placement is from today on.
    if (floor && span && span.start < today && isEvent(it)) {
      const from = span.start >= floor ? span.start : floor;
      const to = span.end < yesterday ? span.end : yesterday;
      if (from <= to) placeWhen(from, to);
    }
    if (isClosed(it) && !closedTask) {
      // A ticked or crossed event stays put on its `when` day, unjudged:
      // the day still records what happened, or what was called off. Its
      // deadline is settled and places nothing; Overdue is Open-only.
      if (wDay === null) continue;
      placeWhen(wDay, span!.end);
      continue;
    }
    // The two dates place independently from here: the `when` row(s),
    // then the deadline's own row unless the item already has one in
    // that group. A lingering closed task is placed here too, where it
    // was while open.
    const deadlineOverdue = dDay !== null && dDay < today;
    if (wDay !== null) {
      placeWhen(wDay, span!.end, dDay === today ? today : undefined);
    } else if (
      (isSlippedTask(it, today) ||
        (closedTask && span !== null && span.end < today)) &&
      !deadlineOverdue
    ) {
      // A task planned for a day that has passed is still to do: Overdue,
      // by its `when`. With the deadline behind it as well the deadline
      // row below is the one: it is what is owed.
      overdueRows.push({
        raw: it.when!,
        row: { item: it, placedBy: "when", tone: "overdue" },
      });
    }
    if (dDay === null) continue;
    if (deadlineOverdue) {
      // Owed now: the Overdue group (red), whatever `when` says.
      overdueRows.push({
        raw: dDay,
        row: { item: it, placedBy: "deadline", tone: "overdue" },
      });
    } else if (wDay === null || dDay < wDay || dDay > span!.end) {
      // Its own row on its day: the special kind of date. On a day the
      // `when` covers, that row carries it instead (`isDueRow`).
      placed.push({
        day: dDay,
        raw: dDay,
        row: {
          item: it,
          placedBy: "deadline",
          tone: dDay === today ? "warning" : "neutral",
        },
      });
    }
  }
  const byRawThenCreated = (
    a: { raw: string; row: DayRow },
    b: { raw: string; row: DayRow },
  ) =>
    a.raw.localeCompare(b.raw) || a.row.item.createdAt - b.row.item.createdAt;
  overdueRows.sort(byRawThenCreated);
  // Closed tasks after the day's other rows, by when they were closed.
  const closedLast = (r: DayRow) => (r.placedBy === "done" ? 1 : 0);
  placed.sort(
    (a, b) =>
      a.day.localeCompare(b.day) ||
      closedLast(a.row) - closedLast(b.row) ||
      dayBand(a.row) - dayBand(b.row) ||
      byRawThenCreated(a, b),
  );

  // The days gone by, then Overdue, then Today and the days ahead.
  const past: DayGroup[] = [];
  const ahead: DayGroup[] = [
    { key: today, label: labels.today, urgency: "today", rows: [] },
  ];
  for (const p of placed) {
    const gone = p.day < today;
    const into = gone ? past : ahead;
    const last = into[into.length - 1];
    if (last?.key === p.day) {
      last.rows.push(p.row);
      continue;
    }
    // Day labels reuse the deadline badge's Tomorrow / weekday / compact
    // date rules, judged against today; a day gone by is its date.
    const info = formatDeadlineBadge(p.day, today, labels, locale, {
      pastAsDate: true,
    });
    into.push({
      key: p.day,
      label: info?.label ?? p.day,
      urgency: gone ? "past" : "future",
      rows: [p.row],
    });
  }
  const out = past;
  if (overdueRows.length > 0) {
    out.push({
      key: OVERDUE_KEY,
      label: labels.overdue,
      urgency: "overdue",
      rows: overdueRows.map((p) => p.row),
    });
  }
  out.push(...ahead);
  return out;
}

/** A `when` row on the day its item's deadline falls: the one row
 *  stands for both dates there, so the surface marks it as due. `day` is
 *  the row's group key. */
export const isDueRow = (row: DayRow, day: string): boolean =>
  row.placedBy === "when" && isOpen(row.item) && row.item.deadline === day;

/** Every item on the agenda once, in display order, with the key of the
 *  group its row is in: what the selection runs over. An item on more
 *  than one day (a multi-day span, a date and a deadline, or a past day
 *  and a day ahead) counts
 *  at its first row from Overdue on, and at its first past row only when
 *  it has none there: a span that began yesterday is one of today's
 *  rows, not yesterday's. */
export function agendaItems(
  groups: readonly DayGroup[],
): { item: ItemView; group: string }[] {
  const current = new Set<string>();
  for (const g of groups) {
    if (g.urgency === "past") continue;
    for (const r of g.rows) current.add(r.item.id);
  }
  const seen = new Set<string>();
  const out: { item: ItemView; group: string }[] = [];
  for (const g of groups) {
    for (const r of g.rows) {
      const id = r.item.id;
      if (seen.has(id) || (g.urgency === "past" && current.has(id))) continue;
      seen.add(id);
      out.push({ item: r.item, group: g.key });
    }
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
 *  task `when` that is over, a deadline today, or a `when` on today, a
 *  span covering it included. Tone is the most urgent counted: overdue,
 *  else warning (a deadline today), else neutral. An event's past `when`
 *  has gone by and counts for nothing; future days are the view's
 *  business, not the badge's. */
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
    if ((dDay !== null && dDay < today) || isSlippedTask(it, today)) {
      count++;
      overdue = true;
    } else if (dDay === today) {
      count++;
      warning = true;
    } else if (it.when && coversToday(it.when, it.duration, today)) {
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

/** How far ahead of today the agenda's "all days" mode reaches when
 *  nothing is placed later: two weeks, so the days ahead read as a
 *  calendar even when they are empty. */
export const AGENDA_ALL_DAYS_AHEAD = 13;

/** The agenda with every day in it, not just the days something is on
 *  (the web Upcoming view's "All days" toggle, a device preference): an
 *  empty group for each missing day from the first past group (if any)
 *  through to the later of the last placed day and `today + ahead`.
 *  Overdue keeps its place between the days gone by and Today. Labels
 *  follow `groupByDay`'s rules. */
export function fillEmptyDays(
  groups: readonly DayGroup[],
  today: string,
  labels: DayGroupLabels,
  locale: string,
  ahead: number = AGENDA_ALL_DAYS_AHEAD,
): DayGroup[] {
  const byDay = new Map<string, DayGroup>();
  let first = today;
  let last = addDaysToStamp(today, ahead);
  for (const g of groups) {
    if (g.key === OVERDUE_KEY) continue;
    byDay.set(g.key, g);
    if (g.key < first) first = g.key;
    if (g.key > last) last = g.key;
  }
  const overdue = groups.find((g) => g.key === OVERDUE_KEY);
  const out: DayGroup[] = [];
  for (let day = first; day <= last; day = addDaysToStamp(day, 1)) {
    if (day === today && overdue) out.push(overdue);
    const have = byDay.get(day);
    if (have) {
      out.push(have);
      continue;
    }
    const info = formatDeadlineBadge(day, today, labels, locale, {
      pastAsDate: true,
    });
    out.push({
      key: day,
      label: day === today ? labels.today : (info?.label ?? day),
      urgency: day < today ? "past" : day === today ? "today" : "future",
      rows: [],
    });
  }
  return out;
}
