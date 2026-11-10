// The typed date input's suggestions (`src/dateSuggest.ts`): which days a
// partial or whole phrase offers, and in what order.
// "Now" is Wed 2026-11-11 10:00.

import { describe, expect, test } from "bun:test";

import { dateSuggestions } from "../src/dateSuggest.ts";

const now = new Date(2026, 10, 11, 10, 0, 0);
const suggest = (q: string, locale = "en-AU") =>
  dateSuggestions(q, { now, locale });

describe("dateSuggestions", () => {
  test("nothing typed, or nothing a date, offers nothing", () => {
    expect(suggest("")).toEqual([]);
    expect(suggest("   ")).toEqual([]);
    expect(suggest("xyz")).toEqual([]);
    expect(suggest("45")).toEqual([]);
  });

  test("a whole phrase that names one day is that day alone", () => {
    expect(suggest("tomorrow")).toEqual(["2026-11-12"]);
    expect(suggest("2026-12-14")).toEqual(["2026-12-14"]);
    expect(suggest("in 3 days")).toEqual(["2026-11-14"]);
  });

  test("a lone weekday is its next three occurrences", () => {
    expect(suggest("friday")).toEqual(["2026-11-13", "2026-11-20", "2026-11-27"]);
    expect(suggest("next friday")).toEqual(["2026-11-20", "2026-11-27", "2026-12-04"]);
  });

  test("a partial word completes to the words a date is made of", () => {
    expect(suggest("tom")).toEqual(["2026-11-12"]);
    expect(suggest("frid")).toEqual(["2026-11-13", "2026-11-20", "2026-11-27"]);
    // today, tomorrow, Tuesday; Thursday is tomorrow again.
    expect(suggest("t")).toEqual(["2026-11-11", "2026-11-12", "2026-11-17"]);
    expect(suggest("s")).toEqual(["2026-11-14", "2026-11-15"]);
  });

  test("a three-letter weekday counts in a date field", () => {
    expect(suggest("fri")).toEqual(["2026-11-13", "2026-11-20", "2026-11-27"]);
    expect(suggest("Sat")).toEqual(["2026-11-14", "2026-11-21", "2026-11-28"]);
  });

  test("a bare day number is that day in the coming months", () => {
    expect(suggest("14")).toEqual([
      "2026-11-14",
      "2026-12-14",
      "2027-01-14",
      "2027-02-14",
      "2027-03-14",
    ]);
    // The 31st skips the months without one.
    expect(suggest("31").slice(0, 3)).toEqual(["2026-12-31", "2027-01-31", "2027-03-31"]);
  });

  test("a day and month comes round each year", () => {
    expect(suggest("14 nov")).toEqual(["2026-11-14", "2027-11-14", "2028-11-14"]);
    expect(suggest("14 n")).toEqual(["2026-11-14", "2027-11-14", "2028-11-14"]);
    // Already past this year.
    expect(suggest("10 november")[0]).toBe("2027-11-10");
    expect(suggest("14 nov 2027")).toEqual(["2027-11-14"]);
  });

  test("a clock time is dropped: the field writes the day", () => {
    expect(suggest("friday 5pm")[0]).toBe("2026-11-13");
  });

  test("Spanish completes its own words, accents optional", () => {
    expect(suggest("vie", "es-ES")).toEqual(["2026-11-13", "2026-11-20", "2026-11-27"]);
    expect(suggest("miercoles", "es-ES")[0]).toBe("2026-11-11");
    expect(suggest("14", "es-ES").slice(0, 2)).toEqual(["2026-11-14", "2026-12-14"]);
    expect(suggest("ma", "es-ES")).toEqual(["2026-11-12", "2026-11-17"]);
  });
});
