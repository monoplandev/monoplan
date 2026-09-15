// The typed time picker's suggestion grammar (`src/timeSuggest.ts`): what
// each partial query means, in both hour cycles, and which reading leads.

import { describe, expect, test } from "bun:test";

import { nearestQuarterIndex, timeSuggestions } from "../src/timeSuggest.ts";

const t = (hour: number, minute: number) => ({ hour, minute });

describe("timeSuggestions", () => {
  test("blank lists every quarter hour from midnight", () => {
    const all = timeSuggestions("", 12);
    expect(all).toHaveLength(96);
    expect(all[0]).toEqual(t(0, 0));
    expect(all[1]).toEqual(t(0, 15));
    expect(all[95]).toEqual(t(23, 45));
    expect(timeSuggestions("   ", 24)).toHaveLength(96);
  });

  test("a bare hour names both halves, daytime first", () => {
    expect(timeSuggestions("1", 12)).toEqual([t(13, 0), t(1, 0)]);
    expect(timeSuggestions("6", 12)).toEqual([t(18, 0), t(6, 0)]);
    expect(timeSuggestions("7", 12)).toEqual([t(7, 0), t(19, 0)]);
    expect(timeSuggestions("11", 12)).toEqual([t(11, 0), t(23, 0)]);
    expect(timeSuggestions("12", 12)).toEqual([t(12, 0), t(0, 0)]);
    expect(timeSuggestions("1", 24)).toEqual([t(13, 0), t(1, 0)]);
    expect(timeSuggestions("9", 24)).toEqual([t(9, 0), t(21, 0)]);
  });

  test("exact, not prefix: 1 never brings 10, 11 or 12", () => {
    expect(timeSuggestions("1", 12)).toHaveLength(2);
    expect(timeSuggestions("1:3", 12)).toHaveLength(2);
  });

  test("hours past 12 exist on a 24-hour clock only", () => {
    expect(timeSuggestions("13", 24)).toEqual([t(13, 0)]);
    expect(timeSuggestions("23", 24)).toEqual([t(23, 0)]);
    expect(timeSuggestions("13", 12)).toEqual([]);
    expect(timeSuggestions("24", 24)).toEqual([]);
  });

  test("zero is midnight on either clock", () => {
    expect(timeSuggestions("0", 12)).toEqual([t(0, 0)]);
    expect(timeSuggestions("00", 24)).toEqual([t(0, 0)]);
    expect(timeSuggestions("0:30", 24)).toEqual([t(0, 30)]);
  });

  test("a single minute digit is the tens", () => {
    expect(timeSuggestions("1:3", 12)).toEqual([t(13, 30), t(1, 30)]);
    expect(timeSuggestions("1:", 12)).toEqual([t(13, 0), t(1, 0)]);
    expect(timeSuggestions("1:7", 12)).toEqual([]);
  });

  test("two minute digits are the minute", () => {
    expect(timeSuggestions("1:03", 12)).toEqual([t(13, 3), t(1, 3)]);
    expect(timeSuggestions("1:30", 12)).toEqual([t(13, 30), t(1, 30)]);
    expect(timeSuggestions("1:60", 12)).toEqual([]);
    expect(timeSuggestions("13:45", 24)).toEqual([t(13, 45)]);
  });

  test("no separator: the trailing two digits are minutes", () => {
    expect(timeSuggestions("130", 12)).toEqual([t(13, 30), t(1, 30)]);
    expect(timeSuggestions("1330", 24)).toEqual([t(13, 30)]);
    expect(timeSuggestions("1330", 12)).toEqual([]);
    expect(timeSuggestions("930", 12)).toEqual([t(9, 30), t(21, 30)]);
    expect(timeSuggestions("12345", 12)).toEqual([]);
    expect(timeSuggestions("1.30", 12)).toEqual([t(13, 30), t(1, 30)]);
  });

  test("an AM / PM suffix keeps one reading", () => {
    expect(timeSuggestions("1p", 12)).toEqual([t(13, 0)]);
    expect(timeSuggestions("1 PM", 12)).toEqual([t(13, 0)]);
    expect(timeSuggestions("1:03am", 12)).toEqual([t(1, 3)]);
    expect(timeSuggestions("1:03 a.m.", 12)).toEqual([t(1, 3)]);
    expect(timeSuggestions("12am", 12)).toEqual([t(0, 0)]);
    expect(timeSuggestions("12pm", 12)).toEqual([t(12, 0)]);
    expect(timeSuggestions("1p", 24)).toEqual([t(13, 0)]);
    expect(timeSuggestions("13pm", 24)).toEqual([t(13, 0)]);
    expect(timeSuggestions("13am", 24)).toEqual([]);
    expect(timeSuggestions("0pm", 12)).toEqual([]);
  });

  test("anything else means nothing", () => {
    expect(timeSuggestions("x", 12)).toEqual([]);
    expect(timeSuggestions("1:3:0", 12)).toEqual([]);
    expect(timeSuggestions("1:300", 12)).toEqual([]);
    expect(timeSuggestions("noon", 12)).toEqual([]);
  });
});

describe("nearestQuarterIndex", () => {
  test("floors the stored time to its quarter-hour row", () => {
    expect(nearestQuarterIndex(t(0, 0))).toBe(0);
    expect(nearestQuarterIndex(t(9, 0))).toBe(36);
    expect(nearestQuarterIndex(t(9, 14))).toBe(36);
    expect(nearestQuarterIndex(t(9, 15))).toBe(37);
    expect(nearestQuarterIndex(t(23, 59))).toBe(95);
  });

  test("defaults to 9:00 without a time", () => {
    expect(nearestQuarterIndex(null)).toBe(36);
    expect(nearestQuarterIndex({})).toBe(36);
  });
});
