// Day grouping behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): every Open item carrying a `when`, on its day. A task whose
// `when` has passed while it is still Open leads in an Overdue group (it
// was planned and did not happen, so it is still to do), so Today reads
// as today's plan; the group only exists while something is overdue. An
// event's past `when` has gone by: it places nothing. A closed event
// keeps its slot on its `when` day, never in Overdue. Binned items never
// appear. Events
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
// leaves the calendar once closed (2026-11-22; the Done view has it);
// an event alone keeps its `when` once closed. An Open task with a
// past `when` is in Overdue only. Pure so it can be unit-tested without a DOM
// (`test/dayGroups.test.ts`).

import {
  addDaysToStamp,
  formatDayLabel,
  spanCoverOffset,
  whenDay,
} from "./format.tsx";
import {
  isBinned,
  isClosed,
  isEvent,
  isOpen,
  type ItemView,
} from "./sync/store.ts";

export type DayTone = "overdue" | "neutral";

export interface DayRow {
  item: ItemView;
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
 *  then untimed tasks, for the day at no particular time (2). Mirrors
 *  `day_band` in the CLI's `agenda.rs`. */
export const dayBand = (row: DayRow): 0 | 1 | 2 => {
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
 *  Open task's `when` is over, oldest first by its `when`, then
 *  `createdAt`. Today is always present after it, empty if nothing is
 *  on, so the surface anchors on the current day.
 *
 *  Within a day, rows order in three bands (`dayBand`): all-day events,
 *  timed rows by the clock, then untimed tasks. Inside a band, the raw
 *  `when`, then `createdAt`. Overdue has no bands.
 *
 *  A closed task (`isClosedTask`) is not on the agenda. `lingers` holds
 *  one just closed where it was, placed as if it were still open, for
 *  the beat before it leaves. A closed event stays on its `when`.
 *
 *  A row placed by a `when` that covers several days is repeated on each
 *  day of the span from today onward (`span` on every copy), so the same
 *  id can occur more than once down the groups; a flat reading of the
 *  rows dedupes by id.
 *
 *  `floor` (a `YYYY-MM-DD` stamp before `today`) adds the days gone by
 *  from that day on, each non-empty one a `past` group ahead of Overdue
 *  and Today, oldest first: events on the days their `when` covered.
 *  An item can then sit on a past day and again from
 *  today on (a span still running). Without it no past day is grouped. */
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
    if (isBinned(it) || !it.when) continue;
    const closedTask = isClosedTask(it);
    // Off the calendar once closed: the Done view is its record.
    if (closedTask && !lingers(it)) continue;
    // From today on, a `when` that is over places nothing, done or not.
    // A span still running is placed from today: `wDay` is its first day
    // there, the start or today, whichever is later.
    const span = whenSpan(it.when, it.duration);
    const wDay =
      span.end >= today ? (span.start >= today ? span.start : today) : null;
    // Every day of the span from `from` to `to`, each row carrying its
    // position in the span when there is more than one day.
    const placeWhen = (from: string, to: string) => {
      for (let day = from; day <= to; day = addDaysToStamp(day, 1)) {
        const row: DayRow = { item: it, tone: "neutral" };
        if (span.total > 1) {
          // Days since the start, +1 for the 1-based label.
          let n = 1;
          for (let d = span.start; d < day; d = addDaysToStamp(d, 1)) n++;
          row.span = { day: n, total: span.total };
        }
        placed.push({ day, raw: it.when!, row });
      }
    };
    // The days gone by: what happened on them (an event), on each day
    // the `when` covered before today and not before the floor. The rest
    // of the placement is from today on.
    if (floor && span.start < today && isEvent(it)) {
      const from = span.start >= floor ? span.start : floor;
      const to = span.end < yesterday ? span.end : yesterday;
      if (from <= to) placeWhen(from, to);
    }
    if (isClosed(it) && !closedTask) {
      // A ticked or crossed event stays put on its `when` day, unjudged:
      // the day still records what happened, or what was called off.
      // Overdue is Open-only.
      if (wDay !== null) placeWhen(wDay, span.end);
      continue;
    }
    // A lingering closed task is placed here too, where it was while
    // open.
    if (wDay !== null) {
      placeWhen(wDay, span.end);
    } else if (isSlippedTask(it, today) || closedTask) {
      // A task planned for a day that has passed is still to do: Overdue,
      // by its `when`.
      overdueRows.push({
        raw: it.when,
        row: { item: it, tone: "overdue" },
      });
    }
  }
  const byRawThenCreated = (
    a: { raw: string; row: DayRow },
    b: { raw: string; row: DayRow },
  ) =>
    a.raw.localeCompare(b.raw) || a.row.item.createdAt - b.row.item.createdAt;
  overdueRows.sort(byRawThenCreated);
  placed.sort(
    (a, b) =>
      a.day.localeCompare(b.day) ||
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
    // Tomorrow / weekday / compact date, judged against today; a day
    // gone by is its date.
    into.push({
      key: p.day,
      label: formatDayLabel(p.day, today, labels, locale) ?? p.day,
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

/** Every item on the agenda once, in display order, with the key of the
 *  group its row is in: what the selection runs over. An item on more
 *  than one day (a multi-day span, or a past day and a day ahead) counts
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

/** Count what is owed or on today: Open items with a task `when` that
 *  is over, or a `when` on today, a span covering it included. Tone is
 *  overdue when any counted task is, else neutral. An event's past
 *  `when` has gone by and counts for nothing; future days are the view's
 *  business, not the badge's. Mirrors `attention_count` in the CLI's
 *  `agenda.rs`. */
export function attentionBadge(
  items: Iterable<ItemView>,
  today: string,
): AttentionBadge {
  let count = 0;
  let overdue = false;
  for (const it of items) {
    if (!isOpen(it)) continue;
    if (isSlippedTask(it, today)) {
      count++;
      overdue = true;
    } else if (it.when && coversToday(it.when, it.duration, today)) {
      count++;
    }
  }
  return { count, tone: overdue ? "overdue" : "neutral" };
}

const coversToday = (
  when: string,
  duration: number | null | undefined,
  today: string,
): boolean => {
  const span = whenSpan(when, duration);
  return span.start <= today && today <= span.end;
};

/** How far ahead of today the agenda reaches with empty days shown when
 *  nothing is placed later: two weeks, so the days ahead read as a
 *  calendar even when they are empty. */
export const AGENDA_ALL_DAYS_AHEAD = 13;

/** The agenda with every day in it, not just the days something is on
 *  (the web Upcoming view's default; "Hide empty days" in its filter
 *  menu, a device preference, turns it off): an
 *  empty group for each missing day from today through to the later of
 *  the last placed day and `today + ahead`. Days gone by are never
 *  filled: a past day shows only when something is on it. Overdue keeps
 *  its place between the days gone by and Today. `reach` (how far the
 *  agenda has been scrolled or jumped ahead) widens the range to that
 *  day; a day before the range's end changes nothing. Labels follow
 *  `groupByDay`'s rules. */
export function fillEmptyDays(
  groups: readonly DayGroup[],
  today: string,
  labels: DayGroupLabels,
  locale: string,
  ahead: number = AGENDA_ALL_DAYS_AHEAD,
  reach: string | null = null,
): DayGroup[] {
  const byDay = new Map<string, DayGroup>();
  let last = addDaysToStamp(today, ahead);
  if (reach !== null && reach > last) last = reach;
  const out: DayGroup[] = [];
  for (const g of groups) {
    if (g.key === OVERDUE_KEY) continue;
    if (g.key < today) {
      if (g.rows.length > 0) out.push(g);
      continue;
    }
    byDay.set(g.key, g);
    if (g.key > last) last = g.key;
  }
  const overdue = groups.find((g) => g.key === OVERDUE_KEY);
  for (let day = today; day <= last; day = addDaysToStamp(day, 1)) {
    if (day === today && overdue) out.push(overdue);
    const have = byDay.get(day);
    if (have) {
      out.push(have);
      continue;
    }
    out.push({
      key: day,
      label: day === today
        ? labels.today
        : (formatDayLabel(day, today, labels, locale) ?? day),
      urgency: day < today ? "past" : day === today ? "today" : "future",
      rows: [],
    });
  }
  return out;
}

/** The agenda with a section for `day` (the day the sidebar's mini
 *  calendar jumped to), empty when nothing is on it, so the jump lands
 *  on that day itself: a past day, which is otherwise never shown empty,
 *  or any day while empty days are hidden. A past day goes among the
 *  days gone by, before Overdue. `groups` comes back as it is when the
 *  day already has its section. */
export function withDay(
  groups: readonly DayGroup[],
  day: string,
  today: string,
  labels: DayGroupLabels,
  locale: string,
): DayGroup[] {
  if (groups.some((g) => g.key === day)) return groups as DayGroup[];
  const gone = day < today;
  let at = groups.findIndex((g) =>
    g.key === OVERDUE_KEY ? gone : g.key > day,
  );
  if (at < 0) at = groups.length;
  const out = groups.slice();
  out.splice(at, 0, {
    key: day,
    label: day === today
      ? labels.today
      : (formatDayLabel(day, today, labels, locale) ?? day),
    urgency: gone ? "past" : day === today ? "today" : "future",
    rows: [],
  });
  return out;
}

/** The `when` dropping `item` on the agenda day `day` writes
 *  (`spec/calendar-plan.md` "Reschedule"): the `when` moves there, the
 *  clock time kept, and a multi-day span moves its start (the `duration`
 *  is untouched, so the span keeps its length), whichever of its rows
 *  was grabbed. Null when nothing would change, or for an item the drop
 *  does not move: a closed task (it sits on the day it was closed) or
 *  one with no date. */
export function dropOnDay(item: ItemView, day: string): string | null {
  if (isBinned(item) || isClosedTask(item) || !item.when) return null;
  if (whenDay(item.when) === day) return null;
  return day + item.when.slice(10);
}

/** The locale's first day of the week, 1 (Monday) to 7 (Sunday), from
 *  `Intl.Locale` week info where the engine has it (`getWeekInfo()`,
 *  or the older `weekInfo` getter); Monday otherwise. */
export function firstDayOfWeek(locale: string): number {
  try {
    const loc = new Intl.Locale(locale) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number };
      weekInfo?: { firstDay: number };
    };
    const day = loc.getWeekInfo?.().firstDay ?? loc.weekInfo?.firstDay;
    return day && day >= 1 && day <= 7 ? day : 1;
  } catch {
    return 1;
  }
}

/** 1 (Monday) to 7 (Sunday) for a `YYYY-MM-DD` stamp. */
const isoWeekday = (stamp: string): number => {
  const [y, m, d] = stamp.split("-").map(Number);
  const wd = new Date(Date.UTC(y!, m! - 1, d!)).getUTCDay();
  return wd === 0 ? 7 : wd;
};

/** ISO 8601 week number of a `YYYY-MM-DD` stamp: the week (Monday to
 *  Sunday) holding that year's first Thursday is week 1. */
export function isoWeekNumber(stamp: string): number {
  const [y, m, d] = stamp.split("-").map(Number);
  const date = new Date(Date.UTC(y!, m! - 1, d!));
  // The Thursday of this ISO week decides its year.
  date.setUTCDate(date.getUTCDate() + 4 - isoWeekday(stamp));
  const yearStart = Date.UTC(date.getUTCFullYear(), 0, 1);
  return Math.ceil(((date.getTime() - yearStart) / 86_400_000 + 1) / 7);
}

/** The first day (`YYYY-MM-DD`) of the week holding `stamp`, weeks
 *  starting on `firstDay` (1 Monday … 7 Sunday). */
export function weekStartOf(stamp: string, firstDay: number): string {
  return addDaysToStamp(stamp, -((isoWeekday(stamp) - firstDay + 7) % 7));
}

/** Which agenda days open a week, with the number to label it: a day
 *  whose week differs from the day group before it (Overdue skipped),
 *  the first day group included. Works with or without empty days
 *  filled in: a week's first day need not be on the agenda. The number
 *  is the ISO week of the Monday inside the week, so a week starting on
 *  a Sunday or a Saturday takes the number of the days that follow. */
export function weekStarts(
  groups: readonly DayGroup[],
  firstDay: number,
): Map<string, number> {
  const out = new Map<string, number>();
  let prev: string | null = null;
  for (const g of groups) {
    if (g.key === OVERDUE_KEY) continue;
    const start = weekStartOf(g.key, firstDay);
    if (start !== prev) {
      const monday = addDaysToStamp(start, (8 - firstDay) % 7);
      out.set(g.key, isoWeekNumber(monday));
      prev = start;
    }
  }
  return out;
}
