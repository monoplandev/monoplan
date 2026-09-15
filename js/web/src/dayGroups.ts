// Day grouping behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): every Open item carrying a `when` or a `deadline`, placed
// once on the earlier of the two days. Anything with a past date of either
// kind leads in an Overdue group so the pile that needs a decision is
// visible at a glance and Today reads as today's plan; the group only
// exists while something is past. Done / binned items never appear. Pure
// so it can be unit-tested without a DOM (`test/dayGroups.test.ts`).

import { formatDeadlineBadge, whenDay } from "./format.tsx";
import { isOpen, type ItemView } from "./sync/store.ts";

/** Most urgent first when comparing. */
export type DayTone = "overdue" | "warning" | "slipped" | "neutral";

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

const FOLD_OVERDUE = 0;
const FOLD_SLIPPED = 1;

/** Bucket `items` by placement day. `today` is the local `YYYY-MM-DD`
 *  stamp everything is judged against. An Overdue group leads when any
 *  date is past: overdue deadlines first (oldest first), then slipped
 *  whens (oldest first), then `createdAt`. Today is always present after
 *  it, empty if nothing is due, so the surface anchors on the current day.
 *
 *  Within a day, rows order by the raw string of the placing field
 *  (all-day ahead of timed), then `createdAt`. */
export function groupByDay(
  items: Iterable<ItemView>,
  today: string,
  labels: DayGroupLabels,
  locale: string,
): DayGroup[] {
  const placed: { day: string; raw: string; row: DayRow }[] = [];
  const overdueRows: { fold: number; raw: string; row: DayRow }[] = [];
  for (const it of items) {
    if (!isOpen(it) || (!it.when && !it.deadline)) continue;
    const wDay = it.when ? whenDay(it.when) : null;
    const dDay = it.deadline ?? null;
    const overdue = dDay !== null && dDay < today;
    const slipped = wDay !== null && wDay < today;
    // Any past date leaves the day ladder for the Overdue group. An overdue
    // deadline is owed now whatever `when` says and places the row (red);
    // a slipped `when` follows, placed by that `when` (muted).
    if (overdue || slipped) {
      overdueRows.push(
        overdue
          ? {
              fold: FOLD_OVERDUE,
              raw: dDay!,
              row: { item: it, placedBy: "deadline", tone: "overdue" },
            }
          : {
              fold: FOLD_SLIPPED,
              raw: it.when!,
              row: { item: it, placedBy: "when", tone: "slipped" },
            },
      );
      continue;
    }
    // Both dates are today or later: place on the earlier. `when` wins a
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
  overdueRows.sort((a, b) => a.fold - b.fold || byRawThenCreated(a, b));
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
