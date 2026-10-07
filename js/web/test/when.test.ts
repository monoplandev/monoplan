// Planned-date (`when`) coverage across the JS layers:
//  - the store hydrates `when` from the attach burst and patches it on
//    live itemWhenChanged events;
//  - the register ⇄ time-field bridge and the hour-cycle resolution that
//    drives Kobalte's TimeField;
//  - `formatWhenBadge` labels and tones.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";
import {
  formatWhenBadge,
  formatWhenTime,
  hourCycle,
  isCompleteTime,
  setTimeFormatPref,
  whenDay,
  whenFromParts,
  endTimeOf,
  endDayOffset,
  durationBetween,
  formatDurationShort,
  isValidEndDayOffset,
  MAX_DURATION_MINUTES,
  allDayEndOffset,
  allDayDurationFor,
  MAX_ALL_DAY_OFFSET,
  spanEndOffset,
  durationForTimedStart,
  whenTime,
} from "../src/format.tsx";

const DOC_ID = "00000000-0000-0000-0000-000000000000";

function engineFrom(doc: Doc): SyncEngine {
  return new SyncEngine(
    doc,
    DOC_ID,
    Dek.generate(),
    0n,
    "test",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
}

describe("store when", () => {
  test("hydrates when from the attach burst", () => {
    const doc = Doc.create();
    const id = doc.addItem("inbox", "planned");
    doc.setItemWhen(id, "2026-07-13T14:00");

    const app = createSyncedApp(engineFrom(doc));
    expect(app.state.itemsById[id]?.when).toBe("2026-07-13T14:00");
  });

  test("patches when on set, then clears it, deadline untouched", () => {
    const doc = Doc.create();
    const id = doc.addItem("inbox", "task");
    const app = createSyncedApp(engineFrom(doc));
    expect(app.state.itemsById[id]?.when).toBeUndefined();

    app.setItemDeadline(id, "2026-10-31");
    app.setItemWhen(id, "2026-12-01");
    expect(app.state.itemsById[id]?.when).toBe("2026-12-01");

    app.setItemWhen(id, "2026-12-01T09:30");
    expect(app.state.itemsById[id]?.when).toBe("2026-12-01T09:30");

    app.setItemWhen(id, null);
    expect(app.state.itemsById[id]?.when).toBeUndefined();
    expect(app.state.itemsById[id]?.deadline).toBe("2026-10-31");
  });

  test("the core rejects a malformed value", () => {
    const doc = Doc.create();
    const id = doc.addItem("inbox", "task");
    expect(() => doc.setItemWhen(id, "2026-12-01T09:30:00")).toThrow();
  });
});

describe("when ⇄ time-field bridge", () => {
  test("day and time parts", () => {
    expect(whenDay("2026-07-13")).toBe("2026-07-13");
    expect(whenDay("2026-07-13T14:05")).toBe("2026-07-13");
    expect(whenTime("2026-07-13")).toBeNull();
    expect(whenTime("2026-07-13T14:05")).toEqual({ hour: 14, minute: 5 });
  });

  test("complete / partial states", () => {
    expect(isCompleteTime({ hour: 9, minute: 0 })).toBe(true);
    expect(isCompleteTime({ hour: 9 })).toBe(false);
  });

  test("whenFromParts pads and falls back to all-day", () => {
    expect(whenFromParts("2026-07-13", { hour: 9, minute: 5 })).toBe("2026-07-13T09:05");
    expect(whenFromParts("2026-07-13", { hour: 9 })).toBe("2026-07-13");
    expect(whenFromParts("2026-07-13", null)).toBe("2026-07-13");
  });
});

describe("duration ⇄ end-time bridge", () => {
  test("endTimeOf adds and wraps past midnight", () => {
    expect(endTimeOf({ hour: 14, minute: 0 }, 90)).toEqual({ hour: 15, minute: 30 });
    expect(endTimeOf({ hour: 23, minute: 15 }, 60)).toEqual({ hour: 0, minute: 15 });
    expect(endTimeOf({ hour: 9, minute: 0 }, 24 * 60)).toEqual({ hour: 9, minute: 0 });
  });

  test("formatDurationShort", () => {
    expect(formatDurationShort(45)).toBe("45m");
    expect(formatDurationShort(120)).toBe("2h");
    expect(formatDurationShort(90)).toBe("1h 30m");
    expect(formatDurationShort(24 * 60)).toBe("1d");
    expect(formatDurationShort(2 * 24 * 60 + 180)).toBe("2d 3h");
    expect(formatDurationShort(24 * 60 + 30)).toBe("1d 30m");
  });

  test("durationBetween treats an end at or before the start as next day", () => {
    expect(durationBetween({ hour: 14, minute: 0 }, { hour: 15, minute: 30 })).toBe(90);
    expect(durationBetween({ hour: 23, minute: 0 }, { hour: 1, minute: 0 })).toBe(120);
    expect(durationBetween({ hour: 9, minute: 0 }, { hour: 9, minute: 0 })).toBe(24 * 60);
  });

  test("durationBetween on a later day keeps that day, capped at a week", () => {
    expect(durationBetween({ hour: 9, minute: 0 }, { hour: 8, minute: 0 }, 2)).toBe(2 * 1440 - 60);
    expect(durationBetween({ hour: 9, minute: 0 }, { hour: 10, minute: 0 }, 2)).toBe(2 * 1440 + 60);
    expect(durationBetween({ hour: 9, minute: 0 }, { hour: 10, minute: 0 }, 7)).toBe(MAX_DURATION_MINUTES);
  });

  test("endDayOffset counts the days the span runs past the start day", () => {
    expect(endDayOffset({ hour: 9, minute: 0 }, 60)).toBe(0);
    expect(endDayOffset({ hour: 23, minute: 0 }, 120)).toBe(1);
    expect(endDayOffset({ hour: 9, minute: 0 }, 1440)).toBe(1);
    expect(endDayOffset({ hour: 9, minute: 0 }, MAX_DURATION_MINUTES)).toBe(7);
  });

  test("end time + end day round-trip to the stored length", () => {
    const start = { hour: 13, minute: 15 };
    for (const minutes of [1, 60, 1439, 1440, 1441, 3000, MAX_DURATION_MINUTES]) {
      const end = endTimeOf(start, minutes);
      expect(durationBetween(start, end, endDayOffset(start, minutes))).toBe(minutes);
    }
  });

  test("isValidEndDayOffset fences the end-date picker", () => {
    const start = { hour: 9, minute: 0 };
    expect(isValidEndDayOffset(start, { hour: 10, minute: 0 }, -1)).toBe(false);
    // The start day only while the end is past the start on the clock.
    expect(isValidEndDayOffset(start, { hour: 10, minute: 0 }, 0)).toBe(true);
    expect(isValidEndDayOffset(start, { hour: 9, minute: 0 }, 0)).toBe(false);
    expect(isValidEndDayOffset(start, { hour: 8, minute: 0 }, 0)).toBe(false);
    // The week ceiling.
    expect(isValidEndDayOffset(start, { hour: 9, minute: 0 }, 7)).toBe(true);
    expect(isValidEndDayOffset(start, { hour: 9, minute: 1 }, 7)).toBe(false);
    expect(isValidEndDayOffset(start, { hour: 8, minute: 0 }, 7)).toBe(true);
    expect(isValidEndDayOffset(start, { hour: 10, minute: 0 }, 8)).toBe(false);
  });
});

describe("all-day spans: duration as whole days", () => {
  test("allDayEndOffset reads whole days, anything under a day as one", () => {
    expect(allDayEndOffset(null)).toBe(0);
    expect(allDayEndOffset(60)).toBe(0);
    expect(allDayEndOffset(1440)).toBe(0);
    expect(allDayEndOffset(1441)).toBe(1);
    expect(allDayEndOffset(3 * 1440)).toBe(2);
    expect(allDayEndOffset(MAX_DURATION_MINUTES)).toBe(MAX_ALL_DAY_OFFSET);
  });

  test("allDayDurationFor writes whole days, none for the start day", () => {
    expect(allDayDurationFor(0)).toBeNull();
    expect(allDayDurationFor(-1)).toBeNull();
    expect(allDayDurationFor(2)).toBe(3 * 1440);
    expect(allDayDurationFor(MAX_ALL_DAY_OFFSET)).toBe(MAX_DURATION_MINUTES);
    expect(allDayDurationFor(MAX_ALL_DAY_OFFSET + 3)).toBe(MAX_DURATION_MINUTES);
  });

  test("round-trip through the register", () => {
    for (const offset of [1, 2, MAX_ALL_DAY_OFFSET]) {
      expect(allDayEndOffset(allDayDurationFor(offset))).toBe(offset);
    }
  });

  test("spanEndOffset picks the reading from the when's shape", () => {
    expect(spanEndOffset("2026-10-06", null)).toBe(0);
    expect(spanEndOffset("2026-10-06", 3 * 1440)).toBe(2);
    expect(spanEndOffset("2026-10-06T09:00", 3 * 1440)).toBe(3);
    expect(spanEndOffset("2026-10-06T23:00", 120)).toBe(1);
    expect(spanEndOffset("2026-10-06T09:00", null)).toBe(0);
  });

  test("durationForTimedStart keeps the last day of a whole-day span", () => {
    // Tue–Thu all-day (three days) with a 09:00 start ends Thu 10:00.
    expect(durationForTimedStart(3 * 1440)).toBe(2 * 1440 + 60);
    expect(endDayOffset({ hour: 9, minute: 0 }, 2 * 1440 + 60)).toBe(2);
    // A single day, a timed length, or nothing are left alone.
    expect(durationForTimedStart(1440)).toBe(1440);
    expect(durationForTimedStart(90)).toBe(90);
    expect(durationForTimedStart(2 * 1440 + 60)).toBe(2 * 1440 + 60);
    expect(durationForTimedStart(null)).toBeNull();
  });
});

describe("hourCycle", () => {
  test("explicit preference wins over locale", () => {
    setTimeFormatPref("12h");
    expect(hourCycle("en-GB")).toBe(12);
    setTimeFormatPref("24h");
    expect(hourCycle("en-US")).toBe(24);
    setTimeFormatPref("auto");
  });

  test("auto follows the locale", () => {
    setTimeFormatPref("auto");
    expect(hourCycle("en-US")).toBe(12);
    expect(hourCycle("en-GB")).toBe(24);
  });

  test("formatted time follows the same cycle", () => {
    setTimeFormatPref("24h");
    expect(formatWhenTime("2026-07-13T14:05", "en-US")).toBe("14:05");
    setTimeFormatPref("12h");
    expect(formatWhenTime("2026-07-13T14:05", "en-GB")).toMatch(/2:05\s?PM/i);
    expect(formatWhenTime("2026-07-13T09:00", "en-GB")).toMatch(/^9\s?AM$/i);
    setTimeFormatPref("24h");
    expect(formatWhenTime("2026-07-13T09:00", "en-US")).toBe("09:00");
    setTimeFormatPref("auto");
    expect(formatWhenTime("2026-07-13", "en-US")).toBe("");
  });
});

describe("formatWhenBadge", () => {
  const labels = { today: "Today", tomorrow: "Tomorrow" };
  const today = "2026-07-07"; // a Tuesday

  test("past is the plain date, never a word or a tone", () => {
    expect(formatWhenBadge("2026-07-06", today, labels, "en-US")).toEqual({
      label: "Jul 6",
      urgency: "future",
    });
    expect(formatWhenBadge("2020-01-01", today, labels, "en-US")?.label).toBe(
      "Jan 1, 2020",
    );
  });

  test("today, tomorrow, weekday, compact date", () => {
    expect(formatWhenBadge(today, today, labels, "en-US")).toEqual({
      label: "Today",
      urgency: "today",
    });
    expect(formatWhenBadge("2026-07-08", today, labels, "en-US")?.label).toBe("Tomorrow");
    expect(formatWhenBadge("2026-07-10", today, labels, "en-US")?.label).toBe("Fri");
    expect(formatWhenBadge("2026-07-20", today, labels, "en-US")?.label).toBe("Jul 20");
  });

  test("timed values append the time", () => {
    setTimeFormatPref("24h");
    expect(formatWhenBadge("2026-07-07T09:30", today, labels, "en-US")?.label).toBe(
      "Today 09:30",
    );
    setTimeFormatPref("auto");
  });

  test("null on a malformed value", () => {
    expect(formatWhenBadge("nope", today, labels, "en-US")).toBeNull();
  });
});
