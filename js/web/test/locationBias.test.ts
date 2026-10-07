// Pure parts of the place-lookup bias: stored-fix parsing, the time zone
// fallback, and the choice between them. The geolocation call itself
// needs a browser and is not covered here.

import { describe, expect, test } from "bun:test";
import { chooseBias, parseStoredFix, tzPoint } from "../src/locationBias.ts";

describe("parseStoredFix", () => {
  test("round-trips a valid fix", () => {
    expect(parseStoredFix('{"lat":-37.82,"lon":144.97,"at":5}')).toEqual({
      lat: -37.82,
      lon: 144.97,
      at: 5,
    });
  });
  test("tolerates a missing timestamp", () => {
    expect(parseStoredFix('{"lat":1,"lon":2}')).toEqual({ lat: 1, lon: 2, at: 0 });
  });
  test("rejects junk, partial and out-of-range values", () => {
    for (const raw of [null, undefined, "", "nope", "{}", '{"lat":1}', '{"lat":91,"lon":0}', '{"lat":0,"lon":"1"}', '{"lat":0,"lon":181}'])
      expect(parseStoredFix(raw)).toBeNull();
  });
});

describe("tzPoint", () => {
  test("canonical and alias names resolve to the same city", () => {
    expect(tzPoint("Asia/Kolkata")).toEqual({ lat: 22.53, lon: 88.37 });
    expect(tzPoint("Asia/Calcutta")).toEqual(tzPoint("Asia/Kolkata"));
  });
  test("zones without geography, unknown names and undefined give null", () => {
    expect(tzPoint("Etc/GMT+5")).toBeNull();
    expect(tzPoint("UTC")).toBeNull();
    expect(tzPoint("Mars/Olympus_Mons")).toBeNull();
    expect(tzPoint(undefined)).toBeNull();
  });
});

describe("chooseBias", () => {
  test("a stored fix wins over the zone, at a tighter zoom", () => {
    expect(chooseBias('{"lat":51.5,"lon":-0.12,"at":0}', "Australia/Melbourne")).toEqual({
      lat: 51.5,
      lon: -0.12,
      zoom: 12,
    });
  });
  test("falls back to the zone's city at a wider zoom", () => {
    expect(chooseBias(null, "Australia/Melbourne")).toEqual({ lat: -37.82, lon: 144.97, zoom: 9 });
    expect(chooseBias("garbage", "Australia/Melbourne")?.zoom).toBe(9);
  });
  test("nothing usable gives null", () => {
    expect(chooseBias(null, "UTC")).toBeNull();
    expect(chooseBias(null, undefined)).toBeNull();
  });
});
