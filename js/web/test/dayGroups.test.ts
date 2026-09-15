// Day bucketing behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): past dates of either kind in a leading Overdue group,
// placement on the earlier of `when` / `deadline`, tone precedence, and
// within-day ordering.

import { describe, expect, test } from "bun:test";

import { groupByDay } from "../src/dayGroups.ts";
import type { ItemView } from "../src/sync/store.ts";

const LABELS = { overdue: "Overdue", today: "Today", tomorrow: "Tomorrow" };
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
  test("places on the earlier field; past dates go to Overdue", () => {
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
      ["overdue", "overdue", ["overdue", "both-late", "past-when"]],
      [TODAY, "today", ["due-today"]],
      ["2026-09-11", "future", ["dl-first"]],
      ["2026-09-12", "future", ["both"]],
    ]);
    expect(groups[0]!.label).toBe("Overdue");
    expect(groups[0]!.rows.map((r) => [r.placedBy, r.tone])).toEqual([
      ["deadline", "overdue"],
      ["deadline", "overdue"],
      ["when", "neutral"],
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

  test("Overdue leads only when something is past: deadlines oldest first, then past whens, then createdAt", () => {
    const groups = groupByDay(
      [
        item("over-b", { deadline: "2026-09-08" }),
        item("over-a", { deadline: "2026-09-08" }),
        item("over-old", { when: "2026-09-30", deadline: "2026-08-20" }),
        item("past-b", { when: "2026-09-07" }),
        item("past-a", { when: "2026-09-07" }),
        item("past-old", { when: "2026-09-01T09:00", deadline: "2026-09-20" }),
        item("own", { when: TODAY }),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [
        "overdue",
        ["over-old", "over-b", "over-a", "past-old", "past-b", "past-a"],
      ],
      [TODAY, ["own"]],
    ]);
    expect(groups[0]!.rows.map((r) => r.tone)).toEqual([
      "overdue",
      "overdue",
      "overdue",
      "neutral",
      "neutral",
      "neutral",
    ]);
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
    expect(ids(groups[0]!)).toEqual([
      "over-old",
      "over-new",
      "past-old",
      "past-new",
    ]);
    expect(ids(groups[1]!)).toEqual(["own-allday", "own-timed"]);
  });

  test("skips done and binned items; Today leads even when empty", () => {
    const groups = groupByDay(
      [
        item("done", { when: TODAY }, { state: "done" }),
        item("binned", { deadline: TODAY }, { binnedAt: 1 }),
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
    expect(groups[1]!.label).toBe("Tomorrow");
  });

  test("empty input still yields an empty Today", () => {
    const groups = groupByDay([], TODAY, LABELS, "en");
    expect(groups.map((g) => g.key)).toEqual([TODAY]);
  });
});
