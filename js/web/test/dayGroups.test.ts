// Day bucketing behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): overdue deadlines in a leading Overdue group, past whens
// dropped, placement on the earlier of `when` / `deadline`, tone
// precedence, within-day ordering, and done rows kept on their `when` day.

import { describe, expect, test } from "bun:test";

import { attentionBadge, groupByDay, whenSpan } from "../src/dayGroups.ts";
import type { ItemView } from "../src/sync/store.ts";

const LABELS = {
  overdue: "Overdue",
  unscheduled: "Unscheduled",
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

const ids = (g: { rows: { item: ItemView }[] }) => g.rows.map((r) => r.item.id);

describe("groupByDay", () => {
  test("places on the earlier field; overdue deadlines go to Overdue, past whens drop out", () => {
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
      ["overdue", "overdue", ["overdue", "both-late"]],
      [TODAY, "today", ["due-today"]],
      ["2026-09-11", "future", ["dl-first"]],
      ["2026-09-12", "future", ["both"]],
    ]);
    expect(groups[0]!.label).toBe("Overdue");
    expect(groups[0]!.rows.map((r) => [r.placedBy, r.tone])).toEqual([
      ["deadline", "overdue"],
      ["deadline", "overdue"],
    ]);
    expect(groups[1]!.rows.map((r) => r.tone)).toEqual(["warning"]);
    expect(groups[2]!.rows[0]!.placedBy).toBe("deadline");
    expect(groups[2]!.rows[0]!.tone).toBe("neutral");
    expect(groups[3]!.rows[0]!.placedBy).toBe("when");
    expect(groups[2]!.label).toBe("Fri");
  });

  test("within a day, all-day leads timed, then createdAt", () => {
    const groups = groupByDay(
      [
        item("t14", { when: "2026-09-12T14:00" }),
        item("allday-a", { when: "2026-09-12" }),
        item("t09", { when: "2026-09-12T09:00" }),
        item("allday-b", { when: "2026-09-12" }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(ids(groups[1]!)).toEqual(["allday-a", "allday-b", "t09", "t14"]);
  });

  test("Overdue holds overdue deadlines only, oldest first, then createdAt", () => {
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
      ["overdue", ["over-old", "over-past-when", "over-b", "over-a"]],
      [TODAY, ["own"]],
    ]);
    expect(groups[0]!.rows.map((r) => [r.placedBy, r.tone])).toEqual([
      ["deadline", "overdue"],
      ["deadline", "overdue"],
      ["deadline", "overdue"],
      ["deadline", "overdue"],
    ]);
  });

  test("a past when places nothing: no Overdue group, and a later deadline places the row", () => {
    const groups = groupByDay(
      [
        item("past-when", { when: "2026-09-07" }),
        item("past-timed", { when: "2026-09-08T09:00" }),
        item("past-due-today", { when: "2026-09-01", deadline: TODAY }),
        item("past-due-later", { when: "2026-09-01T09:00", deadline: "2026-09-20" }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [TODAY, ["past-due-today"]],
      ["2026-09-20", ["past-due-later"]],
    ]);
    expect(groups[0]!.rows[0]).toMatchObject({ placedBy: "deadline", tone: "warning" });
    expect(groups[1]!.rows[0]).toMatchObject({ placedBy: "deadline", tone: "neutral" });
  });

  test("Today holds only its own rows, all-day ahead of timed", () => {
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
    expect(ids(groups[0]!)).toEqual(["over-old", "over-new"]);
    expect(ids(groups[1]!)).toEqual(["own-allday", "own-timed"]);
  });

  test("closed rows keep their when day and slot; binned never show", () => {
    const groups = groupByDay(
      [
        item("done-today", { when: TODAY }, { state: "done" }),
        item("open-today", { when: TODAY }),
        item("done-early", { when: `${TODAY}T08:00` }, { state: "done" }),
        item("binned", { deadline: TODAY }, { binnedAt: 1 }),
        item("future", { when: "2026-09-10" }),
        // Done: deadline is settled and places nothing, even when earlier.
        item(
          "done-both",
          { when: "2026-09-10", deadline: "2026-09-05" },
          { state: "done" },
        ),
        // Done with no `when`: nothing to show.
        item("done-deadline", { deadline: "2026-09-12" }, { state: "done" }),
        // Done in the past: past days are not rendered.
        item("done-past", { when: "2026-09-01" }, { state: "done" }),
        // Cancelled: the same as done. `when` keeps the slot, the
        // deadline is settled and places nothing.
        item(
          "cancelled",
          { when: "2026-09-10", deadline: TODAY },
          { state: "cancelled" },
        ),
        // Cancelled with no `when`: nothing to show.
        item(
          "cancelled-deadline",
          { deadline: "2026-09-12" },
          { state: "cancelled" },
        ),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [TODAY, ["done-today", "open-today", "done-early"]],
      ["2026-09-10", ["future", "done-both", "cancelled"]],
    ]);
    expect(groups[1]!.label).toBe("Tomorrow");
    expect(groups[1]!.rows.slice(1).map((r) => [r.placedBy, r.tone])).toEqual([
      ["when", "neutral"],
      ["when", "neutral"],
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

// `spec/events-plan.md`: an event is an item located in `events`.
describe("groupByDay: events", () => {
  const event = (
    id: string,
    dates: { when?: string; deadline?: string },
    extra: Partial<ItemView> = {},
  ) => item(id, dates, { listId: "events", ...extra });

  test("events place by the same rules as tasks", () => {
    const groups = groupByDay(
      [
        event("gig", { when: "2026-09-12T19:00" }),
        // A deadline on an event is legitimate and places it as usual.
        event("tickets", { deadline: "2026-09-10" }),
        event("owed", { deadline: "2026-09-01" }),
        // Went by: nothing to place, and not unscheduled either.
        event("past", { when: "2026-09-01" }),
        event("done", { when: "2026-09-12" }, { state: "done" }),
        event("cancelled", { when: "2026-09-12" }, { state: "cancelled" }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      ["overdue", ["owed"]],
      [TODAY, []],
      ["2026-09-10", ["tickets"]],
      ["2026-09-12", ["done", "cancelled", "gig"]],
    ]);
  });

  test("Unscheduled holds Open undated events, oldest first, between Overdue and Today", () => {
    const groups = groupByDay(
      [
        item("over", { deadline: "2026-09-01" }),
        event("older", {}),
        event("newer", {}),
        // An undated task is just a task: its list shows it.
        item("task", {}),
        event("done", {}, { state: "done" }),
        event("cancelled", {}, { state: "cancelled" }),
        event("binned", {}, { binnedAt: 1 }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, g.urgency, ids(g)])).toEqual([
      ["overdue", "overdue", ["over"]],
      ["unscheduled", "unscheduled", ["older", "newer"]],
      [TODAY, "today", []],
    ]);
    expect(groups[1]!.label).toBe("Unscheduled");
    expect(groups[1]!.rows.map((r) => [r.placedBy, r.tone])).toEqual([
      ["none", "neutral"],
      ["none", "neutral"],
    ]);
  });

  test("no Unscheduled group when every event has a date", () => {
    const groups = groupByDay(
      [event("gig", { when: "2026-09-12" })],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => g.key)).toEqual([TODAY, "2026-09-12"]);
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

  test("a span that ended before today places nothing; a single day carries no span", () => {
    const groups = groupByDay(
      [
        item("over", { when: "2026-09-05" }, { duration: 3 * 1440 }),
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

  test("an overnight timed start is on both days; closed spans keep every day", () => {
    const groups = groupByDay(
      [
        item("late", { when: `${TODAY}T23:00` }, { duration: 120 }),
        item("done-conf", { when: TODAY }, { duration: 2 * 1440, state: "done" }),
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

  test("a deadline that places the row ahead of its when is one day only; an overdue one wins outright", () => {
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
      ["2026-09-11", ["dl-first"]],
    ]);
  });
});

describe("attentionBadge", () => {
  test("a span covering today counts once; one that ended yesterday does not", () => {
    const badge = attentionBadge(
      [
        item("running", { when: "2026-09-08" }, { duration: 3 * 1440 }),
        item("ended", { when: "2026-09-07" }, { duration: 2 * 1440 }),
        item("ahead", { when: "2026-09-10" }, { duration: 2 * 1440 }),
      ],
      TODAY,
    );
    expect(badge).toEqual({ count: 1, tone: "neutral" });
  });

  test("counts overdue deadlines and today's open rows; past whens and future days count for nothing", () => {
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
    expect(badge).toEqual({ count: 3, tone: "overdue" });
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

  test("unscheduled events count at neutral tone; dated and past ones follow the usual rules", () => {
    const event = (id: string, dates: { when?: string; deadline?: string }) =>
      item(id, dates, { listId: "events" });
    expect(
      attentionBadge(
        [
          event("unscheduled", {}),
          event("past", { when: "2026-09-01" }),
          event("future", { when: "2026-09-12" }),
          item("undated-task", {}),
        ],
        TODAY,
      ),
    ).toEqual({ count: 1, tone: "neutral" });
    expect(
      attentionBadge(
        [event("unscheduled", {}), event("on-today", { when: TODAY })],
        TODAY,
      ),
    ).toEqual({ count: 2, tone: "neutral" });
  });
});
