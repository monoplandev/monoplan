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
// midnight in some engines; h23 / h12 are unambiguous.
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

function calendarDayDiff(later: Date, earlier: Date): number {
  const a = new Date(later.getFullYear(), later.getMonth(), later.getDate()).getTime();
  const b = new Date(earlier.getFullYear(), earlier.getMonth(), earlier.getDate()).getTime();
  return Math.round((a - b) / 86_400_000);
}

const relativeEs = {
  justNow: "ahora mismo",
  minutesAgo: (n: number) => `hace ${n} min`,
  hoursAgo: (n: number) => `hace ${n} h`,
  yesterdayAt: (time: string) => `Ayer ${time}`,
};

const relativeEn = {
  justNow: "just now",
  minutesAgo: (n: number) => `${n}m ago`,
  hoursAgo: (n: number) => `${n}h ago`,
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
  if (diffMs < 60_000) return m.justNow;
  if (diffMs < 3_600_000) return m.minutesAgo(Math.floor(diffMs / 60_000));
  if (diffMs < 86_400_000) return m.hoursAgo(Math.floor(diffMs / 3_600_000));
  const tsDate = new Date(ts);
  const nowDate = new Date(now);
  const days = calendarDayDiff(nowDate, tsDate);
  if (days === 1) return m.yesterdayAt(timeFmt.format(tsDate));
  if (days < 7) return `${weekdayFmt.format(tsDate)} ${timeFmt.format(tsDate)}`;
  if (tsDate.getFullYear() === nowDate.getFullYear()) return monthDayFmt.format(tsDate);
  return monthDayYearFmt.format(tsDate);
}

// ---------- date-only deadlines ----------
//
// Deadlines are floating local calendar dates stored as raw `YYYY-MM-DD`
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

// Urgency drives the badge's color role; the component maps done/binned
// items to a muted variant regardless of this value.
export type DeadlineUrgency = "overdue" | "today" | "future";

export interface DeadlineBadgeInfo {
  label: string;
  urgency: DeadlineUrgency;
}

// `Jul 12`, gaining the year only when it falls outside `ref`'s.
function compactDate(date: Date, ref: Date, locale: string): string {
  const opts: Intl.DateTimeFormatOptions =
    date.getFullYear() === ref.getFullYear()
      ? { month: "short", day: "numeric" }
      : { month: "short", day: "numeric", year: "numeric" };
  return new Intl.DateTimeFormat(locale, opts).format(date);
}

// Compact label + urgency for a deadline, relative to `today` (both raw
// `YYYY-MM-DD`). Rules: before today → "Overdue"; today → "Today";
// tomorrow → "Tomorrow"; within the next 7 days → short weekday; else a
// compact `Jul 12` (with the year when it differs from today's). Labels
// for the fixed cases come from i18n so callers stay locale-correct.
//
// `pastAsDate` swaps the "Overdue" label for that same compact date:
// nothing is still owed on a done/binned item, so the useful fact is when
// it was due, not that the deadline slipped.
export function formatDeadlineBadge(
  deadline: string,
  today: string,
  labels: { overdue: string; today: string; tomorrow: string },
  locale: string,
  opts?: { pastAsDate?: boolean },
): DeadlineBadgeInfo | null {
  const target = parseLocalDateParts(deadline);
  const ref = parseLocalDateParts(today);
  if (!target || !ref) return null;
  const days = calendarDayDiff(target, ref);
  if (days < 0) {
    const label = opts?.pastAsDate ? compactDate(target, ref, locale) : labels.overdue;
    return { label, urgency: "overdue" };
  }
  if (days === 0) return { label: labels.today, urgency: "today" };
  if (days === 1) return { label: labels.tomorrow, urgency: "future" };
  if (days < 7) {
    const weekday = new Intl.DateTimeFormat(locale, { weekday: "short" }).format(target);
    return { label: weekday, urgency: "future" };
  }
  return { label: compactDate(target, ref, locale), urgency: "future" };
}

// ---------- planned dates (`when`) ----------
//
// A `when` is a floating `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM`
// (timed) register (`spec/calendar-plan.md`). Same local-parts rule as
// deadlines: never `new Date(when)`.

/** Hour / minute pair as Kobalte's `TimeField` holds it: either or both
 *  may be undefined while the user is mid-edit. */
export interface TimeParts {
  hour?: number;
  minute?: number;
}

/** The `YYYY-MM-DD` day key of a `when` (its first ten characters). */
export function whenDay(when: string): string {
  return when.slice(0, 10);
}

/** The time part of a timed `when`, or null for an all-day one. */
export function whenTime(when: string): TimeParts | null {
  const m = /^\d{4}-\d{2}-\d{2}T(\d{2}):(\d{2})$/.exec(when);
  if (!m) return null;
  return { hour: Number(m[1]), minute: Number(m[2]) };
}

/** Both segments filled: the field state maps to a timed register. */
export function isCompleteTime(t: TimeParts): t is Required<TimeParts> {
  return t.hour != null && t.minute != null;
}

/** Neither segment filled: the field state maps to an all-day register. */
export function isEmptyTime(t: TimeParts): boolean {
  return t.hour == null && t.minute == null;
}

/** Build a `when` from a day stamp and an optional complete time. A
 *  partial or null time yields the all-day form. */
export function whenFromParts(day: string, time: TimeParts | null | undefined): string {
  if (time && isCompleteTime(time)) {
    return `${day}T${String(time.hour).padStart(2, "0")}:${String(time.minute).padStart(2, "0")}`;
  }
  return day;
}

/** The hour cycle the time field should render in: the explicit 12h /
 *  24h preference, or for "auto" whatever `Intl` resolves for the
 *  locale. Always passed to `TimeField` so the field and every formatted
 *  time in the app agree by construction. */
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

/** Local wall-clock time of a timed `when`, in the preferred cycle; empty
 *  for an all-day value. */
export function formatWhenTime(when: string, locale: string): string {
  const t = whenTime(when);
  const d = parseLocalDateParts(whenDay(when));
  if (!t || !d) return "";
  return timeFormatter(locale).format(
    new Date(d.getFullYear(), d.getMonth(), d.getDate(), t.hour, t.minute),
  );
}

export type WhenUrgency = "today" | "future";

export interface WhenBadgeInfo {
  label: string;
  urgency: WhenUrgency;
}

// Compact label + urgency for a planned date relative to `today`. Before
// today → the compact date itself, unjudged (a `when` is never "overdue",
// and whether it slipped is not decided: the useful fact is which day).
// Today / tomorrow / weekday / compact date otherwise, as deadlines do. A
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

// Task-dialog stamp: the dialog has room for the full picture, so every
// stamp carries its time of day. Today → time only; yesterday → "Yesterday
// HH:MM"; otherwise the date (with the year once it differs) plus time.
//
// `inline` marks a stamp that continues a sentence ("Created yesterday
// 8:06 PM"), where "Yesterday" drops its capital; standalone stamps keep it.
export function formatDialogStamp(
  ts: number,
  now: number,
  locale: string,
  opts?: { inline?: boolean },
): string {
  const tsDate = new Date(ts);
  const nowDate = new Date(now);
  const time = timeFormatter(locale).format(tsDate);
  const days = calendarDayDiff(nowDate, tsDate);
  if (days === 0) return time;
  if (days === 1) {
    const m = locale.startsWith("es") ? relativeEs : relativeEn;
    const label = m.yesterdayAt(time);
    return opts?.inline ? label.charAt(0).toLocaleLowerCase(locale) + label.slice(1) : label;
  }
  return `${compactDate(tsDate, nowDate, locale)} ${time}`;
}
