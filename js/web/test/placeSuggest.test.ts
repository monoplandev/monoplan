// The place field's rules over core's known-place list
// (`spec/place-plan.md` "Reuse"): narrowing as the user types, the exact
// match Enter reuses, and dropping lookup rows that repeat a known place.
// The ranking and dedup of the list itself are core's
// (`core/src/places.rs`); `js/core/test/doc.test.ts` covers the wasm
// round trip.

import { describe, expect, test } from "bun:test";

import {
  dropKnown,
  exactKnown,
  filterKnown,
  foldLabel,
  samePlace,
  type PlaceSuggestion,
} from "../src/placeSuggest.ts";
import type { Place } from "../src/sync/store.ts";

const sug = (place: Place, count = 1, lastUsed = 0): PlaceSuggestion => ({
  place,
  count,
  lastUsed,
});

const gym: Place = { label: "Gym", lat: -33.8688, lon: 151.2093, ref: "osm:node/1" };
const home: Place = { label: "Home" };
const work: Place = { label: "Work", lat: 1, lon: 2, address: "1 George St, Sydney" };
const known = [sug(gym, 3), sug(home), sug(work)];

describe("filterKnown", () => {
  test("a blank query keeps the order and applies the limit", () => {
    expect(filterKnown(known, "", 10).map((s) => s.place.label)).toEqual(["Gym", "Home", "Work"]);
    expect(filterKnown(known, "  ", 2).map((s) => s.place.label)).toEqual(["Gym", "Home"]);
  });

  test("narrows by label or address prefix from the first character", () => {
    expect(filterKnown(known, "g", 10).map((s) => s.place.label)).toEqual(["Gym", "Work"]);
    expect(filterKnown(known, "geo", 10).map((s) => s.place.label)).toEqual(["Work"]);
    expect(filterKnown(known, "ho", 10).map((s) => s.place.label)).toEqual(["Home"]);
    expect(filterKnown(known, "zzz", 10)).toEqual([]);
  });
});

describe("exactKnown", () => {
  test("needs the whole label, folded, not a prefix", () => {
    expect(exactKnown(known, "gym")).toBe(gym);
    expect(exactKnown(known, " GYM ")).toBe(gym);
    expect(exactKnown(known, "gy")).toBeUndefined();
    expect(exactKnown(known, "")).toBeUndefined();
  });

  test("folds like core", () => {
    expect(foldLabel("  Café  Noir ")).toBe("cafe noir");
  });
});

describe("dropKnown", () => {
  test("drops lookup rows sharing a ref or rounded coordinates with a shown known place", () => {
    const remote: Place[] = [
      { label: "Fitness First", lat: -33.86881, lon: 151.20929, ref: "osm:node/1" },
      { label: "Another Gym", lat: -33.86882, lon: 151.20931, ref: "osm:node/2" },
      { label: "Far Gym", lat: -33.9, lon: 151.2, ref: "osm:node/3" },
    ];
    expect(dropKnown(remote, [sug(gym)]).map((p) => p.label)).toEqual(["Far Gym"]);
    // Nothing shown, nothing dropped.
    expect(dropKnown(remote, []).length).toBe(3);
  });
});

describe("samePlace", () => {
  test("compares the whole value", () => {
    expect(samePlace(gym, { ...gym })).toBe(true);
    expect(samePlace(gym, { ...gym, address: "x" })).toBe(false);
    expect(samePlace(home, { label: "Home", address: undefined })).toBe(true);
    expect(samePlace(null, undefined)).toBe(true);
    expect(samePlace(home, null)).toBe(false);
  });
});
