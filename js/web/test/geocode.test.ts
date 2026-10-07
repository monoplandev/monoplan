// `photonToPlace`: the pure mapping from a Photon GeoJSON feature to a
// `Place`. The network path (`searchPlaces`) is not exercised here.

import { describe, expect, test } from "bun:test";

import { osmMapUrl, photonToPlace } from "../src/geocode.ts";

const feature = (
  properties: NonNullable<Parameters<typeof photonToPlace>[0]["properties"]>,
  coordinates?: unknown,
) => ({
  geometry: coordinates === undefined ? undefined : { type: "Point", coordinates },
  properties,
});

describe("photonToPlace", () => {
  test("uses the feature name, the address parts as address, and an osm ref", () => {
    expect(
      photonToPlace(
        feature(
          {
            osm_type: "N",
            osm_id: 123,
            name: "Luigi's",
            housenumber: "1",
            street: "George Street",
            city: "Sydney",
            state: "New South Wales",
            country: "Australia",
          },
          [151.2093, -33.8688],
        ),
      ),
    ).toEqual({
      label: "Luigi's",
      lat: -33.8688,
      lon: 151.2093,
      address: "1 George Street, Sydney, New South Wales, Australia",
      ref: "osm:node/123",
    });
  });

  test("falls back to the first address part as label when there is no name", () => {
    expect(
      photonToPlace(
        feature(
          { osm_type: "W", osm_id: 9, street: "George Street", city: "Sydney", country: "Australia" },
          [151.2, -33.8],
        ),
      ),
    ).toEqual({
      label: "George Street",
      lat: -33.8,
      lon: 151.2,
      address: "Sydney, Australia",
      ref: "osm:way/9",
    });
  });

  test("a city feature does not repeat its own name in the address", () => {
    expect(
      photonToPlace(feature({ name: "Sydney", city: "Sydney", state: "NSW", country: "Australia" })),
    ).toEqual({ label: "Sydney", address: "NSW, Australia" });
  });

  test("drops coordinates that are not finite numbers or are out of range", () => {
    expect(photonToPlace(feature({ name: "Nowhere" }, [10, 95]))).toEqual({ label: "Nowhere" });
    expect(photonToPlace(feature({ name: "X" }, ["151.2", "-33.8"]))).toEqual({ label: "X" });
    expect(photonToPlace(feature({ name: "Y" }, [10]))).toEqual({ label: "Y" });
    expect(photonToPlace(feature({ name: "Z" }, [Number.NaN, 1]))).toEqual({ label: "Z" });
  });

  test("no ref without a known osm type and an integer id; nothing to label gives null", () => {
    expect(photonToPlace(feature({ name: "Z", osm_type: "X", osm_id: 1 }))).toEqual({ label: "Z" });
    expect(photonToPlace(feature({ name: "Z", osm_type: "R" }))).toEqual({ label: "Z" });
    expect(photonToPlace(feature({ name: "Z", osm_type: "R", osm_id: 2 }))).toEqual({
      label: "Z",
      ref: "osm:relation/2",
    });
    expect(photonToPlace(feature({ name: "   " }))).toBeNull();
    expect(photonToPlace({})).toBeNull();
  });
});

describe("osmMapUrl", () => {
  test("centres the map on the coordinates with a marker", () => {
    expect(osmMapUrl(-33.8688, 151.2093)).toBe(
      "https://www.openstreetmap.org/?mlat=-33.8688&mlon=151.2093#map=17/-33.8688/151.2093",
    );
  });
});
