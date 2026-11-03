// Natural-language date phrases typed into a title ("friday night",
// "tomorrow at 5pm", "14 nov"), found and resolved into the `when`
// register's floating form (`spec/calendar-plan.md`). Pure: text and a
// reference instant in, a phrase with its offsets and value out, so the
// grammar decisions are testable without a DOM. `whenToken.ts` turns a
// found phrase into the inline token the editors show.
//
// The recogniser is chrono-node's casual English / Spanish parser, picked
// by locale. Chrono does the heavy lifting (weekdays, relative days,
// month names, numeric dates, clock times) and only matches whole words,
// so "frida" and "tomorrows" are nothing to it. This module decides which
// of its matches are worth acting on and how a match becomes a register
// value:
//
//   - A match counts only when it pins a day (an explicit day, a weekday,
//     a relative day such as "tomorrow" or "in 3 days") or an explicit
//     clock time. A bare month name ("march") or "next month" fixes
//     neither and is ignored, which also sidelines ordinary words chrono
//     happens to know.
//   - A three-letter weekday on its own ("sun", "wed", "sat") is ignored:
//     too often a word of the title. "sat 3pm" or "saturday" still count.
//   - A phrase the user has rejected (`exclude`, by its exact words) is
//     passed over; a longer phrase that extends it is not.
//   - With an `anchor` (the token already standing), only a match that
//     starts at or before the anchor's start and reaches into it counts:
//     the phrase as it now reads, grown ("friday" → "friday night"),
//     shrunk ("tomorrow at 5p" is "tomorrow" until the "m" lands) or
//     gone. Another phrase elsewhere in the text is left alone.
//   - Otherwise, of several candidates the one ending last wins (dates
//     trail the title), the longest when they end together.
//   - An explicit hour gives a timed `when`. A time-of-day word ("night",
//     "morning") gives an *estimated* time from the table below, not
//     chrono's own reading (its "night" is 22:00, late for a plan); the
//     word is matched in the phrase, since chrono also implies an hour
//     on phrases that said nothing about time. Neither gives all-day.
//   - A bare hour from 1 to 6 with no am / pm reads as afternoon
//     ("at 3" is 15:00), the ordering `timeSuggest.ts` uses for the same
//     ambiguity; 7 to 11 stay morning. A phrase that is only a time
//     ("at 3", "3pm") is today if that time is still ahead, else
//     tomorrow, decided after the adjustment.
//
// The register value is built from chrono's components, never from a
// `Date` it resolves, so the floating wall-clock reading is preserved
// in every zone (`spec/calendar-plan.md`, "Floating").

import * as chronoEn from "chrono-node/en";
import * as chronoEs from "chrono-node/es";

export interface WhenPhrase {
  /** Character offsets of the phrase in the text, `[start, end)`. */
  start: number;
  end: number;
  /** The phrase as typed. */
  text: string;
  /** Normalised register value: `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`. */
  when: string;
  /** The time part came from a time-of-day word, not a clock time. */
  estimated: boolean;
}

export interface FindWhenOptions {
  /** The reference instant, local wall-clock. */
  now: Date;
  /** BCP 47 tag; picks the Spanish grammar for `es*`, English otherwise. */
  locale: string;
  /** Phrases not to take, as `phraseKey` renders them. */
  exclude?: ReadonlySet<string>;
  /** The range a standing token occupies: only a match that starts at
   *  or before `start` and ends after it counts. */
  anchor?: { start: number; end: number } | null;
}

/** Time-of-day words and the hour each estimates, per grammar. Matched
 *  against the phrase chrono took, so only a word chrono read as part of
 *  the date counts ("noche" in "viernes noche", not in "noche de paz"). */
const TIME_OF_DAY: Record<"en" | "es", { re: RegExp; hour: number }[]> = {
  en: [
    { re: /\bmorning\b/i, hour: 9 },
    { re: /\bafternoon\b/i, hour: 15 },
    { re: /\bevening\b/i, hour: 18 },
    { re: /\b(?:night|tonight)\b/i, hour: 20 },
  ],
  es: [
    { re: /\b(?:por|de|en) la mañana\b/i, hour: 9 },
    { re: /\btarde\b/i, hour: 15 },
    { re: /\bnoche\b/i, hour: 20 },
  ],
};

const WEEKDAY_ABBREVIATION = /^[a-záéíóú]{3}$/i;

type ParsedResult = ReturnType<typeof chronoEn.casual.parse>[number];

const pad = (n: number) => String(n).padStart(2, "0");

function grammarFor(locale: string): "en" | "es" {
  return /^es\b/i.test(locale) ? "es" : "en";
}

function parserFor(grammar: "en" | "es") {
  return grammar === "es" ? chronoEs.casual : chronoEn.casual;
}

const localDate = (d: Date) =>
  `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** The form a phrase is remembered in once rejected: case and
 *  whitespace folded, so retyping it the same way is the same phrase. */
export function phraseKey(text: string): string {
  return collapseWhitespace(text).toLowerCase();
}

/** Whether a chrono match pins a day or a clock time. */
function anchored(r: ParsedResult): boolean {
  const s = r.start;
  if (s.isCertain("hour")) return true;
  if (!s.isCertain("day") && !s.isCertain("weekday")) return false;
  // A lone three-letter weekday is a word of the title until it is
  // qualified ("sat 3pm", "next sat") or spelled out.
  return !(s.isCertain("weekday") && !s.isCertain("day") && WEEKDAY_ABBREVIATION.test(r.text));
}

function registerValue(
  r: ParsedResult,
  now: Date,
  grammar: "en" | "es",
): { when: string; estimated: boolean } | null {
  const s = r.start;
  const year = s.get("year");
  const month = s.get("month");
  const day = s.get("day");
  if (year == null || month == null || day == null) return null;
  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  let date = `${year}-${pad(month)}-${pad(day)}`;
  if (s.isCertain("hour")) {
    let hour = s.get("hour") ?? 0;
    const minute = s.get("minute") ?? 0;
    if (!s.isCertain("meridiem") && hour >= 1 && hour <= 6) hour += 12;
    if (!s.isCertain("day") && !s.isCertain("weekday")) {
      // Time only: chrono picked the day for its own reading of the
      // hour; pick it again for the adjusted one.
      const ahead = hour * 60 + minute >= now.getHours() * 60 + now.getMinutes();
      date = localDate(
        ahead
          ? now
          : new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1),
      );
    }
    return { when: `${date}T${pad(hour)}:${pad(minute)}`, estimated: false };
  }
  const word = TIME_OF_DAY[grammar].find((w) => w.re.test(r.text));
  if (!word) return { when: date, estimated: false };
  return { when: `${date}T${pad(word.hour)}:00`, estimated: true };
}

/** The date phrase to act on in `text`, or null when there is none. */
export function findWhenPhrase(text: string, opts: FindWhenOptions): WhenPhrase | null {
  if (!text.trim()) return null;
  const grammar = grammarFor(opts.locale);
  const results = parserFor(grammar).parse(text, opts.now, { forwardDate: true });
  const anchor = opts.anchor ?? null;
  let best: WhenPhrase | null = null;
  for (const r of results) {
    const start = r.index;
    const end = start + r.text.length;
    if (anchor && (start > anchor.start || end <= anchor.start)) continue;
    if (!anchored(r)) continue;
    if (opts.exclude?.has(phraseKey(r.text))) continue;
    const value = registerValue(r, opts.now, grammar);
    if (!value) continue;
    if (best) {
      // Against an anchor the longest reading of the phrase wins; free,
      // the one ending last, then the longest.
      const better = anchor
        ? r.text.length > best.text.length
        : end > best.end || (end === best.end && r.text.length > best.text.length);
      if (!better) continue;
    }
    best = { start, end, text: r.text, ...value };
  }
  return best;
}

/** `text` with the phrase cut out and the seam closed: whitespace runs
 *  collapsed to one space, ends trimmed. */
export function titleWithoutPhrase(text: string, phrase: { start: number; end: number }): string {
  return collapseWhitespace(text.slice(0, phrase.start) + " " + text.slice(phrase.end));
}

export function collapseWhitespace(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}
