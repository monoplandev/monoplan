// Day bucketing behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): tasks whose `when` has passed in a leading Overdue group,
// events' past whens dropped, tone, within-day ordering, and closed
// events kept on their `when` day.

import { describe, expect, test } from "bun:test";

import {
  agendaItems,
  attentionBadge,
  dropOnDay,
  fillEmptyDays,
  withDay,
  groupByDay,
  isoWeekNumber,
  weekStarts,
  weekStartOf,
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
  dates: { when?: string },
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
  test("the when places the row; slipped tasks go to Overdue", () => {
    const groups = groupByDay(
      [
        item("past-when", { when: "2026-09-01" }),
        item("today", { when: TODAY }),
        item("fri", { when: "2026-09-11" }),
        item("sat", { when: "2026-09-12" }),
        item("undated", {}),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, g.urgency, ids(g)])).toEqual([
      ["overdue", "overdue", ["past-when"]],
      [TODAY, "today", ["today"]],
      ["2026-09-11", "future", ["fri"]],
      ["2026-09-12", "future", ["sat"]],
    ]);
    expect(groups[0]!.label).toBe("Overdue");
    expect(groups[0]!.rows.map((r) => r.tone)).toEqual(["overdue"]);
    expect(groups[2]!.label).toBe("Fri");
    expect(groups.slice(1).map((g) => g.rows[0]!.tone)).toEqual([
      "neutral",
      "neutral",
      "neutral",
    ]);
  });

  test("within a day: all-day events, then timed, then untimed tasks", () => {
    const groups = groupByDay(
      [
        item("task-a", { when: "2026-09-12" }),
        item("t14", { when: "2026-09-12T14:00" }),
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
      "birthday",
      "gig09",
      "t14",
      "task-a",
      "task-b",
    ]);
  });

  test("Overdue orders slipped whens oldest first, then createdAt", () => {
    const groups = groupByDay(
      [
        item("over-b", { when: "2026-09-08" }),
        item("over-a", { when: "2026-09-08" }),
        item("over-old", { when: "2026-08-20" }),
        item("past-timed", { when: "2026-09-07T08:00" }),
        item("own", { when: TODAY }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      ["overdue", ["over-old", "past-timed", "over-b", "over-a"]],
      [TODAY, ["own"]],
    ]);
    expect(groups[0]!.rows.every((r) => r.tone === "overdue")).toBe(true);
  });

  test("a task's past when is Overdue; a span still running is not", () => {
    const groups = groupByDay(
      [
        item("past-when", { when: "2026-09-07" }),
        item("past-timed", { when: "2026-09-08T09:00" }),
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
      ["overdue", ["span-over", "past-when", "past-timed"]],
      [TODAY, ["running"]],
      ["2026-09-10", ["running"]],
    ]);
    for (const row of groups[0]!.rows) {
      expect(row).toMatchObject({ tone: "overdue" });
      expect(row.span).toBeUndefined();
    }
  });

  test("Today holds only its own rows, timed ahead of untimed tasks", () => {
    const groups = groupByDay(
      [
        item("own-timed", { when: "2026-09-09T10:00" }),
        item("past-new", { when: "2026-09-07T08:00" }),
        item("over-new", { when: "2026-09-08" }),
        item("past-old", { when: "2026-09-01" }),
        item("over-old", { when: "2026-08-20" }),
        item("own-allday", { when: TODAY }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(ids(groups[0]!)).toEqual(["over-old", "past-old", "past-new", "over-new"]);
    expect(ids(groups[1]!)).toEqual(["own-timed", "own-allday"]);
  });

  test("a closed task is off the agenda; a closed event keeps its when", () => {
    const groups = groupByDay(
      [
        item("done-today", { when: TODAY }, closedOn(TODAY, 10)),
        item("open-today", { when: TODAY }),
        item("done-early", { when: `${TODAY}T08:00` }, closedOn(TODAY, 9)),
        item("binned", { when: TODAY }, { binnedAt: 1 }),
        item("future", { when: "2026-09-10" }),
        item("done-ahead", { when: "2026-09-10" }, closedOn(TODAY, 11)),
        item("done-past", { when: TODAY }, closedOn("2026-09-01")),
        item("cancelled", { when: "2026-09-10" }, closedOn(TODAY, 13, "cancelled")),
        item("skewed", { when: TODAY }, closedOn("2026-09-11")),
        item("event-done", { when: "2026-09-10" }, { ...closedOn(TODAY), event: true }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [TODAY, ["open-today"]],
      ["2026-09-10", ["event-done", "future"]],
    ]);
    expect(groups[1]!.rows[0]).toMatchObject({ tone: "neutral" });
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
    dates: { when?: string },
    extra: Partial<ItemView> = {},
  ) => item(id, dates, { event: true, ...extra });

  test("events place by the same rules as tasks", () => {
    const groups = groupByDay(
      [
        event("gig", { when: "2026-09-12T19:00" }),
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
      ["overdue", ["slipped"]],
      [TODAY, []],
      ["2026-09-12", ["done", "cancelled", "gig"]],
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

  test("counts slipped tasks and today's open rows; future days count for nothing", () => {
    const badge = attentionBadge(
      [
        item("on-today", { when: `${TODAY}T10:00` }),
        item("all-day-today", { when: TODAY }),
        item("past-when", { when: "2026-09-07" }),
        item("future", { when: "2026-09-10" }),
        item("undated", {}),
      ],
      TODAY,
    );
    expect(badge).toEqual({ count: 3, tone: "overdue" });
    // A slipped task alone turns the badge overdue.
    expect(attentionBadge([item("past-when", { when: "2026-09-07" })], TODAY)).toEqual({
      count: 1,
      tone: "overdue",
    });
  });

  test("without a slipped task the tone is neutral", () => {
    expect(attentionBadge([item("on", { when: TODAY })], TODAY)).toEqual({
      count: 1,
      tone: "neutral",
    });
  });

  test("Open-only: done, cancelled and binned rows do not count", () => {
    const badge = attentionBadge(
      [
        item("done", { when: TODAY }, { state: "done" }),
        item("cancelled", { when: "2026-09-01" }, { state: "cancelled" }),
        item("binned", { when: TODAY }, { binnedAt: 1 }),
      ],
      TODAY,
    );
    expect(badge).toEqual({ count: 0, tone: "neutral" });
  });

  test("an event's past `when` counts for nothing; one on today counts once", () => {
    const event = (id: string, dates: { when?: string }) =>
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
      ["overdue", "overdue", ["slipped"]],
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

  test("an event gone by counts at its past day", () => {
    const groups = groupByDay(
      [
        item("gone", { when: "2026-09-01" }, { event: true }),
        item("ahead", { when: "2026-09-12" }, { event: true }),
      ],
      TODAY,
      LABELS,
      "en",
      FLOOR,
    );
    expect(shape(groups)).toEqual([
      ["2026-09-01", "past", ["gone"]],
      [TODAY, "today", []],
      ["2026-09-12", "future", ["ahead"]],
    ]);
    expect(agendaItems(groups).map((r) => [r.item.id, r.group])).toEqual([
      ["gone", "2026-09-01"],
      ["ahead", "2026-09-12"],
    ]);
  });
});

describe("fillEmptyDays", () => {
  test("every day from today to two weeks ahead, past days only when non-empty, Overdue before Today", () => {
    const groups = groupByDay(
      [
        item("past", { when: "2026-09-06" }, { event: true }),
        item("late", { when: "2026-09-01" }),
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

  test("a picked day gets its own empty section, a past one before Overdue", () => {
    const groups = groupByDay(
      [
        item("past", { when: "2026-09-06" }, { event: true }),
        item("late", { when: "2026-09-01" }),
        item("soon", { when: "2026-09-11" }),
      ],
      TODAY,
      LABELS,
      "en",
      "2025-09-09",
    );
    const keys = (day: string) =>
      withDay(groups, day, TODAY, LABELS, "en").map((g) => g.key);
    expect(keys("2026-09-07")).toEqual([
      "2026-09-06", "2026-09-07", "overdue", TODAY, "2026-09-11",
    ]);
    expect(keys("2026-09-02")[0]).toBe("2026-09-02");
    expect(keys("2026-09-10")).toEqual([
      "2026-09-06", "overdue", TODAY, "2026-09-10", "2026-09-11",
    ]);
    expect(keys("2026-12-01").at(-1)).toBe("2026-12-01");
    const past = withDay(groups, "2026-09-07", TODAY, LABELS, "en")[1]!;
    expect(past.urgency).toBe("past");
    expect(past.rows).toEqual([]);
    // Already there: the same groups back.
    expect(withDay(groups, "2026-09-11", TODAY, LABELS, "en")).toBe(groups);
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
    expect(dropOnDay(item("a", { when: "2026-09-10T14:30" }), "2026-09-12")).toBe(
      "2026-09-12T14:30",
    );
    expect(dropOnDay(item("b", { when: "2026-09-10" }), "2026-09-01")).toBe("2026-09-01");
  });

  test("a span moves its start; same day, closed tasks and undated items are no-ops", () => {
    const span = item("e", { when: "2026-09-08" }, { duration: 3 * 1440 });
    expect(dropOnDay(span, "2026-09-20")).toBe("2026-09-20");
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
