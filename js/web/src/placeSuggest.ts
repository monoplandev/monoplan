// Known places for the place field (`spec/place-plan.md` "Reuse"): the
// deduped, recency-ranked places already on items, as core computes
// them (`Doc::place_suggestions`), plus the pure rules the field applies
// on top: narrowing them as the user types, picking an exact match on
// Enter, and keeping the geocoder's rows from repeating one the user
// already has. The ranking is core's; nothing here re-sorts.

import { matchesName, tokenize } from "./search.ts";
import type { Place } from "./sync/store.ts";

export interface PlaceSuggestion {
  place: Place;
  /** Items (not binned) carrying this place. */
  count: number;
  /** Unix millis of the newest use. */
  lastUsed: number;
}

/** The field's label-dedup key, the search folding (`fold_label` in core). */
export const foldLabel = (label: string): string => tokenize(label).join(" ");

/** Known places matching a typed query: every query token prefixes a
 *  token of the label or the address (core's `place_matches`). An empty
 *  query keeps them all. Order is preserved. */
export function filterKnown(
  known: readonly PlaceSuggestion[],
  query: string,
  limit: number,
): PlaceSuggestion[] {
  const q = query.trim();
  const out: PlaceSuggestion[] = [];
  for (const s of known) {
    const hay = s.place.address ? `${s.place.label} ${s.place.address}` : s.place.label;
    if (q === "" || matchesName(hay, q)) {
      out.push(s);
      if (out.length >= limit) break;
    }
  }
  return out;
}

/** The top-ranked known place whose folded label equals the typed
 *  text's, so Enter on "gym" reuses the geocoded Gym instead of writing
 *  a label-only one. A prefix is not a match. */
export function exactKnown(
  known: readonly PlaceSuggestion[],
  label: string,
): Place | undefined {
  const want = foldLabel(label);
  if (!want) return undefined;
  return known.find((s) => foldLabel(s.place.label) === want)?.place;
}

/** Coordinates rounded to ~4 decimals (~10 m), core's bucket. */
const coordKey = (p: Place): string | undefined =>
  p.lat != null && p.lon != null
    ? `${Math.round(p.lat * 1e4)},${Math.round(p.lon * 1e4)}`
    : undefined;

/** Geocoder rows with any that repeat a shown known place dropped: the
 *  same provider ref, or the same rounded coordinates. The known row
 *  wins because it carries the label the user chose. */
export function dropKnown(remote: readonly Place[], shown: readonly PlaceSuggestion[]): Place[] {
  const refs = new Set<string>();
  const coords = new Set<string>();
  for (const s of shown) {
    if (s.place.ref) refs.add(s.place.ref);
    const c = coordKey(s.place);
    if (c) coords.add(c);
  }
  return remote.filter((p) => {
    if (p.ref && refs.has(p.ref)) return false;
    const c = coordKey(p);
    return !(c && coords.has(c));
  });
}

/** Whole-value equality, the register's own notion of "unchanged". */
export function samePlace(a: Place | null | undefined, b: Place | null | undefined): boolean {
  if (!a || !b) return !a && !b;
  return (
    a.label === b.label &&
    a.lat === b.lat &&
    a.lon === b.lon &&
    (a.address ?? undefined) === (b.address ?? undefined) &&
    (a.ref ?? undefined) === (b.ref ?? undefined)
  );
}
