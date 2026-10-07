// Place lookup against Komoot's public Photon instance (OpenStreetMap
// data, github.com/komoot/photon).
//
// This is the only place the web client talks to anything but its own
// server, so the constraints are spelled out here:
//
// - Queries are NOT end-to-end encrypted. The search string, and the
//   browser's Referer (this origin), go to photon.komoot.io. Only what the
//   user types into the place field leaves the device; the place that
//   gets stored on the item is encrypted like every register.
// - Photon is built for search-as-you-type (from three characters), and
//   its public instance asks only for a "reasonable" request rate, with
//   extensive use throttled and no availability guarantee. So: the field
//   debounces input, requests are serialised with a gap between them, and
//   results are cached in memory by normalised query. The endpoint is one
//   constant so a self-hosted Photon (or another provider with the same
//   shape) is a one-line swap until the server-side knob exists
//   (`spec/place-plan.md` "Deferred").
// - Data is © OpenStreetMap contributors (ODbL); the results popover
//   carries the credit.
//
// A self-hosted deployment that wants no third-party calls simply never
// searches: a typed label is a complete place on its own.

import type { Place } from "./sync/store.ts";

const ENDPOINT = "https://photon.komoot.io/api/";
const MIN_GAP_MS = 600;
const CACHE_MAX = 200;
const LIMIT = 8;
/** Photon answers 400 for a language it does not index; only these are
 *  safe to pass, anything else falls back to the default names. */
const LANGS: ReadonlySet<string> = new Set(["en", "de", "fr", "it"]);

/** The subset of a Photon GeoJSON feature the mapping reads. */
export interface PhotonFeature {
  geometry?: { type?: string; coordinates?: unknown };
  properties?: {
    osm_id?: number;
    osm_type?: string;
    name?: string;
    housenumber?: string;
    street?: string;
    district?: string;
    city?: string;
    county?: string;
    state?: string;
    postcode?: string;
    country?: string;
  };
}

const OSM_TYPES: Record<string, string> = { N: "node", W: "way", R: "relation" };

const finiteDegrees = (raw: unknown, bound: number): number | undefined =>
  typeof raw === "number" && Number.isFinite(raw) && Math.abs(raw) <= bound
    ? raw
    : undefined;

const str = (v: unknown): string => (typeof v === "string" ? v.trim() : "");

/** Map one Photon feature to a `Place`: the feature's own name as the
 *  label (the first address part when it has none), the remaining address
 *  parts joined as the address, coordinates when both are in range, and an
 *  `osm:<type>/<id>` ref. `null` when there is nothing to label it with. */
export function photonToPlace(f: PhotonFeature): Place | null {
  const p = f.properties ?? {};
  const streetLine = [str(p.housenumber), str(p.street)].filter(Boolean).join(" ");
  const parts: string[] = [];
  for (const part of [streetLine, str(p.district), str(p.city), str(p.state), str(p.country)]) {
    if (part && !parts.includes(part)) parts.push(part);
  }
  let label = str(p.name);
  if (!label) label = parts.shift() ?? "";
  if (!label) return null;
  const place: Place = { label };
  const coords = Array.isArray(f.geometry?.coordinates) ? f.geometry.coordinates : [];
  const lon = finiteDegrees(coords[0], 180);
  const lat = finiteDegrees(coords[1], 90);
  if (lat !== undefined && lon !== undefined) {
    place.lat = lat;
    place.lon = lon;
  }
  const address = parts.filter((part) => part !== label).join(", ");
  if (address) place.address = address;
  const type = typeof p.osm_type === "string" ? OSM_TYPES[p.osm_type] : undefined;
  if (type && typeof p.osm_id === "number" && Number.isInteger(p.osm_id))
    place.ref = `osm:${type}/${p.osm_id}`;
  return place;
}

const normalise = (q: string): string => q.trim().replace(/\s+/g, " ").toLowerCase();

// Insertion-ordered so the oldest entry is the first key.
const cache = new Map<string, Place[]>();
const remember = (key: string, places: Place[]) => {
  cache.delete(key);
  cache.set(key, places);
  if (cache.size > CACHE_MAX) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
};

// One request at a time, a gap apart. Each call chains on the previous
// one's settlement, so a burst of searches drains at a polite rate rather
// than in parallel; an aborted call (the user kept typing) drops out of
// the chain before it ever fetches.
let chain: Promise<unknown> = Promise.resolve();
let lastSentAt = 0;
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Search for places matching a free-text query. Resolves to an empty
 *  list for a blank query; rejects on a network or HTTP failure (and on
 *  abort). Cached results come back without a request. */
export function searchPlaces(
  query: string,
  opts: { signal?: AbortSignal } = {},
): Promise<Place[]> {
  const key = normalise(query);
  if (!key) return Promise.resolve([]);
  const hit = cache.get(key);
  if (hit) return Promise.resolve(hit);
  const run = async (): Promise<Place[]> => {
    const again = cache.get(key);
    if (again) return again;
    opts.signal?.throwIfAborted();
    const wait = lastSentAt + MIN_GAP_MS - Date.now();
    if (wait > 0) await sleep(wait);
    opts.signal?.throwIfAborted();
    lastSentAt = Date.now();
    const params = new URLSearchParams({ q: key, limit: String(LIMIT) });
    const lang = (typeof navigator !== "undefined" ? navigator.language : "")
      .split("-")[0]
      ?.toLowerCase();
    if (lang && LANGS.has(lang)) params.set("lang", lang);
    const res = await fetch(`${ENDPOINT}?${params.toString()}`, {
      signal: opts.signal,
      headers: { Accept: "application/json" },
    });
    if (!res.ok) throw new Error(`photon: HTTP ${res.status}`);
    const body = (await res.json()) as { features?: unknown };
    const rows = Array.isArray(body?.features) ? (body.features as PhotonFeature[]) : [];
    const places = rows.map(photonToPlace).filter((p): p is Place => p !== null);
    remember(key, places);
    return places;
  };
  // The chain swallows the previous failure so one bad request does not
  // poison every later one; the caller still gets its own rejection.
  const next = chain.then(run, run);
  chain = next.catch(() => undefined);
  return next;
}

/** OpenStreetMap link centred on a place's coordinates. */
export const osmMapUrl = (lat: number, lon: number): string =>
  `https://www.openstreetmap.org/?mlat=${lat}&mlon=${lon}#map=17/${lat}/${lon}`;
