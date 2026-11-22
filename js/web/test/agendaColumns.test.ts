import { expect, test } from "bun:test";
import { columnWindow, neighbourDayRow } from "../src/agendaColumns.ts";

test("draws a pane either side of the pane", () => {
  // Pitch 100, pane 300 wide at 1000: columns 7 (700) through 16 (1600).
  expect(columnWindow(1000, 300, 50, 100)).toEqual({ start: 7, end: 16 });
});

test("clamps to the days there are", () => {
  expect(columnWindow(0, 300, 50, 100)).toEqual({ start: 0, end: 6 });
  expect(columnWindow(4900, 300, 50, 100)).toEqual({ start: 46, end: 49 });
  expect(columnWindow(99999, 300, 3, 100)).toEqual({ start: 2, end: 2 });
});

test("no days, nothing drawn", () => {
  expect(columnWindow(0, 300, 0, 100)).toEqual({ start: 0, end: -1 });
});

test("a pane not laid out yet still draws its column", () => {
  expect(columnWindow(500, 0, 50, 100)).toEqual({ start: 5, end: 5 });
});

test("← / → land on the same row of the next day", () => {
  expect(neighbourDayRow([3, 4, 5], 1, 2, 1)).toEqual({ day: 2, row: 2 });
  expect(neighbourDayRow([3, 4, 5], 1, 2, -1)).toEqual({ day: 0, row: 2 });
});

test("a shorter day takes its last row", () => {
  expect(neighbourDayRow([1, 4], 1, 3, -1)).toEqual({ day: 0, row: 0 });
});

test("empty days are passed over", () => {
  expect(neighbourDayRow([2, 0, 0, 2], 0, 1, 1)).toEqual({ day: 3, row: 1 });
  expect(neighbourDayRow([2, 0, 0, 2], 3, 0, -1)).toEqual({ day: 0, row: 0 });
});

test("no wrap past either end", () => {
  expect(neighbourDayRow([2, 2], 1, 0, 1)).toBeNull();
  expect(neighbourDayRow([2, 2], 0, 0, -1)).toBeNull();
  expect(neighbourDayRow([2, 0], 0, 0, 1)).toBeNull();
});

test("entering from outside starts at the day beside `from`", () => {
  expect(neighbourDayRow([1, 0, 2], 0, 0, 1)).toEqual({ day: 2, row: 0 });
  expect(neighbourDayRow([1, 0, 2], 1, 0, -1)).toEqual({ day: 0, row: 0 });
});
