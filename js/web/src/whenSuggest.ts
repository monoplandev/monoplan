// Date suggestions read out of a title. A date phrase typed into a title
// ("dentist friday 8:30") never changes the title and never sets a date
// by itself: it raises a question, "Fri 14 Nov 08:30?", shown in orange
// where the date would be, which the user answers yes or no
// (`WhenSuggestion.tsx`). Decided 2026-11-13, replacing the inline token
// that cut the words out of the title (`spec/calendar-plan.md`, "Typed
// date phrases").
//
// The question is the item's `whenSuggested` register, a resolved
// `when`-shaped value:
//
//   - It is written when a title edit brings a date phrase the title did
//     not have before: the phrase is resolved once, against the moment
//     it was typed, so "friday" cannot drift to a later Friday, and
//     clients with no parser can show and answer it.
//   - Yes moves it into `when`; No clears it. Nothing is kept of a No:
//     the same wording is not asked again because only a *changed*
//     phrase writes a suggestion, so fixing a typo elsewhere in the
//     title leaves the answer standing.
//   - A phrase edited out of the title takes its unanswered suggestion
//     with it.
//   - It shows only on an open item with no `when`. Setting a date any
//     other way clears it (the core does that).
//
// This module is the rule, pure but for the device preference; the
// editors (`TaskDialog.tsx`, `Row.tsx`, `Workspace.tsx`) decide when a
// title counts as changed.

import { createSignal } from "solid-js";
import { formatWhenBadge, todayStamp } from "./format.tsx";
import { hasHostSettings, hostSetting, onHostSettings } from "./hostSettings.ts";
import { findWhenPhrase, phraseKey } from "./whenParse.ts";

// Whether typed dates are recognised at all: a device preference like
// the time format, so it lives in a cookie beside it (`format.tsx`), on
// by default. In the macOS app's window the app owns it
// (`hostSettings.ts`): read from there, followed live, never written.
// Off, no title raises a suggestion; one already pending still shows.
const TYPED_DATES_COOKIE = "typeddates";

function readTypedDatesPref(): boolean {
  const host = hostSetting("typedDates");
  if (host !== undefined) return host;
  try {
    return !/(?:^|; )typeddates=0(?:;|$)/.test(document.cookie);
  } catch {
    return true;
  }
}

const [typedDatesPref, setTypedDatesSignal] = createSignal(readTypedDatesPref());
export { typedDatesPref };
onHostSettings((s) => {
  if (s.typedDates !== undefined) setTypedDatesSignal(s.typedDates);
});

export function setTypedDatesPref(on: boolean): void {
  setTypedDatesSignal(on);
  if (hasHostSettings()) return;
  try {
    document.cookie = on
      ? `${TYPED_DATES_COOKIE}=;path=/;max-age=0;SameSite=Lax`
      : `${TYPED_DATES_COOKIE}=0;path=/;max-age=31536000;SameSite=Lax`;
  } catch {}
}

export interface TitleDate {
  /** Resolved register value: `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`. */
  when: string;
  /** The phrase's wording, folded (`phraseKey`): what makes two titles
   *  "the same date phrase". */
  key: string;
}

export interface TitleDateOptions {
  /** The reference instant, unix millis. */
  now: number;
  locale: string;
}

/** The date phrase in `text`, or null: none, or the preference is off. */
export function titleDate(text: string, opts: TitleDateOptions): TitleDate | null {
  if (!typedDatesPref()) return null;
  const p = findWhenPhrase(text, { now: new Date(opts.now), locale: opts.locale });
  return p ? { when: p.when, key: phraseKey(p.text) } : null;
}

/** The suggestion `text` raises over a title whose phrase was
 *  `baselineKey` (null: it had none): its phrase, when that is a new
 *  wording; null when there is no phrase or it is the one already
 *  there (asked and answered, or still pending in the register). */
export function raisedDate(
  text: string,
  baselineKey: string | null,
  opts: TitleDateOptions,
): TitleDate | null {
  const d = titleDate(text, opts);
  return d && d.key !== baselineKey ? d : null;
}

/** What a title change does to the pending suggestion: write a new one
 *  (`{ set }`), drop a stale one (`"clear"`), or leave things be. */
export function suggestionChange(
  text: string,
  baselineKey: string | null,
  opts: TitleDateOptions,
): { set: string } | "clear" | null {
  const d = titleDate(text, opts);
  if (!d) return baselineKey !== null ? "clear" : null;
  return d.key !== baselineKey ? { set: d.when } : null;
}

/** The slice of an item the rule reads. */
export interface SuggestableItem {
  when?: string;
  whenSuggested?: string;
  closed: boolean;
}

/** Apply a title change to item `id`'s suggestion register. `item` is
 *  the item as it stands; a closed item or one with a date takes no
 *  suggestion (a date is a plan, and neither has one to make). Writes
 *  only on a real change, so it adds no empty undo step. */
export function applySuggestionChange(
  app: { setItemWhenSuggested(id: string, when: string | null): void },
  id: string,
  item: SuggestableItem,
  text: string,
  baselineKey: string | null,
  opts: TitleDateOptions,
): void {
  if (item.closed || item.when) return;
  const change = suggestionChange(text, baselineKey, opts);
  if (change === null) return;
  if (change === "clear") {
    if (item.whenSuggested) app.setItemWhenSuggested(id, null);
  } else if (change.set !== item.whenSuggested) {
    app.setItemWhenSuggested(id, change.set);
  }
}

/** A suggestion as the question reads: the day as the badges label it,
 *  the time in the app's format ("Fri 14 Nov 08:30"). The caller adds
 *  the question mark (`m().when.suggestion`). */
export function suggestionLabel(
  when: string,
  now: number,
  locale: string,
  labels: { today: string; tomorrow: string },
): string {
  return formatWhenBadge(when, todayStamp(now), labels, locale)?.label ?? when;
}
