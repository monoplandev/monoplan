// Date suggestions read out of a title. A date phrase typed into a title
// ("dentist friday 8:30") never changes the title and never sets a date
// by itself: it raises a question, "Suggested: Fri 14 Nov 08:30", shown in orange
// above the date field, which the user answers yes or no
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
//   - It shows only in the task surface, on an open item with no
//     `when`; lists and boards carry no trace of it. Setting a date any
//     other way clears it (the core does that).
//   - An item with a day but no time still takes the *time* of a phrase
//     (2026-11-25): "dentist 8:30" on an item dated Friday suggests
//     Friday 08:30, whatever day the phrase itself names or implies
//     (`suggestionFor`). The picked day stands; a phrase with no time
//     has nothing to add, and a timed `when` takes no suggestion.
//
// This module is the rule, pure; the editors (`TaskDialog.tsx`, `Row.tsx`, `Workspace.tsx`) decide when a
// title counts as changed.

import { formatWhenBadge, todayStamp } from "./format.tsx";
import { findWhenPhrase, phraseKey } from "./whenParse.ts";

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

/** The date phrase in `text`, or null. */
export function titleDate(text: string, opts: TitleDateOptions): TitleDate | null {
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

/** What a phrase's value `phraseWhen` suggests for an item whose date
 *  is `current`: all of it when there is no date, its time on the day
 *  already picked when the date is all-day and the phrase has a time,
 *  nothing otherwise. */
export function suggestionFor(
  phraseWhen: string,
  current: string | null | undefined,
): string | null {
  if (!current) return phraseWhen;
  if (current.length !== 10 || phraseWhen.length <= 10) return null;
  return current + phraseWhen.slice(10);
}

/** The stored suggestion as a surface shows it beside the date
 *  `current`: as it is with no date; beside an all-day date only a time
 *  on that same day (anything else is a leftover of a concurrent
 *  write); never beside a timed one. */
export function shownSuggestion(
  stored: string | null | undefined,
  current: string | null | undefined,
): string | null {
  if (!stored) return null;
  if (!current) return stored;
  return current.length === 10 && stored.length > 10 && stored.startsWith(current)
    ? stored
    : null;
}

/** The slice of an item the rule reads. */
export interface SuggestableItem {
  when?: string;
  whenSuggested?: string;
  closed: boolean;
}

/** Apply a title change to item `id`'s suggestion register. `item` is
 *  the item as it stands; a closed item or one with a time takes no
 *  suggestion (a date is a plan, and neither has one to make), and one
 *  with a day only takes a time (`suggestionFor`). Writes only on a
 *  real change, so it adds no empty undo step. */
export function applySuggestionChange(
  app: { setItemWhenSuggested(id: string, when: string | null): void },
  id: string,
  item: SuggestableItem,
  text: string,
  baselineKey: string | null,
  opts: TitleDateOptions,
): void {
  if (item.closed) return;
  const change = suggestionChange(text, baselineKey, opts);
  if (change === null) return;
  // A new phrase with nothing to add to the date in place (no time, or
  // the date has one) drops the suggestion the old phrase raised.
  const set = change === "clear" ? null : suggestionFor(change.set, item.when);
  if (set === null) {
    if (item.whenSuggested) app.setItemWhenSuggested(id, null);
  } else if (set !== item.whenSuggested) {
    app.setItemWhenSuggested(id, set);
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
