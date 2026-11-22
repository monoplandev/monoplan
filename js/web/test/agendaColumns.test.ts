import { expect, test } from "bun:test";
import { columnWindow } from "../src/agendaColumns.ts";

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
