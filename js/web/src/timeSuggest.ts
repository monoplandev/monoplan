// Suggestion rules for the task dialog's typed time picker
// (`TimePicker.tsx`). Pure: a typed query and the hour cycle in, a list
// of complete times out, so the grammar is testable without a DOM.
//
// The query is read as an hour, optionally minutes, optionally an AM / PM
// suffix, and each reading yields exactly the times it names rather than
// every time it prefixes ("1" is 1 o'clock, not 10, 11 and 12 as well):
//
//   ""        every quarter hour from midnight, in day order
//   "1"       1:00 in both halves of the day (12h: 1 AM / 1 PM; 24h:
//             01:00 / 13:00); "13" is 13:00 alone in 24h and nothing in
//             12h, where no such hour exists
//   "1:3"     1:30 in both halves: a single minute digit is the tens
//   "1:03"    1:03 in both halves
//   "130"     1:30 as well; "1330" 13:30 — the last two digits are the
//             minutes when there is no separator, so the mobile numeric
//             keypad (no colon) can still say a time. "." also separates.
//   "1p"      the PM reading alone (any of a / p / am / pm, any case)
//
// Two readings are ordered by distance from noon so the daytime one is
// under the cursor: 1 to 6 and 12 suggest PM first, 7 to 11 AM first.

import type { TimeParts } from "./format.tsx";

export type TimeSuggestion = Required<TimeParts>;

/** Minutes between rows of the untyped list. */
const STEP = 15;

const QUERY = /^\s*(\d{1,4})(?:[:.](\d{0,2}))?\s*(?:([ap])\.?\s*m?\.?)?\s*$/i;

export function timeSuggestions(
  query: string,
  cycle: 12 | 24,
): TimeSuggestion[] {
  if (query.trim() === "") return allQuarterHours();
  const m = QUERY.exec(query);
  if (!m) return [];
  let digits = m[1];
  let minuteText = m[2];
  const half = m[3]?.toLowerCase() as "a" | "p" | undefined;

  // Separator-less form: split the trailing two digits off as minutes.
  // Three or four digits only ever mean that; one or two are an hour.
  if (minuteText === undefined && digits.length > 2) {
    minuteText = digits.slice(-2);
    digits = digits.slice(0, -2);
  } else if (digits.length > 2) {
    return [];
  }
  const hour = Number(digits);

  let minute: number;
  if (minuteText === undefined || minuteText === "") {
    minute = 0;
  } else if (minuteText.length === 1) {
    minute = Number(minuteText) * 10;
  } else {
    minute = Number(minuteText);
  }
  if (minute > 59) return [];

  return hourReadings(hour, half, cycle).map((h) => ({ hour: h, minute }));
}

/** The 24-hour hours a typed hour can mean, most likely first. */
function hourReadings(
  hour: number,
  half: "a" | "p" | undefined,
  cycle: 12 | 24,
): number[] {
  if (hour > 23) return [];
  if (hour > 12) {
    // Only a 24-hour clock has these; a PM suffix is redundant, an AM
    // one contradictory.
    return cycle === 24 && half !== "a" ? [hour] : [];
  }
  if (hour === 0) {
    // "0" is midnight on either clock (a 12-hour user reaching for it
    // gets "12 AM"); a PM suffix contradicts it.
    return half === "p" ? [] : [0];
  }
  const am = hour === 12 ? 0 : hour;
  const pm = hour === 12 ? 12 : hour + 12;
  if (half === "a") return [am];
  if (half === "p") return [pm];
  return Math.abs(pm - 12) <= Math.abs(am - 12) ? [pm, am] : [am, pm];
}

function allQuarterHours(): TimeSuggestion[] {
  const out: TimeSuggestion[] = [];
  for (let t = 0; t < 24 * 60; t += STEP) {
    out.push({ hour: Math.floor(t / 60), minute: t % 60 });
  }
  return out;
}

/** Index of the untyped list's row nearest to (at or before) `time`, so
 *  the stored value is under the cursor when the picker opens blank. */
export function nearestQuarterIndex(time: TimeParts | null): number {
  if (!time || time.hour == null || time.minute == null) return 9 * 4;
  return Math.floor((time.hour * 60 + time.minute) / STEP);
}
