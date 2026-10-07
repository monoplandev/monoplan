// Where the user roughly is, for ranking place lookups (`geocode.ts`).
//
// Photon takes a point to bias its results toward, so "Richmond" resolves
// to the suburb down the road rather than the one in Virginia. Two
// sources, best first:
//
// - The Geolocation API, attempted once per boot (`refreshLocationBias`).
//   The browser prompts the first time and remembers the answer per
//   origin, so later boots are silent either way. A fix is rounded to two
//   decimals (~1km) before it is stored or sent anywhere: ranking needs
//   the city, not the street. It lives in localStorage so the next boot
//   can bias from the last known point before (or without) a fresh fix.
// - The time zone's principal city (`tz-coords.ts`), derived from the
//   zone name the browser reports. No permission, no network; coarse,
//   and sent with a wider zoom so it nudges rather than dominates.
//
// Neither point is end-to-end encrypted: it rides the Photon query
// string like the search text does (`spec/place-plan.md` "Privacy").

import { TZ_COORDS } from "./tz-coords.ts";

export const LOCATION_BIAS_KEY = "monoplan.location-bias";

/** How long a stored fix is trusted before the boot refresh is allowed
 *  to ask the platform for a new one (also the `maximumAge` passed to the
 *  platform, so a cached OS fix within this window costs nothing). */
const FRESH_MS = 60 * 60 * 1000;
const TIMEOUT_MS = 10_000;

export interface LocationBias {
  lat: number;
  lon: number;
  /** Photon `zoom`: the scale the bias applies at. Smaller is wider. */
  zoom: number;
}

interface StoredFix {
  lat: number;
  lon: number;
  /** Epoch ms the fix was taken. */
  at: number;
}

const round2 = (v: number): number => Math.round(v * 100) / 100;

const inRange = (v: unknown, bound: number): v is number =>
  typeof v === "number" && Number.isFinite(v) && Math.abs(v) <= bound;

/** Parse a stored fix; `null` for anything malformed or out of range. */
export function parseStoredFix(raw: string | null | undefined): StoredFix | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<StoredFix> | null;
    if (!v || !inRange(v.lat, 90) || !inRange(v.lon, 180)) return null;
    const at = typeof v.at === "number" && Number.isFinite(v.at) ? v.at : 0;
    return { lat: v.lat, lon: v.lon, at };
  } catch {
    return null;
  }
}

/** The principal city of a time zone name, or `null` for a zone with no
 *  geography (`Etc/*`, `UTC`) or an unknown name. */
export function tzPoint(zone: string | undefined): { lat: number; lon: number } | null {
  if (!zone) return null;
  const hit = TZ_COORDS[zone];
  return hit ? { lat: hit[0], lon: hit[1] } : null;
}

/** Pick the bias from a stored fix (preferred, any age) else the time
 *  zone; pure so it can be tested without a DOM. */
export function chooseBias(
  stored: string | null | undefined,
  zone: string | undefined,
): LocationBias | null {
  const fix = parseStoredFix(stored);
  if (fix) return { lat: fix.lat, lon: fix.lon, zoom: 12 };
  const tz = tzPoint(zone);
  return tz ? { lat: tz.lat, lon: tz.lon, zoom: 9 } : null;
}

const readStored = (): string | null => {
  try {
    return localStorage.getItem(LOCATION_BIAS_KEY);
  } catch {
    return null;
  }
};

const currentZone = (): string | undefined => {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return undefined;
  }
};

/** The bias to send with a lookup right now, or `null` for none. */
export const locationBias = (): LocationBias | null =>
  chooseBias(readStored(), currentZone());

/** Attempt a geolocation fix and store it. Never throws and never
 *  rejects: a denied permission, an unsupported platform, a timeout or a
 *  stale stored fix that fails to refresh all leave things as they were.
 *  Skipped while the stored fix is fresh, so the platform is asked at
 *  most about once an hour across reloads. */
export async function refreshLocationBias(): Promise<void> {
  try {
    if (typeof navigator === "undefined" || !navigator.geolocation) return;
    const have = parseStoredFix(readStored());
    if (have && Date.now() - have.at < FRESH_MS) return;
    // A remembered denial fails fast without a prompt anyway; this just
    // saves the call where the platform lets us ask first.
    const perm = await navigator.permissions
      ?.query({ name: "geolocation" })
      .catch(() => null);
    if (perm?.state === "denied") return;
    const pos = await new Promise<GeolocationPosition>((resolve, reject) =>
      navigator.geolocation.getCurrentPosition(resolve, reject, {
        enableHighAccuracy: false,
        maximumAge: FRESH_MS,
        timeout: TIMEOUT_MS,
      }),
    );
    const fix: StoredFix = {
      lat: round2(pos.coords.latitude),
      lon: round2(pos.coords.longitude),
      at: Date.now(),
    };
    if (!inRange(fix.lat, 90) || !inRange(fix.lon, 180)) return;
    localStorage.setItem(LOCATION_BIAS_KEY, JSON.stringify(fix));
  } catch {
    // Nothing to do: the time zone fallback covers it.
  }
}
