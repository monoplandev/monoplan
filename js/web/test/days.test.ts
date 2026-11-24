// Date-only day helpers in `format.tsx`: local calendar parts throughout
// (never `new Date("YYYY-MM-DD")`), and `formatDayLabel`'s
// today / tomorrow / weekday / compact-date labels.

import { expect, test, describe } from "bun:test";

import {
  addDaysToStamp,
  formatDayLabel,
  localDateStamp,
  todayStamp,
} from "../src/format.tsx";

describe("date-part helpers", () => {
  test("localDateStamp / todayStamp use local calendar parts", () => {
    // 2026-03-09, well clear of any midnight edge — same day locally.
    const d = new Date(2026, 2, 9, 13, 30);
    expect(localDateStamp(d)).toBe("2026-03-09");
    expect(todayStamp(d.getTime())).toBe("2026-03-09");
  });

  test("addDaysToStamp crosses a month boundary", () => {
    expect(addDaysToStamp("2026-01-31", 1)).toBe("2026-02-01");
    expect(addDaysToStamp("2026-03-01", -1)).toBe("2026-02-28");
  });
});

describe("formatDayLabel", () => {
  const labels = { today: "Today", tomorrow: "Tomorrow" };
  const today = "2026-07-07"; // a Tuesday

  test("a day gone by is its compact date", () => {
    expect(formatDayLabel("2026-07-06", today, labels, "en-US")).toBe("Jul 6");
    expect(formatDayLabel("2020-01-01", today, labels, "en-US")).toBe("Jan 1, 2020");
  });

  test("today and tomorrow", () => {
    expect(formatDayLabel(today, today, labels, "en-US")).toBe("Today");
    expect(formatDayLabel("2026-07-08", today, labels, "en-US")).toBe("Tomorrow");
  });

  test("within the next 7 days shows a weekday", () => {
    // 2026-07-10 is a Friday.
    expect(formatDayLabel("2026-07-10", today, labels, "en-US")).toBe("Fri");
  });

  test("further out shows a compact date", () => {
    expect(formatDayLabel("2026-07-20", today, labels, "en-US")).toBe("Jul 20");
  });

  test("null on a malformed stamp", () => {
    expect(formatDayLabel("nope", today, labels, "en-US")).toBeNull();
  });
});
