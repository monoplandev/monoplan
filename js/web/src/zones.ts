// Time zones for the task surface's zone picker (`ZonePicker.tsx`;
// `spec/calendar-plan.md` "Fixed instants"). Pure but for the two wasm
// calls at the bottom, so the list and its matching are testable without
// a DOM.
//
// The division of labour: the core owns every conversion (one tzdb, one
// rule for the hours a zone skips or repeats). The browser's `Intl` only
// supplies what a person reads and picks from: the list of zone ids, each
// zone's name in the app's language ("Central European Time") and its
// offset on the item's day ("GMT+1"). A label a little out at a
// transition is a label, never a stored value.
//
// A zone is named by its city, read out of the id ("Europe/Paris" is
// "Paris", "America/Argentina/Buenos_Aires" is "Buenos Aires"): `Intl`
// has no localised city names to offer.
//
// Matching: every typed word must begin a word of the zone's city, id,
// name or offset, accents and case aside ("new y", "eastern", "gmt+1",
// "utc+1", "sao paulo"). Zones whose city starts with the query lead,
// then those with a city word starting with its first word, then the
// rest; each tier by city. A blank query is every zone by city, with the
// device's own first.

import { whenInZone, whenLocalized } from "@monoplan/core/wasm";

export interface ZoneOption {
  /** IANA id, as stored. */
  id: string;
  /** The id's last segment, underscores as spaces. */
  city: string;
  /** The zone's name in the app's language, or "" when `Intl` has none. */
  name: string;
  /** Offset on the day asked about, e.g. "GMT+1"; "" when unknown. */
  offset: string;
}

/** The device's IANA zone, or null when the browser cannot say. */
export function deviceZone(): string | null {
  try {
    return new Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

let ids: string[] | null = null;

/** Every zone id the browser lists, plus UTC, which some leave out. */
export function zoneIds(): readonly string[] {
  if (ids) return ids;
  let listed: string[] = [];
  try {
    listed = Intl.supportedValuesOf("timeZone");
  } catch {
    // An engine without the list still offers the device's zone and UTC.
  }
  const all = new Set(listed);
  all.add("UTC");
  const device = deviceZone();
  if (device) all.add(device);
  ids = [...all];
  return ids;
}

export function zoneCity(id: string): string {
  return id.slice(id.lastIndexOf("/") + 1).replaceAll("_", " ");
}

/** Noon UTC on a `YYYY-MM-DD` day: the instant offsets are quoted at. */
function noonUtc(day: string): Date {
  const [y, mo, d] = day.split("-").map(Number);
  const date = new Date(Date.UTC(y ?? NaN, (mo ?? NaN) - 1, d ?? NaN, 12));
  return Number.isNaN(date.getTime()) ? new Date() : date;
}

function zoneNamePart(
  id: string,
  locale: string,
  style: "shortOffset" | "longGeneric",
  at: Date,
): string {
  try {
    return (
      new Intl.DateTimeFormat(locale, { timeZone: id, timeZoneName: style })
        .formatToParts(at)
        .find((p) => p.type === "timeZoneName")?.value ?? ""
    );
  } catch {
    // A zone the browser does not know (it came from another device).
    return "";
  }
}

/** One zone as the picker shows it, offset as of `day` (`YYYY-MM-DD`). */
export function zoneOption(id: string, locale: string, day: string): ZoneOption {
  const at = noonUtc(day);
  return {
    id,
    city: zoneCity(id),
    name: zoneNamePart(id, locale, "longGeneric", at),
    offset: zoneNamePart(id, locale, "shortOffset", at),
  };
}

let cache: { key: string; options: ZoneOption[] } | null = null;

/** Every zone by city, offsets as of `day`. Some hundreds of `Intl`
 *  formatters to build (tens of milliseconds), so the last answer is
 *  kept: a picker asks again on every keystroke. */
export function zoneOptions(locale: string, day: string): readonly ZoneOption[] {
  const key = `${locale}|${day}`;
  if (cache?.key === key) return cache.options;
  const collator = new Intl.Collator(locale);
  const options = zoneIds()
    .map((id) => zoneOption(id, locale, day))
    .sort((a, b) => collator.compare(a.city, b.city) || collator.compare(a.id, b.id));
  cache = { key, options };
  return options;
}

const fold = (s: string) =>
  s
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();

const words = (s: string) => fold(s).split(/[\s/_,()]+/).filter(Boolean);

/** The zones a typed query names, best first; `first` (the device's
 *  zone) leads the blank list. */
export function zoneSuggestions(
  query: string,
  options: readonly ZoneOption[],
  first?: string | null,
): ZoneOption[] {
  const typed = words(query);
  if (typed.length === 0) {
    const lead = options.find((o) => o.id === first);
    return lead ? [lead, ...options.filter((o) => o !== lead)] : [...options];
  }
  const whole = typed.join(" ");
  const tiers: ZoneOption[][] = [[], [], []];
  for (const o of options) {
    const city = words(o.city);
    // "utc+1" finds what `Intl` writes as "GMT+1".
    const offset = fold(o.offset);
    const hay = [...city, ...words(o.id), ...words(o.name), offset, offset.replace("gmt", "utc")];
    if (!typed.every((t) => hay.some((w) => w.startsWith(t)))) continue;
    const tier = city.join(" ").startsWith(whole)
      ? 0
      : city.some((w) => w.startsWith(typed[0] ?? ""))
        ? 1
        : 2;
    tiers[tier]?.push(o);
  }
  return tiers.flat();
}

/** The wall time (`YYYY-MM-DDTHH:MM`) in `zone` at the instant the
 *  device-local timed `when` names, or null when the value is not timed
 *  or the core's tzdb does not know the zone. */
export function wallInZone(when: string, zone: string): string | null {
  return whenInZone(when, zone) ?? null;
}

/** The register value for a device-local `when` fixed to `zone`: what
 *  `setItemWhen` takes to write both at once (a capture, a copy). Falls
 *  back to the floating value where it cannot be converted. */
export function storedWhen(when: string, zone: string | null | undefined): string {
  const wall = zone ? wallInZone(when, zone) : null;
  return wall ? `${wall}[${zone}]` : when;
}

/** A device-local timed `when` after its zone changes from `from` to
 *  `to` (null is floating), as the core's `set_item_when_zone` would
 *  leave it: fixing keeps the clock digits the value has in `from` and
 *  reads them in `to`; unfixing keeps what the device shows. For the
 *  capture buffer, which has no item to ask the core about. */
export function rezonedWhen(when: string, from: string | null, to: string | null): string {
  if (!to) return when;
  const digits = from ? (wallInZone(when, from) ?? when) : when;
  return whenLocalized(`${digits}[${to}]`);
}
