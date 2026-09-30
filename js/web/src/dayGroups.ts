// Day grouping behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): every Open item carrying a `when` or a `deadline`, placed
// once on the earlier of the two days. An overdue deadline leads in an
// Overdue group so what is owed is visible at a glance and Today reads as
// today's plan; the group only exists while something is overdue. A past
// `when` has gone by like an event: it places nothing, and an item with
// only that is not selected for this view. Done items keep their calendar
// day via `when` only ("happens on" outlives the tick, "owed by" does
// not): they keep their slot on that day, never in Overdue and never
// placed by deadline. Binned items never appear. Pure so it can be
// unit-tested without a DOM (`test/dayGroups.test.ts`).

import { formatDeadlineBadge, whenDay } from "./format.tsx";
import {
  isBinned,
  isCancelled,
  isDone,
  isOpen,
  type ItemView,
} from "./sync/store.ts";

/** Most urgent first when comparing. */
export type DayTone = "overdue" | "warning" | "neutral";

export interface DayRow {
  item: ItemView;
  /** Which field put the row on this day. */
  placedBy: "when" | "deadline";
  tone: DayTone;
}

/** Group key of the Overdue group, which has no day of its own. */
export const OVERDUE_KEY = "overdue";

export interface DayGroup {
  /** Group key: the `YYYY-MM-DD` stamp of the day, or `OVERDUE_KEY`. */
  key: string;
  label: string;
  urgency: "overdue" | "today" | "future";
  rows: DayRow[];
}

export interface DayGroupLabels {
  overdue: string;
  today: string;
  tomorrow: string;
}

/** Bucket `items` by placement day. `today` is the local `YYYY-MM-DD`
 *  stamp everything is judged against. An Overdue group leads when any
 *  deadline is past, oldest first, then `createdAt`. Today is always
 *  present after it, empty if nothing is due, so the surface anchors on
 *  the current day.
 *
 *  Within a day, rows order by the raw string of the placing field
 *  (all-day ahead of timed), then `createdAt`; ticking a row does not
 *  move it. */
export function groupByDay(
  items: Iterable<ItemView>,
  today: string,
  labels: DayGroupLabels,
  locale: string,
): DayGroup[] {
  const placed: { day: string; raw: string; row: DayRow }[] = [];
  const overdueRows: { raw: string; row: DayRow }[] = [];
  for (const it of items) {
    // A cancelled item did not happen: unlike a done one it keeps no
    // calendar slot, so it drops out with the binned ones.
    if (isBinned(it) || isCancelled(it) || (!it.when && !it.deadline)) continue;
    // Past days are not rendered here, so a past `when` places nothing,
    // done or not.
    const wDay = it.when && whenDay(it.when) >= today ? whenDay(it.when) : null;
    const dDay = it.deadline ?? null;
    if (isDone(it)) {
      // A ticked item stays put on its `when` day, unjudged. Its deadline
      // is settled and places nothing; Overdue is Open-only.
      if (wDay === null) continue;
      placed.push({
        day: wDay,
        raw: it.when!,
        row: { item: it, placedBy: "when", tone: "neutral" },
      });
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
      placed.push({
        day: wDay,
        raw: it.when!,
        row: { item: it, placedBy: "when", tone },
      });
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
 *  deadline today, or a `when` today. Tone is the most urgent counted:
 *  overdue, else warning (a deadline today), else neutral. A past `when`
 *  has gone by like an event and counts for nothing; future days are the
 *  view's business, not the badge's. */
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
    } else if (it.when && whenDay(it.when) === today) {
      count++;
    }
  }
  return { count, tone: overdue ? "overdue" : warning ? "warning" : "neutral" };
}
