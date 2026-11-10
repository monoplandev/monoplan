// Suggestion rules for the task dialog's typed date input
// (`WhenField.tsx`). Pure: the typed text and a reference instant in, a
// short list of days out, so the rules are testable without a DOM. The
// reading of a phrase is `whenParse.ts`'s, the same grammar the title
// tokens use; this module only decides which phrases to put to it and how
// one reading becomes a few rows.
//
//   - The text as typed is read first, so what it already says leads the
//     list ("tomorrow", "14 nov", "next friday").
//   - A trailing partial word is completed against the words a date is
//     made of (today, tomorrow, weekday and month names), since the parser
//     only takes whole words: "t" offers today, tomorrow, Tuesday and
//     Thursday, "14 n" the 14th of November. A finished three-letter
//     weekday counts too ("fri"): the title grammar passes it over as a
//     likely word of the title, which a date field has none of.
//   - A bare day number ("14") is that day in each coming month.
//   - A phrase that comes round again fills the list when it is the only
//     reading: "friday" is the next three Fridays, "14 nov" the next three
//     years' 14th of November. With several readings each gives its first
//     day only.
//
// Only the day of a reading is kept: the field writes the date, and the
// time has its own row.

import { collapseWhitespace, findWhenPhrase, type WhenPhrase } from "./whenParse.ts";

/** Rows shown at most. */
const LIMIT = 5;
/** Occurrences of a lone repeating phrase. */
const REPEATS = 3;

interface Vocabulary {
  /** Whole words that stand for a day on their own. */
  days: string[];
  months: string[];
  /** A day number and a month name as a phrase the parser reads. */
  dayOfMonth: (day: string, month: string) => string;
}

const VOCABULARY: Record<"en" | "es", Vocabulary> = {
  en: {
    days: [
      "today", "tomorrow",
      "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
    ],
    months: [
      "january", "february", "march", "april", "may", "june", "july",
      "august", "september", "october", "november", "december",
    ],
    dayOfMonth: (day, month) => `${day} ${month}`,
  },
  es: {
    days: [
      "hoy", "mañana",
      "lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo",
    ],
    months: [
      "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio",
      "agosto", "septiembre", "octubre", "noviembre", "diciembre",
    ],
    dayOfMonth: (day, month) => `${day} de ${month}`,
  },
};

const pad = (n: number) => String(n).padStart(2, "0");

/** Case and accents folded, so "miercoles" finds "miércoles". */
const fold = (s: string) =>
  s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/** `stamp` moved on by whole days or years; null when the day does not
 *  exist there (29 Feb outside a leap year). */
function shift(stamp: string, by: { days?: number; years?: number }): string | null {
  const [y, m, d] = stamp.split("-").map(Number);
  const year = y + (by.years ?? 0);
  const date = new Date(year, m - 1, d + (by.days ?? 0));
  if (by.years && (date.getMonth() !== m - 1 || date.getDate() !== d)) return null;
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The phrases to put to the parser for `text`, most literal first. */
function candidates(text: string, now: Date, v: Vocabulary): string[] {
  const out = [text];
  if (/^\d{1,2}$/.test(text)) {
    for (let i = 0; i < 12; i++) {
      out.push(v.dayOfMonth(text, v.months[(now.getMonth() + i) % 12]));
    }
    return out;
  }
  const tail = /^(.*?)(\p{L}+)$/u.exec(text);
  if (!tail) return out;
  const [, head, word] = tail;
  const prefix = fold(word);
  for (const w of [...v.days, ...v.months]) {
    const folded = fold(w);
    if (folded.startsWith(prefix) && (folded !== prefix || w !== word.toLowerCase())) {
      out.push(head + w);
    }
  }
  return out;
}

/** The days `query` could mean as `YYYY-MM-DD`, best reading first; empty
 *  when it names none. */
export function dateSuggestions(
  query: string,
  opts: { now: Date; locale: string },
): string[] {
  const text = collapseWhitespace(query);
  if (!text) return [];
  const grammar = /^es\b/i.test(opts.locale) ? "es" : "en";
  const phrases: WhenPhrase[] = [];
  const seen = new Set<string>();
  for (const c of candidates(text, opts.now, VOCABULARY[grammar])) {
    const p = findWhenPhrase(c, opts);
    if (!p) continue;
    const day = p.when.slice(0, 10);
    if (seen.has(day)) continue;
    seen.add(day);
    phrases.push(p);
    if (phrases.length === LIMIT) break;
  }
  const days = phrases.map((p) => p.when.slice(0, 10));
  const only = phrases.length === 1 ? phrases[0] : null;
  if (only?.repeat) {
    for (let i = 1; i < REPEATS; i++) {
      const next = shift(
        days[0],
        only.repeat === "week" ? { days: 7 * i } : { years: i },
      );
      if (next) days.push(next);
    }
  }
  return days;
}
