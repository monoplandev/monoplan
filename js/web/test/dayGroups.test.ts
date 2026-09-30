// Day bucketing behind the Upcoming view (`spec/calendar-plan.md`
// "Agenda"): overdue deadlines in a leading Overdue group, past whens
// dropped, placement on the earlier of `when` / `deadline`, tone
// precedence, within-day ordering, and done rows kept on their `when` day.

import { describe, expect, test } from "bun:test";

import { attentionBadge, groupByDay } from "../src/dayGroups.ts";
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

  test("done rows keep their when day and slot; binned never show", () => {
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
        // Cancelled: never placed, whatever its dates — it did not happen.
        item(
          "cancelled",
          { when: "2026-09-10", deadline: TODAY },
          { state: "cancelled" },
        ),
      ],
      TODAY,
      LABELS,
      "en",
    );
    expect(groups.map((g) => [g.key, ids(g)])).toEqual([
      [TODAY, ["done-today", "open-today", "done-early"]],
      ["2026-09-10", ["future", "done-both"]],
    ]);
    expect(groups[1]!.label).toBe("Tomorrow");
    expect(groups[1]!.rows[1]!.placedBy).toBe("when");
    expect(groups[1]!.rows[1]!.tone).toBe("neutral");
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

describe("attentionBadge", () => {
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
});
