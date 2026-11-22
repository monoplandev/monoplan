// Day bucketing behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): overdue deadlines and tasks whose `when` has passed in a
// leading Overdue group, events' past whens dropped, the `when` and the
// `deadline` each placing a row of their own (one row per item per
// group), tone, within-day ordering, and done rows kept on their `when`
// day.

import { describe, expect, test } from "bun:test";

import {
  agendaItems,
  attentionBadge,
  dropOnDay,
  fillEmptyDays,
  groupByDay,
  isoWeekNumber,
  weekStarts,
  weekStartOf,
  isDueRow,
  whenSpan,
} from "../src/dayGroups.ts";
import type { ItemView } from "../src/sync/store.ts";

const LABELS = {
  overdue: "Overdue",
  today: "Today",
  tomorrow: "Tomorrow",
};
const TODAY = "2026-09-09";

let seq = 0;
function item(
  id: string,
  dates: { when?: string; deadline?: string },
  extra: Partial<ItemView> = {},
): ItemView {
  seq += 1;
  return {
    id,
    docId: "doc-1",
    listId: "inbox",
    text: id,
    notes: "",
    state: "backlog",
    createdAt: seq,
    lifecycleAt: seq,
    ...dates,
    ...extra,
  } as ItemView;
}

/** Closed at `hour` o'clock, local, on `day`. */
function closedOn(day: string, hour = 12, state = "done"): Partial<ItemView> {
  const [y, mo, d] = day.split("-").map(Number);
  return { state, lifecycleAt: new Date(y!, mo! - 1, d!, hour).getTime() } as Partial<ItemView>;
}

const ids = (g: { rows: { item: ItemView }[] }) => g.rows.map((r) => r.item.id);

describe("groupByDay", () => {
  test("each date places its own row; overdue deadlines and slipped tasks go to Overdue", () => {
    const groups = groupByDay(
      [
        item("past-when", { when: "2026-09-01" }),
        item("overdue", { deadline: "2026-09-05" }),
        item("both-late", { when: "2026-09-20", deadline: "2026-09-08" }),
        item("both", { when: "2026-09-12", deadline: "2026-09-30" }),
        item("dl-first", { when: "2026-09-15", deadline: "2026-09-11" }),
        item("due-today", { deadline: TODAY }),
        item("undated", {}),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, g.urgency, ids(g)])).toEqual([
      ["overdue", "overdue", ["past-when", "overdue", "both-late"]],
      [TODAY, "today", ["due-today"]],
      ["2026-09-11", "future", ["dl-first"]],
      ["2026-09-12", "future", ["both"]],
      ["2026-09-15", "future", ["dl-first"]],
      ["2026-09-20", "future", ["both-late"]],
      ["2026-09-30", "future", ["both"]],
    ]);
    expect(groups[0]!.label).toBe("Overdue");
    expect(groups[0]!.rows.map((r) => [r.placedBy, r.tone])).toEqual([
      ["when", "overdue"],
      ["deadline", "overdue"],
      ["deadline", "overdue"],
    ]);
    expect(groups[1]!.rows.map((r) => r.tone)).toEqual(["warning"]);
    expect(groups[2]!.rows[0]!.placedBy).toBe("deadline");
    expect(groups[2]!.rows[0]!.tone).toBe("neutral");
    expect(groups[2]!.label).toBe("Fri");
    // The date row and the deadline row of one item, on their own days.
    expect(groups.slice(3).map((g) => [g.rows[0]!.placedBy, g.rows[0]!.tone])).toEqual([
      ["when", "neutral"],
      ["when", "neutral"],
      ["when", "neutral"],
      ["deadline", "neutral"],
    ]);
  });

  test("within a day: all-day events and deadlines, then timed, then untimed tasks", () => {
    const groups = groupByDay(
      [
        item("task-a", { when: "2026-09-12" }),
        item("t14", { when: "2026-09-12T14:00" }),
        item("due", { deadline: "2026-09-12" }),
        item("birthday", { when: "2026-09-12" }, { event: true }),
        item("gig09", { when: "2026-09-12T09:00" }, { event: true }),
        item("task-b", { when: "2026-09-12" }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    // Inside a band: raw string (all the same day), then createdAt.
    expect(ids(groups[1]!)).toEqual([
      "due",
      "birthday",
      "gig09",
      "t14",
      "task-a",
      "task-b",
    ]);
  });

  test("Overdue orders deadlines and slipped whens together, oldest first, then createdAt", () => {
    const groups = groupByDay(
      [
        item("over-b", { deadline: "2026-09-08" }),
        item("over-a", { deadline: "2026-09-08" }),
        item("over-old", { when: "2026-09-30", deadline: "2026-08-20" }),
        item("over-past-when", { when: "2026-09-01", deadline: "2026-09-02" }),
        item("past-when", { when: "2026-09-07" }),
        item("own", { when: TODAY }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      ["overdue", ["over-old", "over-past-when", "past-when", "over-b", "over-a"]],
      [TODAY, ["own"]],
      // Owed since August, and still on the day it is planned for.
      ["2026-09-30", ["over-old"]],
    ]);
    // Both dates past: the deadline places the row, it is what is owed.
    expect(groups[0]!.rows.map((r) => [r.placedBy, r.tone])).toEqual([
      ["deadline", "overdue"],
      ["deadline", "overdue"],
      ["when", "overdue"],
      ["deadline", "overdue"],
      ["deadline", "overdue"],
    ]);
  });

  test("a task's past when is Overdue whatever its deadline; a span still running is not", () => {
    const groups = groupByDay(
      [
        item("past-when", { when: "2026-09-07" }),
        item("past-timed", { when: "2026-09-08T09:00" }),
        item("past-due-today", { when: "2026-09-01", deadline: TODAY }),
        item("past-due-later", { when: "2026-09-01T09:00", deadline: "2026-09-20" }),
        // Started yesterday, runs three days: under way, not slipped.
        item("running", { when: "2026-09-08" }, { duration: 3 * 1440 }),
        // Ended yesterday.
        item("span-over", { when: "2026-09-06" }, { duration: 3 * 1440 }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [
        "overdue",
        ["past-due-today", "past-due-later", "span-over", "past-when", "past-timed"],
      ],
      // A deadline still to come keeps its own row on its day.
      [TODAY, ["past-due-today", "running"]],
      ["2026-09-10", ["running"]],
      ["2026-09-20", ["past-due-later"]],
    ]);
    expect(groups[1]!.rows[0]).toMatchObject({ placedBy: "deadline", tone: "warning" });
    for (const row of groups[0]!.rows) {
      expect(row).toMatchObject({ placedBy: "when", tone: "overdue" });
      expect(row.span).toBeUndefined();
    }
  });

  test("Today holds only its own rows, timed ahead of untimed tasks", () => {
    const groups = groupByDay(
      [
        item("own-timed", { when: "2026-09-09T10:00" }),
        item("past-new", { when: "2026-09-07T08:00" }),
        item("over-new", { deadline: "2026-09-08" }),
        item("past-old", { when: "2026-09-01" }),
        item("over-old", { deadline: "2026-08-20" }),
        item("own-allday", { when: TODAY }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(ids(groups[0]!)).toEqual(["over-old", "past-old", "past-new", "over-new"]);
    expect(ids(groups[1]!)).toEqual(["own-timed", "own-allday"]);
  });

  test("a closed task is on the day it was closed; a closed event keeps its when", () => {
    const groups = groupByDay(
      [
        item("done-today", { when: TODAY }, closedOn(TODAY, 10)),
        item("open-today", { when: TODAY }),
        // Its planned time says nothing of when it was done.
        item("done-early", { when: `${TODAY}T08:00` }, closedOn(TODAY, 9)),
        item("binned", { deadline: TODAY }, { binnedAt: 1 }),
        item("future", { when: "2026-09-10" }),
        // Planned for tomorrow, done today: today's.
        item("done-both", { when: "2026-09-10", deadline: "2026-09-05" }, closedOn(TODAY, 11)),
        item("done-deadline", { deadline: "2026-09-12" }, closedOn(TODAY, 12)),
        // Closed on a day gone by: off the calendar.
        item("done-past", { when: TODAY }, closedOn("2026-09-01")),
        item("cancelled", { when: "2026-09-10" }, closedOn(TODAY, 13, "cancelled")),
        // A clock ahead of ours: read as today.
        item("skewed", { when: TODAY }, closedOn("2026-09-11")),
        item("event-done", { when: "2026-09-10" }, { ...closedOn(TODAY), event: true }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [
        TODAY,
        ["open-today", "done-early", "done-today", "done-both", "done-deadline", "cancelled", "skewed"],
      ],
      ["2026-09-10", ["event-done", "future"]],
    ]);
    expect(groups[0]!.rows.slice(1).every((r) => r.placedBy === "done" && r.tone === "neutral")).toBe(true);
    expect(groups[1]!.rows[0]).toMatchObject({ placedBy: "when" });
  });

  test("a lingering closed task stays where it was while open", () => {
    const groups = groupByDay(
      [
        item("open-a", { when: TODAY }),
        item("ticked", { when: TODAY }, closedOn(TODAY)),
        item("open-b", { when: TODAY }),
        item("ticked-late", { when: "2026-09-01" }, closedOn(TODAY)),
        item("ticked-ahead", { when: "2026-09-10" }, closedOn(TODAY)),
      ],
      TODAY,
      LABELS,
      "en",
      undefined,
      () => true,
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      ["overdue", ["ticked-late"]],
      [TODAY, ["open-a", "ticked", "open-b"]],
      ["2026-09-10", ["ticked-ahead"]],
    ]);
  });

  test("Today leads even when empty", () => {
    const groups = groupByDay(
      [
        item("binned", { when: TODAY }, { binnedAt: 1 }),
        item("future", { when: "2026-09-10" }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [TODAY, []],
      ["2026-09-10", ["future"]],
    ]);
  });

  test("empty input still yields an empty Today", () => {
    const groups = groupByDay([], TODAY, LABELS, "en");
    expect(groups.map((g) => g.key)).toEqual([TODAY]);
  });
});

// `spec/events-plan.md`: an event is a dated item with the `event` flag.
describe("groupByDay: events", () => {
  const event = (
    id: string,
    dates: { when?: string; deadline?: string },
    extra: Partial<ItemView> = {},
  ) => item(id, dates, { event: true, ...extra });

  test("events place by the same rules as tasks", () => {
    const groups = groupByDay(
      [
        event("gig", { when: "2026-09-12T19:00" }),
        // A deadline on an event is legitimate: its own row, as usual.
        event("tickets", { when: "2026-09-12", deadline: "2026-09-10" }),
        event("owed", { when: "2026-09-12", deadline: "2026-09-01" }),
        // Went by: nothing to place. A task in the same spot is overdue.
        event("past", { when: "2026-09-01" }),
        item("slipped", { when: "2026-09-02" }),
        event("done", { when: "2026-09-12" }, { state: "done" }),
        event("cancelled", { when: "2026-09-12" }, { state: "cancelled" }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      ["overdue", ["owed", "slipped"]],
      [TODAY, []],
      ["2026-09-10", ["tickets"]],
      ["2026-09-12", ["tickets", "owed", "done", "cancelled", "gig"]],
    ]);
  });
});

describe("whenSpan", () => {
  test("one day without a duration, whole days beside an all-day start", () => {
    expect(whenSpan("2026-09-10", null)).toEqual({
      start: "2026-09-10",
      end: "2026-09-10",
      total: 1,
    });
    expect(whenSpan("2026-09-10", 3 * 1440)).toEqual({
      start: "2026-09-10",
      end: "2026-09-12",
      total: 3,
    });
    // Under a day beside an all-day start is a single day.
    expect(whenSpan("2026-09-10", 90).total).toBe(1);
  });

  test("a timed start spans by its clock end: overnight crosses into the next day", () => {
    expect(whenSpan("2026-09-10T23:00", 120)).toEqual({
      start: "2026-09-10",
      end: "2026-09-11",
      total: 2,
    });
    expect(whenSpan("2026-09-10T09:00", 120).total).toBe(1);
  });

  test("a timed span ending at midnight exactly stops on the day before", () => {
    expect(whenSpan("2026-09-10T21:00", 180)).toEqual({
      start: "2026-09-10",
      end: "2026-09-10",
      total: 1,
    });
    // One minute past midnight is on the next day.
    expect(whenSpan("2026-09-10T21:00", 181).total).toBe(2);
    // Midnight to midnight is the one day; two days on ends on the second.
    expect(whenSpan("2026-09-10T00:00", 1440).total).toBe(1);
    expect(whenSpan("2026-09-10T21:00", 1440 + 180).end).toBe("2026-09-11");
  });
});

describe("groupByDay: multi-day spans", () => {
  const spanRows = (g: { rows: { item: ItemView; span?: unknown }[] }) =>
    g.rows.map((r) => [r.item.id, r.span]);

  test("an all-day span appears on each day it covers, counting its days", () => {
    const groups = groupByDay(
      [item("conf", { when: "2026-09-10" }, { duration: 3 * 1440 })],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, spanRows(g)])).toEqual([
      [TODAY, []],
      ["2026-09-10", [["conf", { day: 1, total: 3 }]]],
      ["2026-09-11", [["conf", { day: 2, total: 3 }]]],
      ["2026-09-12", [["conf", { day: 3, total: 3 }]]],
    ]);
  });

  test("a span already under way is placed from today, with its day count intact", () => {
    const groups = groupByDay(
      [item("conf", { when: "2026-09-08" }, { duration: 3 * 1440 })],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, spanRows(g)])).toEqual([
      [TODAY, [["conf", { day: 2, total: 3 }]]],
      ["2026-09-10", [["conf", { day: 3, total: 3 }]]],
    ]);
  });

  test("an event's span that ended before today places nothing; a single day carries no span", () => {
    const groups = groupByDay(
      [
        item("over", { when: "2026-09-05" }, { duration: 3 * 1440, event: true }),
        item("single", { when: TODAY }, { duration: 60 }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, spanRows(g)])).toEqual([
      [TODAY, [["single", undefined]]],
    ]);
  });

  test("an overnight timed start is on both days; a closed event's span keeps every day", () => {
    const groups = groupByDay(
      [
        item("late", { when: `${TODAY}T23:00` }, { duration: 120 }),
        item("done-conf", { when: TODAY }, { duration: 2 * 1440, state: "done", event: true }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [TODAY, ["done-conf", "late"]],
      ["2026-09-10", ["done-conf", "late"]],
    ]);
  });

  test("a deadline is one day beside the span; an overdue one is in Overdue besides", () => {
    const groups = groupByDay(
      [
        item("dl-first", { when: "2026-09-12", deadline: "2026-09-11" }, { duration: 3 * 1440 }),
        item("owed", { when: "2026-09-10", deadline: "2026-09-01" }, { duration: 3 * 1440 }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      ["overdue", ["owed"]],
      [TODAY, []],
      ["2026-09-10", ["owed"]],
      ["2026-09-11", ["dl-first", "owed"]],
      ["2026-09-12", ["owed", "dl-first"]],
      ["2026-09-13", ["dl-first"]],
      ["2026-09-14", ["dl-first"]],
    ]);
    expect(groups[3]!.rows[0]).toMatchObject({ placedBy: "deadline" });
    expect(groups[3]!.rows[0]!.span).toBeUndefined();
  });

  test("a deadline on a day the when covers rides on that row", () => {
    const groups = groupByDay(
      [
        item("same-day", { when: "2026-09-12T10:00", deadline: "2026-09-12" }),
        item("in-span", { when: "2026-09-14", deadline: "2026-09-15" }, { duration: 3 * 1440 }),
        item("today", { when: `${TODAY}T10:00`, deadline: TODAY }),
        item("done", { when: "2026-09-12", deadline: "2026-09-12" }, { state: "done", event: true }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    // One row per item per day, all of them the `when`'s.
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [TODAY, ["today"]],
      ["2026-09-12", ["done", "same-day"]],
      ["2026-09-14", ["in-span"]],
      ["2026-09-15", ["in-span"]],
      ["2026-09-16", ["in-span"]],
    ]);
    for (const g of groups) {
      for (const r of g.rows) expect(r.placedBy).toBe("when");
    }
    // The row on the deadline's day says so; due today is the warning.
    const due = groups.flatMap((g) =>
      g.rows.filter((r) => isDueRow(r, g.key)).map((r) => [g.key, r.item.id, r.tone]),
    );
    expect(due).toEqual([
      [TODAY, "today", "warning"],
      ["2026-09-12", "same-day", "neutral"],
      ["2026-09-15", "in-span", "neutral"],
    ]);
  });
});

describe("attentionBadge", () => {
  test("a span covering today counts once; an event's that ended yesterday does not", () => {
    const badge = attentionBadge(
      [
        item("running", { when: "2026-09-08" }, { duration: 3 * 1440 }),
        item("ended", { when: "2026-09-07" }, { duration: 2 * 1440, event: true }),
        item("ahead", { when: "2026-09-10" }, { duration: 2 * 1440 }),
      ],
      TODAY,
    );
    expect(badge).toEqual({ count: 1, tone: "neutral" });
  });

  test("counts overdue deadlines, slipped tasks and today's open rows; future days count for nothing", () => {
    const badge = attentionBadge(
      [
        item("over", { deadline: "2026-09-08" }),
        item("due-today", { when: "2026-09-20", deadline: TODAY }),
        item("on-today", { when: `${TODAY}T10:00`, deadline: "2026-09-20" }),
        item("past-when", { when: "2026-09-07" }),
        item("past-when-future-due", { when: "2026-09-01", deadline: "2026-09-20" }),
        item("future", { when: "2026-09-10" }),
        item("undated", {}),
      ],
      TODAY,
    );
    expect(badge).toEqual({ count: 5, tone: "overdue" });
    // A slipped task alone turns the badge overdue.
    expect(attentionBadge([item("past-when", { when: "2026-09-07" })], TODAY)).toEqual({
      count: 1,
      tone: "overdue",
    });
  });

  test("tone is the most urgent counted: warning for a deadline today, else neutral", () => {
    expect(
      attentionBadge(
        [item("on", { when: TODAY }), item("due", { deadline: TODAY })],
        TODAY,
      ),
    ).toEqual({ count: 2, tone: "warning" });
    expect(attentionBadge([item("on", { when: TODAY })], TODAY)).toEqual({
      count: 1,
      tone: "neutral",
    });
  });

  test("Open-only: done, cancelled and binned rows do not count", () => {
    const badge = attentionBadge(
      [
        item("done", { when: TODAY }, { state: "done" }),
        item("cancelled", { deadline: "2026-09-01" }, { state: "cancelled" }),
        item("binned", { deadline: TODAY }, { binnedAt: 1 }),
      ],
      TODAY,
    );
    expect(badge).toEqual({ count: 0, tone: "neutral" });
  });

  test("an event's past `when` counts for nothing; one on today counts once", () => {
    const event = (id: string, dates: { when?: string; deadline?: string }) =>
      item(id, dates, { event: true });
    expect(
      attentionBadge(
        [
          event("past", { when: "2026-09-01" }),
          event("future", { when: "2026-09-12" }),
          event("on-today", { when: TODAY }),
          item("undated-task", {}),
        ],
        TODAY,
      ),
    ).toEqual({ count: 1, tone: "neutral" });
  });
});

describe("groupByDay with a floor (the days gone by)", () => {
  const FLOOR = "2026-08-01";
  const shape = (groups: ReturnType<typeof groupByDay>) =>
    groups.map((g) => [g.key, g.urgency, ids(g)]);

  test("past days hold events only, ahead of Overdue and Today", () => {
    const groups = groupByDay(
      [
        item("event", { when: "2026-09-02T10:00" }, { event: true }),
        item("done", { when: "2026-09-02" }, closedOn("2026-09-05")),
        item("older", { when: "2026-08-20" }, { event: true }),
        item("slipped", { when: "2026-09-03" }),
        item("late", { deadline: "2026-09-04" }),
        item("done-deadline", { deadline: "2026-09-04" }, closedOn("2026-09-05", 13)),
        item("before-floor", { when: "2026-07-31" }, { event: true }),
        item("ahead", { when: "2026-09-10" }),
      ],
      TODAY,
      LABELS,
      "en",
      FLOOR,
    );
    expect(shape(groups)).toEqual([
      ["2026-08-20", "past", ["older"]],
      ["2026-09-02", "past", ["event"]],
      ["overdue", "overdue", ["slipped", "late"]],
      [TODAY, "today", []],
      ["2026-09-10", "future", ["ahead"]],
    ]);
    expect(groups.every((g) => g.rows.every((r) => g.urgency !== "past" || r.tone === "neutral"))).toBe(true);
  });

  test("no floor, no past days", () => {
    const groups = groupByDay(
      [item("event", { when: "2026-09-02" }, { event: true })],
      TODAY,
      LABELS,
      "en",
    );
    expect(shape(groups)).toEqual([[TODAY, "today", []]]);
  });

  test("a span running through today is on its past days and from today on", () => {
    const groups = groupByDay(
      [item("trip", { when: "2026-09-08" }, { event: true, duration: 3 * 1440 })],
      TODAY,
      LABELS,
      "en",
      FLOOR,
    );
    expect(shape(groups)).toEqual([
      ["2026-09-08", "past", ["trip"]],
      [TODAY, "today", ["trip"]],
      ["2026-09-10", "future", ["trip"]],
    ]);
    expect(groups.map((g) => g.rows[0]!.span)).toEqual([
      { day: 1, total: 3 },
      { day: 2, total: 3 },
      { day: 3, total: 3 },
    ]);
    // One item, counted where it is now, not on the day gone by.
    expect(agendaItems(groups).map((r) => [r.item.id, r.group])).toEqual([
      ["trip", TODAY],
    ]);
  });

  test("an event that went by with a deadline ahead is on both days", () => {
    const groups = groupByDay(
      [
        item("both", { when: "2026-09-02", deadline: "2026-09-12" }, { event: true }),
        item("gone", { when: "2026-09-01" }, { event: true }),
      ],
      TODAY,
      LABELS,
      "en",
      FLOOR,
    );
    expect(shape(groups)).toEqual([
      ["2026-09-01", "past", ["gone"]],
      ["2026-09-02", "past", ["both"]],
      [TODAY, "today", []],
      ["2026-09-12", "future", ["both"]],
    ]);
    expect(agendaItems(groups).map((r) => [r.item.id, r.group])).toEqual([
      ["gone", "2026-09-01"],
      ["both", "2026-09-12"],
    ]);
  });
});

describe("fillEmptyDays", () => {
  test("every day from today to two weeks ahead, past days only when non-empty, Overdue before Today", () => {
    const groups = groupByDay(
      [
        item("past", { when: "2026-09-06" }, { event: true }),
        item("late", { deadline: "2026-09-01" }),
        item("soon", { when: "2026-09-11" }),
      ],
      TODAY,
      LABELS,
      "en",
      "2025-09-09",
    );
    const filled = fillEmptyDays(groups, TODAY, LABELS, "en");
    const keys = filled.map((g) => g.key);
    expect(keys.slice(0, 3)).toEqual(["2026-09-06", "overdue", TODAY]);
    expect(keys[keys.length - 1]).toBe("2026-09-22");
    expect(filled.length).toBe(1 + 1 + 14);
    expect(filled.find((g) => g.key === "2026-09-11")!.rows.length).toBe(1);
    expect(filled.find((g) => g.key === "2026-09-07")).toBeUndefined();
    expect(filled.find((g) => g.key === "2026-09-10")!.label).toBe("Tomorrow");
  });

  test("a reach day widens the range ahead, never into empty past days", () => {
    const ahead = fillEmptyDays([], TODAY, LABELS, "en", undefined, "2026-10-31");
    expect(ahead[0]!.key).toBe(TODAY);
    expect(ahead[ahead.length - 1]!.key).toBe("2026-10-31");
    const back = fillEmptyDays([], TODAY, LABELS, "en", undefined, "2026-09-01");
    expect(back[0]!.key).toBe(TODAY);
    expect(back[back.length - 1]!.key).toBe("2026-09-22");
  });

  test("reaches past two weeks to the last placed day", () => {
    const groups = groupByDay(
      [item("far", { when: "2026-10-01" })],
      TODAY,
      LABELS,
      "en",
    );
    const filled = fillEmptyDays(groups, TODAY, LABELS, "en");
    expect(filled[0]!.key).toBe(TODAY);
    expect(filled[filled.length - 1]!.key).toBe("2026-10-01");
    expect(filled.length).toBe(23);
  });
});

describe("dropOnDay", () => {
  test("moves the when, keeping the clock time", () => {
    expect(dropOnDay(item("a", { when: "2026-09-10T14:30" }), "2026-09-12")).toEqual({
      field: "when",
      value: "2026-09-12T14:30",
    });
    expect(dropOnDay(item("b", { when: "2026-09-10" }), "2026-09-01")).toEqual({
      field: "when",
      value: "2026-09-01",
    });
  });

  test("an item with both dates moves its when; deadline-only moves the deadline", () => {
    const both = item("c", { when: "2026-09-10", deadline: "2026-09-15" });
    expect(dropOnDay(both, "2026-09-11")).toEqual({ field: "when", value: "2026-09-11" });
    const due = item("d", { deadline: "2026-09-15" });
    expect(dropOnDay(due, "2026-09-11")).toEqual({ field: "deadline", value: "2026-09-11" });
  });

  test("a span moves its start; same day, closed tasks and undated items are no-ops", () => {
    const span = item("e", { when: "2026-09-08" }, { duration: 3 * 1440 });
    expect(dropOnDay(span, "2026-09-20")).toEqual({ field: "when", value: "2026-09-20" });
    expect(dropOnDay(item("f", { when: "2026-09-10T09:00" }), "2026-09-10")).toBeNull();
    expect(
      dropOnDay(item("g", { when: "2026-09-10" }, { state: "done" }), "2026-09-11"),
    ).toBeNull();
    expect(dropOnDay(item("h", {}), "2026-09-11")).toBeNull();
  });
});

describe("weeks", () => {
  test("ISO week numbers, across year ends", () => {
    expect(isoWeekNumber("2026-11-22")).toBe(47);
    expect(isoWeekNumber("2026-11-23")).toBe(48);
    expect(isoWeekNumber("2026-01-01")).toBe(1);
    expect(isoWeekNumber("2027-01-01")).toBe(53);
    expect(isoWeekNumber("2024-12-30")).toBe(1);
  });

  test("week start for Monday and Sunday weeks", () => {
    expect(weekStartOf("2026-11-22", 1)).toBe("2026-11-16");
    expect(weekStartOf("2026-11-23", 1)).toBe("2026-11-23");
    expect(weekStartOf("2026-11-22", 7)).toBe("2026-11-22");
    expect(weekStartOf("2026-11-28", 7)).toBe("2026-11-22");
  });

  test("marks the first day group of each week, Overdue skipped, gaps allowed", () => {
    const g = (key: string) => ({ key, label: key, urgency: "future" as const, rows: [] });
    const groups = [
      g("2026-11-20"),
      { ...g("overdue"), urgency: "overdue" as const },
      g("2026-11-22"),
      g("2026-11-25"),
      g("2026-12-01"),
    ];
    expect([...weekStarts(groups, 1)]).toEqual([
      ["2026-11-20", 47],
      ["2026-11-25", 48],
      ["2026-12-01", 49],
    ]);
    // Sunday weeks: the 22nd opens the week numbered by Monday the 23rd.
    expect([...weekStarts(groups, 7)]).toEqual([
      ["2026-11-20", 47],
      ["2026-11-22", 48],
      ["2026-12-01", 49],
    ]);
  });
});
