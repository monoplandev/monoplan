// The typed-date grammar behind date suggestions (`src/whenParse.ts`):
// which phrases are taken and what register value they become.
// "Now" is Tue 2026-11-03 12:00.

import { describe, expect, test } from "bun:test";

import { findWhenPhrase, phraseKey, type FindWhenOptions } from "../src/whenParse.ts";

const now = new Date(2026, 10, 3, 12, 0, 0);
const find = (text: string, extra: Partial<FindWhenOptions> = {}) =>
  findWhenPhrase(text, { now, locale: "en-AU", ...extra });

describe("findWhenPhrase", () => {
  test("a weekday with a time-of-day word is an estimated time", () => {
    const p = find("friday night");
    expect(p).toMatchObject({ start: 0, end: 12, text: "friday night" });
    expect(p?.when).toBe("2026-11-06T20:00");
    expect(p?.estimated).toBe(true);
  });

  test("morning estimates mid-morning rather than dawn", () => {
    expect(find("saturday morning")?.when).toBe("2026-11-07T09:00");
    expect(find("tonight")?.when).toBe("2026-11-03T20:00");
  });

  test("a clock time is explicit", () => {
    const p = find("tomorrow at 5pm");
    expect(p?.when).toBe("2026-11-04T17:00");
    expect(p?.estimated).toBe(false);
    expect(find("sat 3pm")?.when).toBe("2026-11-07T15:00");
    expect(find("tomorrow 17:00")?.when).toBe("2026-11-04T17:00");
    expect(find("friday night at 9")?.when).toBe("2026-11-06T21:00");
  });

  test("a bare small hour reads as afternoon, a large one as morning", () => {
    expect(find("meet at 3")?.when).toBe("2026-11-03T15:00");
    // 09:00 is behind a noon reference, so the time-only phrase lands
    // on tomorrow; the same phrase before nine would be today.
    expect(find("meet at 9")?.when).toBe("2026-11-04T09:00");
    expect(find("meet at 9", { now: new Date(2026, 10, 3, 8, 0) })?.when).toBe(
      "2026-11-03T09:00",
    );
    expect(find("noon")?.when).toBe("2026-11-03T12:00");
    expect(find("3pm")?.when).toBe("2026-11-03T15:00");
  });

  test("a day without a time is all-day", () => {
    expect(find("call mum friday")).toMatchObject({
      start: 9,
      end: 15,
      when: "2026-11-06",
      estimated: false,
    });
    expect(find("this weekend")?.when).toBe("2026-11-07");
    expect(find("in 3 days")?.when).toBe("2026-11-06");
    expect(find("14 nov")?.when).toBe("2026-11-14");
    expect(find("14/11")?.when).toBe("2026-11-14");
    expect(find("next friday")?.when).toBe("2026-11-13");
    expect(find("end of month")?.when).toBe("2026-11-30");
  });

  test("a phrase that pins no day is not taken", () => {
    expect(find("march")).toBeNull();
    expect(find("next month")).toBeNull();
    expect(find("buy 10 eggs")).toBeNull();
    expect(find("call may about the report")).toBeNull();
  });

  test("a three-letter weekday alone is a word, qualified it is a date", () => {
    expect(find("sun cream")).toBeNull();
    expect(find("wed")).toBeNull();
    // chrono's "next <weekday>" is that day of next week ("next friday"
    // above is the 13th on the same rule).
    expect(find("next sat")?.when).toBe("2026-11-14");
    expect(find("saturday")?.when).toBe("2026-11-07");
  });

  test("a bare duration is nothing; with its preposition it is a date", () => {
    // Under forwardDate chrono reads "a d" as "in a day" (tomorrow).
    expect(find("a d")).toBeNull();
    expect(find("call a d")).toBeNull();
    expect(find("1 d")).toBeNull();
    expect(find("3 days")).toBeNull();
    expect(find("half an hour")).toBeNull();
    expect(find("in a day")?.when).toBe("2026-11-04");
    expect(find("in 3 days")?.when).toBe("2026-11-06");
    expect(find("3 days later")?.when).toBe("2026-11-06");
    expect(find("3 días", { locale: "es" })).toBeNull();
    expect(find("en 3 días", { locale: "es" })?.when).toBe("2026-11-06");
  });

  test("a half-typed word or a suffixed one is nothing yet", () => {
    expect(find("call mum frida")).toBeNull();
    expect(find("tomorrows meeting")).toBeNull();
    expect(find("tonights")).toBeNull();
    expect(find("friday repo")?.text).toBe("friday");
  });

  test("the phrase ending last wins", () => {
    const p = find("tomorrow or friday");
    expect(p?.text).toBe("friday");
    expect(p?.when).toBe("2026-11-06");
  });

  test("a phrase's key folds case and spacing", () => {
    expect(phraseKey("  Friday   Night ")).toBe("friday night");
  });

  test("spanish follows the locale", () => {
    expect(find("comprar pan mañana", { locale: "es" })).toMatchObject({
      start: 12,
      end: 18,
      when: "2026-11-04",
    });
    expect(find("viernes noche", { locale: "es-ES" })).toMatchObject({
      when: "2026-11-06T20:00",
      estimated: true,
    });
    expect(find("mañana a las 5", { locale: "es" })?.when).toBe("2026-11-04T17:00");
  });

  test("blank text has no phrase", () => {
    expect(find("")).toBeNull();
    expect(find("   ")).toBeNull();
  });
});
