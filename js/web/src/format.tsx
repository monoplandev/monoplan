import { createSignal } from "solid-js";

// Shared 60s tick for relative-time labels ("5m ago"). One signal, one
// interval — every Row that reads it stays fresh without spawning its
// own timer.
const [nowMs, setNowMs] = createSignal(Date.now());
setInterval(() => setNowMs(Date.now()), 60_000);
export { nowMs };

// ---------- 12 / 24-hour display preference ----------
//
// "auto" defers to the browser's locale default (what `Intl` picks for
// the active locale); "12h" / "24h" force a cycle. A per-device display
// preference like theme and language, so it lives in a cookie alongside
// them rather than in the synced doc.

export type TimeFormatPreference = "auto" | "12h" | "24h";

const TIME_FORMAT_COOKIE = "hourcycle";
const TIME_FORMAT_MAX_AGE = 31536000; // 1 year

function readTimeFormat(): TimeFormatPreference {
  try {
    const m = document.cookie.match(/(?:^|; )hourcycle=(12|24)/);
    if (m?.[1] === "12") return "12h";
    if (m?.[1] === "24") return "24h";
  } catch {}
  return "auto";
}

// Same tolerance as the reader: no `document` (tests, workers) means the
// signal still flips, the cookie just isn't persisted.
function writeTimeFormat(pref: TimeFormatPreference) {
  const attrs = (maxAge: number) => `path=/;max-age=${maxAge};SameSite=Lax`;
  try {
    if (pref === "auto") {
      document.cookie = `${TIME_FORMAT_COOKIE}=;${attrs(0)}`;
    } else {
      document.cookie = `${TIME_FORMAT_COOKIE}=${pref === "12h" ? "12" : "24"};${attrs(TIME_FORMAT_MAX_AGE)}`;
    }
  } catch {}
}

const [timeFormatPref, setTimeFormatSignal] = createSignal<TimeFormatPreference>(
  readTimeFormat(),
);
export { timeFormatPref };

export function setTimeFormatPref(pref: TimeFormatPreference): void {
  setTimeFormatSignal(pref);
  writeTimeFormat(pref);
}

// `hourCycle` rather than `hour12: false` — the latter yields "24:05" at
// midnight in some replicas; h23 / h12 are unambiguous.
function hourCycleOpts(): Intl.DateTimeFormatOptions {
  switch (timeFormatPref()) {
    case "12h":
      return { hourCycle: "h12" };
    case "24h":
      return { hourCycle: "h23" };
    default:
      return {};
  }
}

// The one place a time of day gets formatted. Every stamp below goes
// through here so the 12 / 24-hour preference applies uniformly; reading
// the preference signal inside makes callers re-render when it changes.
export function timeFormatter(locale: string): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat(locale, {
    hour: "numeric",
    minute: "2-digit",
    ...hourCycleOpts(),
  });
}

// Full date + time, for hover titles where the compact stamp isn't enough.
export function formatDateTime(ts: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
    ...hourCycleOpts(),
  }).format(new Date(ts));
}

/** Whole calendar days from `earlier` to `later`, on local parts. */
export function calendarDayDiff(later: Date, earlier: Date): number {
  const a = new Date(later.getFullYear(), later.getMonth(), later.getDate()).getTime();
  const b = new Date(earlier.getFullYear(), earlier.getMonth(), earlier.getDate()).getTime();
  return Math.round((a - b) / 86_400_000);
}

const relativeEs = {
  minutesAgo: (n: number) => `hace ${n} min`,
  hoursAgo: (n: number) => `hace ${n} h`,
  todayAt: (time: string) => `Hoy ${time}`,
  yesterdayAt: (time: string) => `Ayer ${time}`,
};

const relativeEn = {
  minutesAgo: (n: number) => `${n}m ago`,
  hoursAgo: (n: number) => `${n}h ago`,
  todayAt: (time: string) => `Today ${time}`,
  yesterdayAt: (time: string) => `Yesterday ${time}`,
};

export function formatRelative(ts: number, now: number, locale: string): string {
  const diffMs = now - ts;
  const m = locale.startsWith("es") ? relativeEs : relativeEn;
  const timeFmt = timeFormatter(locale);
  const weekdayFmt = new Intl.DateTimeFormat(locale, { weekday: "short" });
  const monthDayFmt = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
  });
  const monthDayYearFmt = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  // Under a minute reads "0m ago": `nowMs` ticks once a minute, so a
  // seconds figure would sit stale, and "just now" said nothing.
  if (diffMs < 3_600_000) return m.minutesAgo(Math.max(0, Math.floor(diffMs / 60_000)));
  if (diffMs < 86_400_000) return m.hoursAgo(Math.floor(diffMs / 3_600_000));
  const tsDate = new Date(ts);
  const nowDate = new Date(now);
  const days = calendarDayDiff(nowDate, tsDate);
  if (days === 1) return m.yesterdayAt(timeFmt.format(tsDate));
  if (days < 7) return `${weekdayFmt.format(tsDate)} ${timeFmt.format(tsDate)}`;
  if (tsDate.getFullYear() === nowDate.getFullYear()) return monthDayFmt.format(tsDate);
  return monthDayYearFmt.format(tsDate);
}

// ---------- date-only days ----------
//
// Days are floating local calendar dates stored as raw `YYYY-MM-DD`
// strings. Everything here works on local date *parts* — we never call
// `new Date("YYYY-MM-DD")`, which parses as UTC midnight and shifts the
// day backwards in negative-offset timezones.

// Local `YYYY-MM-DD` stamp for a Date, built from its local parts.
export function localDateStamp(d: Date): string {
  const y = d.getFullYear();
  const mo = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${mo}-${day}`;
}

// Today's local calendar date as `YYYY-MM-DD`. `now` defaults to the
// current instant; callers thread the shared `nowMs()` tick so the value
// rolls over at local midnight without a bespoke timer.
export function todayStamp(now: number = Date.now()): string {
  return localDateStamp(new Date(now));
}

// `stamp` shifted by `n` whole days, staying on local calendar parts.
// Used for the dialog's "Tomorrow" quick action (`addDaysToStamp(today, 1)`).
export function addDaysToStamp(stamp: string, n: number): string {
  const d = parseLocalDateParts(stamp);
  if (!d) return stamp;
  return localDateStamp(new Date(d.getFullYear(), d.getMonth(), d.getDate() + n));
}

// Parse `YYYY-MM-DD` into a local-midnight Date via explicit parts.
// Returns null for anything that isn't a well-formed stamp.
export function parseLocalDateParts(stamp: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(stamp);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
}

// `Jul 12`, gaining the year only when it falls outside `ref`'s.
function compactDate(date: Date, ref: Date, locale: string): string {
  const opts: Intl.DateTimeFormatOptions =
    date.getFullYear() === ref.getFullYear()
      ? { month: "short", day: "numeric" }
      : { month: "short", day: "numeric", year: "numeric" };
  return new Intl.DateTimeFormat(locale, opts).format(date);
}

// Compact label for a day, relative to `today` (both raw `YYYY-MM-DD`).
// Rules: today → "Today"; tomorrow → "Tomorrow"; within the next 7 days
// → short weekday; else (past days included) a compact `Jul 12` (with
// the year when it differs from today's). Labels for the fixed cases
// come from i18n so callers stay locale-correct.
export function formatDayLabel(
  day: string,
  today: string,
  labels: { today: string; tomorrow: string },
  locale: string,
): string | null {
  const target = parseLocalDateParts(day);
  const ref = parseLocalDateParts(today);
  if (!target || !ref) return null;
  const days = calendarDayDiff(target, ref);
  if (days === 0) return labels.today;
  if (days === 1) return labels.tomorrow;
  if (days > 1 && days < 7) {
    return new Intl.DateTimeFormat(locale, { weekday: "short" }).format(target);
  }
  return compactDate(target, ref, locale);
}

// ---------- planned dates (`when`) ----------
//
// A `when` is a floating `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM`
// (timed) register (`spec/calendar-plan.md`). Same local-parts rule as
// days: never `new Date(when)`.

/** Hour / minute pair of a `when`; both set for a timed value. The
 *  optional shape is what `whenTime` parses, narrowed by `isCompleteTime`. */
export interface TimeParts {
  hour?: number;
  minute?: number;
}

/** The `YYYY-MM-DD` day key of a `when` (its first ten characters). */
export function whenDay(when: string): string {
  return when.slice(0, 10);
}

/** The time part of a timed `when`, or null for an all-day one. */
/** Length the core writes beside a timed `when` that lands on an item
 *  with no duration (`DEFAULT_DURATION_MINUTES` in core `doc.rs`); the
 *  new-item buffer mirrors it so the end field shows before commit. */
export const DEFAULT_DURATION_MINUTES = 60;

export function whenTime(when: string): Required<TimeParts> | null {
  const m = /^\d{4}-\d{2}-\d{2}T(\d{2}):(\d{2})$/.exec(when);
  if (!m) return null;
  return { hour: Number(m[1]), minute: Number(m[2]) };
}

/** The local instant a timed `when` starts, in ms, or null for an
 *  all-day or malformed one. Built from explicit local parts (see
 *  `parseLocalDateParts`), never `new Date(string)`. */
export function whenStartMs(when: string): number | null {
  const t = whenTime(when);
  if (!t) return null;
  const d = parseLocalDateParts(whenDay(when));
  if (!d) return null;
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), t.hour, t.minute).getTime();
}

/** Both parts set: the pair maps to a timed register. */
export function isCompleteTime(t: TimeParts): t is Required<TimeParts> {
  return t.hour != null && t.minute != null;
}

/** Minutes in a day, the wrap for end-time arithmetic. */
const DAY_MINUTES = 24 * 60;

/** The core's ceiling on `duration`: one year of days (`spec/data-model.md`). */
export const MAX_DURATION_MINUTES = 365 * DAY_MINUTES;

/** Wall-clock end of a timed `when` that runs for `minutes`: the start
 *  plus the length, wrapped past midnight. A span longer than a day still
 *  reads as a clock time; the day it lands on is not shown. */
export function endTimeOf(start: Required<TimeParts>, minutes: number): Required<TimeParts> {
  const total = (((start.hour * 60 + start.minute + minutes) % DAY_MINUTES) + DAY_MINUTES) % DAY_MINUTES;
  return { hour: Math.floor(total / 60), minute: total % 60 };
}

/** Days past the start day that the end of a `minutes`-long span lands
 *  on: 0 for a same-day span, 1 once it crosses midnight. The end day of
 *  a timed `when` is `addDaysToStamp(whenDay(when), endDayOffset(...))`. */
export function endDayOffset(start: Required<TimeParts>, minutes: number): number {
  return Math.floor((start.hour * 60 + start.minute + minutes) / DAY_MINUTES);
}

/** Days past the start day that a timed span still covers: the day its
 *  last minute falls on. The end instant is exclusive, so a span ending
 *  at 00:00 does not reach into the day that starts then (21:00 + 3h is
 *  the one evening), where `endDayOffset` puts the end itself on it. */
export function coveredDayOffset(start: Required<TimeParts>, minutes: number): number {
  if (minutes <= 0) return 0;
  return Math.floor((start.hour * 60 + start.minute + minutes - 1) / DAY_MINUTES);
}

/** Beside an all-day `when` the length counts whole days: a span of
 *  `minutes` covers `ceil(minutes / day)` days, so its last day is that
 *  many minus one past the start day (`spec/calendar-plan.md` "Field:
 *  `duration`"). A length under a day, left behind by a timed value whose
 *  time was stripped, is a single day. Null or zero is a single day too. */
export function allDayEndOffset(minutes: number | null | undefined): number {
  if (!minutes || minutes <= 0) return 0;
  return Math.ceil(minutes / DAY_MINUTES) - 1;
}

/** The span runs past one day, which makes the item an event whatever
 *  its Event box says (the core's `spans_days`, `spec/events-plan.md`
 *  "Multi-day is an event"): an all-day `when` covering two days or
 *  more, or a timed one lasting 24 hours or more. A timed span under 24
 *  hours that crosses midnight is one sitting and stays what it is. */
export function spansDays(
  when: string | null | undefined,
  minutes: number | null | undefined,
): boolean {
  if (!when || !minutes) return false;
  return whenTime(when) ? minutes >= DAY_MINUTES : minutes > DAY_MINUTES;
}

/** The last day an all-day span can reach: a year of days, so 364 past
 *  the start. */
export const MAX_ALL_DAY_OFFSET = MAX_DURATION_MINUTES / DAY_MINUTES - 1;

/** The length to store for an all-day span whose last day is `offset`
 *  days past the start: whole days, or null for a single day (no span to
 *  record; the register is dropped). */
export function allDayDurationFor(offset: number): number | null {
  if (offset <= 0) return null;
  return Math.min(offset + 1, MAX_ALL_DAY_OFFSET + 1) * DAY_MINUTES;
}

/** Days past the start day a `when` ends on, timed or all-day, given the
 *  stored length; 0 without one. */
export function spanEndOffset(when: string, minutes: number | null | undefined): number {
  const start = whenTime(when);
  if (start) return minutes ? endDayOffset(start, minutes) : 0;
  return allDayEndOffset(minutes);
}

/** Days past the start day a `when` covers, timed or all-day: what lays
 *  an item out on days. Differs from `spanEndOffset` only for a timed
 *  span ending exactly at midnight, which stops short of the next day. */
export function spanCoverOffset(when: string, minutes: number | null | undefined): number {
  const start = whenTime(when);
  if (start) return minutes ? coveredDayOffset(start, minutes) : 0;
  return allDayEndOffset(minutes);
}

/** A whole-day length carried from an all-day span onto a timed start
 *  would end a day late (Tue–Thu as three days is 72h, so from Tue 09:00
 *  it ends Fri 09:00). Reshape it to end on the same last day: the days
 *  up to it plus the default slot. Any other length is kept as is. */
export function durationForTimedStart(minutes: number | null | undefined): number | null {
  if (!minutes || minutes < 2 * DAY_MINUTES || minutes % DAY_MINUTES !== 0) {
    return minutes ?? null;
  }
  return minutes - DAY_MINUTES + DEFAULT_DURATION_MINUTES;
}

/** Compact length: `45m`, `2h`, `1h 30m`, and past a day `1d`, `2d 3h`,
 *  `1d 30m`. */
export function formatDurationShort(minutes: number): string {
  const d = Math.floor(minutes / DAY_MINUTES);
  const h = Math.floor((minutes % DAY_MINUTES) / 60);
  const m = minutes % 60;
  const parts: string[] = [];
  if (d > 0) parts.push(`${d}d`);
  if (h > 0) parts.push(`${h}h`);
  if (m > 0 || parts.length === 0) parts.push(`${m}m`);
  return parts.join(" ");
}

/** Length in minutes from a start to an end typed as a clock time, the
 *  end landing `dayOffset` days after the start day. On the start day an
 *  end at or before the start on the clock means the next day, so 23:00
 *  to 01:00 is two hours and 09:00 to 09:00 is a full day; on a later day
 *  the clock reading is taken as is, so the day holds when the time is
 *  retyped. Capped at the core's one-year ceiling. */
export function durationBetween(
  start: Required<TimeParts>,
  end: Required<TimeParts>,
  dayOffset = 0,
): number {
  const diff = end.hour * 60 + end.minute - (start.hour * 60 + start.minute);
  const total = dayOffset * DAY_MINUTES + diff;
  return Math.min(total > 0 ? total : total + DAY_MINUTES, MAX_DURATION_MINUTES);
}

/** The length that pulls a span back onto its start day, for dropping
 *  the end date. All-day: null, a single day. Timed: the end's clock time
 *  is kept when it falls after the start on the clock (Mon 14:00 to Wed
 *  16:00 becomes 14:00 to 16:00); otherwise the default slot, shortened
 *  so a late start still ends before midnight. A span already on its
 *  start day is returned unchanged. */
export function sameDayDuration(
  start: Required<TimeParts> | null,
  minutes: number | null | undefined,
): number | null {
  if (!start) return null;
  const startMinutes = start.hour * 60 + start.minute;
  if (minutes && endDayOffset(start, minutes) === 0) return minutes;
  if (minutes) {
    const end = endTimeOf(start, minutes);
    if (isValidEndDayOffset(start, end, 0)) return durationBetween(start, end, 0);
  }
  return Math.max(1, Math.min(DEFAULT_DURATION_MINUTES, DAY_MINUTES - 1 - startMinutes));
}

/** Whether a span from `start` to `end` on the clock, ending `dayOffset`
 *  days on, is a length the register can hold: at least a minute (so the
 *  start day is out while the end is not past the start on the clock) and
 *  at most a week. Drives which days the end-date picker offers. */
export function isValidEndDayOffset(
  start: Required<TimeParts>,
  end: Required<TimeParts>,
  dayOffset: number,
): boolean {
  if (dayOffset < 0) return false;
  const diff = end.hour * 60 + end.minute - (start.hour * 60 + start.minute);
  const total = dayOffset * DAY_MINUTES + diff;
  return total >= 1 && total <= MAX_DURATION_MINUTES;
}

/** Build a `when` from a day stamp and an optional complete time. A
 *  partial or null time yields the all-day form. */
export function whenFromParts(day: string, time: TimeParts | null | undefined): string {
  if (time && isCompleteTime(time)) {
    return `${day}T${String(time.hour).padStart(2, "0")}:${String(time.minute).padStart(2, "0")}`;
  }
  return day;
}

/** The hour cycle the time pickers render in: the explicit 12h / 24h
 *  preference, or for "auto" whatever `Intl` resolves for the locale.
 *  Always passed to `TimePicker` so the picker and every formatted time
 *  in the app agree by construction. */
export function hourCycle(locale: string): 12 | 24 {
  switch (timeFormatPref()) {
    case "12h":
      return 12;
    case "24h":
      return 24;
    default: {
      const hc = new Intl.DateTimeFormat(locale, { hour: "numeric" }).resolvedOptions().hourCycle;
      return hc === "h11" || hc === "h12" ? 12 : 24;
    }
  }
}

/** A wall-clock time in the preferred cycle, for the time pickers and
 *  every `when` label. On the hour a 12-hour clock reads "1 PM" rather
 *  than "1:00 PM": the minutes carry nothing there. A 24-hour clock keeps
 *  "13:00", where a bare "13" does not read as a time. */
export function formatClockTime(t: Required<TimeParts>, locale: string): string {
  const date = new Date(2000, 0, 1, t.hour, t.minute);
  if (t.minute === 0 && hourCycle(locale) === 12) {
    return new Intl.DateTimeFormat(locale, { hour: "numeric", hourCycle: "h12" }).format(date);
  }
  return timeFormatter(locale).format(date);
}

/** Local wall-clock time of a timed `when`, in the preferred cycle; empty
 *  for an all-day value. */
export function formatWhenTime(when: string, locale: string): string {
  const t = whenTime(when);
  return t ? formatClockTime(t, locale) : "";
}

export type WhenUrgency = "today" | "future";

export interface WhenBadgeInfo {
  label: string;
  urgency: WhenUrgency;
}

// Compact label + urgency for a planned date relative to `today`. Before
// today → the compact date itself, unjudged (a `when` is never "overdue",
// and whether it slipped is not decided: the useful fact is which day).
// Today / tomorrow / weekday / compact date otherwise, as `formatDayLabel`. A
// timed value appends its wall-clock time.
export function formatWhenBadge(
  when: string,
  today: string,
  labels: { today: string; tomorrow: string },
  locale: string,
): WhenBadgeInfo | null {
  const target = parseLocalDateParts(whenDay(when));
  const ref = parseLocalDateParts(today);
  if (!target || !ref) return null;
  const days = calendarDayDiff(target, ref);
  let label: string;
  let urgency: WhenUrgency;
  if (days < 0) {
    label = compactDate(target, ref, locale);
    urgency = "future";
  } else if (days === 0) {
    label = labels.today;
    urgency = "today";
  } else if (days === 1) {
    label = labels.tomorrow;
    urgency = "future";
  } else if (days < 7) {
    label = new Intl.DateTimeFormat(locale, { weekday: "short" }).format(target);
    urgency = "future";
  } else {
    label = compactDate(target, ref, locale);
    urgency = "future";
  }
  const time = formatWhenTime(when, locale);
  return { label: time ? `${label} ${time}` : label, urgency };
}

// Done-view stamp: same calendar day as `now` → time of day; otherwise
// the date. Strips the "X minutes ago" / "Yesterday HH:MM" / "Mon HH:MM"
// noise the relative format produces, since once a Done row ages past
// today the exact moment it got ticked off isn't useful — the date is.
export function formatDoneStamp(ts: number, now: number, locale: string): string {
  const tsDate = new Date(ts);
  const nowDate = new Date(now);
  if (calendarDayDiff(nowDate, tsDate) === 0) {
    return timeFormatter(locale).format(tsDate);
  }
  if (tsDate.getFullYear() === nowDate.getFullYear()) {
    return new Intl.DateTimeFormat(locale, {
      month: "short",
      day: "numeric",
    }).format(tsDate);
  }
  return new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(tsDate);
}

// Task-dialog stamp, for the activity rows: every stamp carries its time
// of day. Today → "Today 2:05 PM"; yesterday → "Yesterday 8:06 PM";
// otherwise the locale's day and abbreviated month plus time, "9 Nov
// 8:19 AM" ("Nov 9" in en-US), the year only when it is not this one.
// The rows are end-aligned in tabular figures, so they still compare at
// a glance one under the other.
export function formatDialogStamp(ts: number, now: number, locale: string): string {
  const tsDate = new Date(ts);
  const nowDate = new Date(now);
  const time = timeFormatter(locale).format(tsDate);
  const days = calendarDayDiff(nowDate, tsDate);
  const m = locale.startsWith("es") ? relativeEs : relativeEn;
  if (days === 0) return m.todayAt(time);
  if (days === 1) return m.yesterdayAt(time);
  const date = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    ...(tsDate.getFullYear() === nowDate.getFullYear() ? {} : { year: "numeric" }),
  }).format(tsDate);
  return `${date} ${time}`;
}

// Elapsed span between two instants, for the task dialog's activity
// section ("1d 3h"). Two units at most, the larger first, the smaller
// only when it is non-zero, each a single letter: y, d, h, m, s (no
// weeks or months, whose letters would be taken for something else).
// The same in every language.
export function formatElapsed(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  const units: [number, string][] = [
    [365 * 86_400, "y"],
    [86_400, "d"],
    [3_600, "h"],
    [60, "m"],
    [1, "s"],
  ];
  const first = units.findIndex(([size]) => seconds >= size);
  if (first < 0) return "0s";
  const whole = Math.floor(seconds / units[first][0]);
  const next = units[first + 1];
  const rest = next ? Math.floor((seconds - whole * units[first][0]) / next[0]) : 0;
  return rest > 0 ? `${whole}${units[first][1]} ${rest}${next[1]}` : `${whole}${units[first][1]}`;
}
