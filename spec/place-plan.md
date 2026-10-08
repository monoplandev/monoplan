# Place: plan

**Status: Phase 0 (core, CLI) and Phase 1 (web) built 2026-10-07; Phase 2
reuse (known places in core, CLI and web) built 2026-10-08.**
Adds an optional `place` register to items: a named place, optionally
pinned to coordinates. Events use it for "where it happens"; tasks use it
for "where it can be done". The two share one field because they share
one value; only the consumer differs.

Companion to `data-model.md` (the register), `cli.md` (the verb),
`events-plan.md` (events are the first consumer). Location-triggered
notifications are deliberately out of scope here; see "Deferred".

## Decisions in one screen

| Question | Decision |
|---|---|
| One field for events and tasks, or two? | **One, `place`.** An event's "where it happens" and a task's "where it can be done" differ in intent, not in data. The consumer decides what to do with it (show it, later: fence it), the way `when` is one register that `is_event` reads differently. |
| Why not `location`? | Taken: `location` is the atomic placement register (`"<list_id>:<placement_id>"`, `data-model.md`). |
| Shape | **One atomic plain `LoroValue` map** `{label, lat?, lon?, address?, ref?}`, set whole, the `lifecycle` pattern as a map rather than a list. A label and its coordinates can never tear under concurrent edit: two devices writing different places resolve to one of them, never a label from one with the coordinates of the other. |
| Is label-only a place? | **Yes.** "Dinner at Luigi's" needs nothing more. Coordinates are an optional pair (both or neither). |
| A registry of saved places? | **No** (matches the flat-tags call in `tags-not-projects`). Reuse comes from the places already on items, deduped by label and rounded coordinates and ranked by recency. The value is self-contained, so an item never dangles. Built 2026-10-08; see "Reuse". |
| Where do coordinates come from? | **A geocoder, search-as-you-type.** Tier order per client: a platform geocoder where one exists (MapKit on Apple, keyless), else a server-configured provider, else none (label only, pasted coordinates). The web client's first cut uses Komoot's public Photon instance directly from the browser; see "Geocoding". Mapbox was considered and dropped 2026-10-07: its default geocoding forbids storing results, permanent mode has no free tier, and a browser token is the wrong shape for a self-hosted tool. |
| Is the lookup E2EE? | **No, and the spec says so.** The query text goes to whoever runs the geocoder. The stored value is encrypted like every register. Nothing else about the item leaves the device. |
| Notifications | **Out of scope** (decided 2026-10-07). A later `trigger` register (arrive / leave) and on-device geofencing on mobile; the server never learns a location. The place value carries nothing notification-specific (no radius) until then. |
| Schema | **Additive within v4.** A new optional key on the item map; no new container, no version bump, no import step. |

## Field: `place`

```
place = {
  label:   string      // required; the user-facing name, trimmed, 1..=200 chars
  lat:     f64?        // WGS84 decimal degrees, -90..=90; set with `lon` or not at all
  lon:     f64?        // WGS84 decimal degrees, -180..=180
  address: string?     // display-only formatted address, trimmed, <=500 chars
  ref:     string?     // opaque provider hint, e.g. "osm:node/123"; nothing depends on it
}
```

- **Atomic.** Written as one plain map value on the item's `LoroMap`, never
  as a child container. Whole-value LWW.
- **Absent ≡ unset.** Clearing deletes the key.
- **Canonical on write.** `Place::normalized` trims every string, drops
  empty optionals, and rejects: an empty or over-long label; a lone `lat`
  or `lon`; a non-finite or out-of-range coordinate; an over-long detail.
  A rejected value returns `Invalid` and leaves the doc untouched.
- **Lenient on read.** A value that is not a well-formed place map (a
  stray write, a newer shape, a lone coordinate) reads as unset; an
  integer coordinate from a lax writer reads as a float. Export dumps
  carry `place` only when set; import drops a malformed one rather than
  failing.
- **`ref`** is a hint for re-resolving against the provider that produced
  the place. A provider going away loses nothing: label, coordinates and
  address stand on their own.

## Mutation and events

- `set_item_place(item_id, Option<&Place>)` — canonicalise, insert or
  delete, one commit, `ItemPlaceChanged { id, place }`.
- `ItemAdded` carries `place`. The full-resync, diff-translation and
  bulk-diff paths all emit `ItemPlaceChanged` on a change.
- Hashed into the logical fingerprint (label, coordinate bits, address,
  ref).
- wasm: `setItemPlace(itemId, placeJson?)` on `Doc` and `SyncEngine`,
  JSON text in; `AppEventJs.place` is JSON text out; the workspace
  snapshot carries `place` as an object.

## Geocoding

A lookup turns typed text into a place with coordinates. It is the only
part of this feature that talks to anything outside the device, so it
is opt-in per action and never automatic.

**Web, first cut: Komoot's public Photon instance**
(`photon.komoot.io`, github.com/komoot/photon), called from the browser.
Photon is built for search-as-you-type from three characters; its public
instance asks for a reasonable rate, throttles extensive use, and gives no
availability guarantee. Public Nominatim was the first cut for a few hours
and was dropped because its usage policy forbids client-side autocomplete.
The client's shape:

- **Debounced on input.** A lookup fires 350 ms after the last keystroke
  once three characters are in and the text differs from the stored label.
  Each keystroke restarts the clock and abandons the lookup in flight.
- **Serialised with a gap.** Requests go through one queue with a minimum
  gap, so a burst drains politely rather than in parallel.
- **Results are cached** in memory by normalised query for the tab's
  life, so a repeated query never re-hits the service.
- **Language** is passed only when Photon indexes it (en, de, fr, it);
  anything else gets the default names.
- **Attribution.** The results popover carries "© OpenStreetMap
  contributors" (ODbL).
- **Location bias** (built 2026-10-07). Each request carries `lat`,
  `lon` and `zoom` so an ambiguous name ranks the nearby match first.
  The point comes from `locationBias.ts`: a geolocation fix if one is
  stored (zoom 12), else the principal city of the browser's reported
  time zone (zoom 9, a wider nudge). The fix is attempted once per boot,
  fire-and-forget: the browser prompts the first time and remembers the
  answer per origin, a denial or timeout leaves things as they were, and
  a stored fix under an hour old skips the call. Fixes are rounded to
  two decimals (~1km) before storage (localStorage, not per account) or
  sending. The time zone table is `tz-coords.ts`, generated by
  `scripts/vendor-tz.ts` from IANA `zone.tab` plus `backward` links, so
  the alias names ICU-based browsers still report resolve too; `Etc/*`
  and `UTC` carry no point and give no bias.

Mapping a feature to a place: `label` is `properties.name`, falling back
to the first address part; `address` is the remaining parts
(house number + street, district, city, state, country) joined, with the
label's own part left out; `lat` / `lon` come from the GeoJSON point
(`[lon, lat]`) and are range-checked; `ref` is `osm:<node|way|relation>/<osm_id>`.

**Privacy.** The query text (what the user types into the place field)
and the bias point (a ~1km-rounded geolocation fix or the time zone's
city) are seen by Komoot and are not covered by E2EE. The Monoplan server sees
nothing: the browser calls Photon directly, so there is no proxy to log
it, and no key to protect.

**Later tiers** (not built): a server-configured provider behind a
proxy route, so self-hosters get one knob and the SaaS can run its own
Photon when the public instance's fair use runs out (a self-hosted Photon
is a jar plus an index; JVM-free options such as open-geocode or Airmail
are worth re-checking then); MapKit on Apple clients, keyless and
on-platform; a "use current location" button via the geolocation API
with no third party at all (the boot-time bias fix above already holds
the permission, so the button would reuse it). Precedence per client: platform, then
server-configured, then none.

## Reuse

The doc is the cache. Every pick is stored on an item, encrypted and
synced, so "the places I use" is a derived view, the same on every
device, with no registry and no per-device history to drift. The web
geocoder cache (above) still spares the service a repeated query within
a tab; this is what spares the user the query at all.

**Core query.** `Doc::place_suggestions(query, limit) ->
Vec<PlaceSuggestion { place, count, last_used }>` (`places.rs`), a
sibling of `Doc::search`. A full scan per call, not an index: distinct
places are few, and the view is derived from the `ItemView`s the doc
already produces. wasm: `placeSuggestionsJson(query, limit)` on `Doc`
and `SyncEngine`, JSON out.

- **Which items count.** Everything not binned. Done items count: last
  week's done gym visit still makes "Gym" current. Archived lists count.
- **Recency.** A place's `last_used` is the newest, across its items, of
  `created_at`, `lifecycle_at` (the last state move, so a Done stamps it)
  and `when` read as UTC midnight (a floating stand-in good enough to
  rank by; a future `when` sorts first, since that place is being planned
  around). There is no `touched_at` on items; when one lands it joins
  this max.
- **Dedup.** Group by the folded label (the search tokenizer, so case,
  accents and punctuation never split one place), then by coordinates
  rounded to ~4 decimals (~10 m). Label-only uses fold into the most
  recent coordinated bucket of the same label and lend it their count
  and recency, so a CLI-typed "gym" and a geocoded "Gym" are one
  suggestion, the one with coordinates. Two "Office"s with different
  coordinates stay separate; the address tells them apart. Each bucket
  shows its most recently used value (casing, address, ref).
- **Order.** Newest use first, then most used, then label.
- **Query.** Every query token must prefix a token of the label or the
  address (`matches_name`), so "geo" finds a place at "1 George St".
  Empty matches all. Shared by `monoplan places <query>` and the web
  field's narrowing (the web calls once per popover open with an empty
  query and narrows the result with the same `matchesName`).

**Web field.** Two tiers in one popover, known rows (a small clock glyph
after the label) above lookup rows, ruled off when both show; the
OpenStreetMap credit only with lookup rows.

- Focusing the blank field lists the six most recent places. No network.
- Typing narrows the known places from the first character, no
  debounce, no three-character floor.
- From three characters the Photon lookup runs as before and lands under
  the known rows, minus any row that repeats a shown known place (same
  provider ref, or same rounded coordinates). The known row wins: it
  carries the label the user chose. The lookup's "No results" / "Lookup
  failed" notes show only when no known row is showing.
- Enter on text whose folded label equals a known place's reuses that
  place whole, coordinates included, instead of writing a label-only
  place and losing them. A prefix is not a match.

**CLI.** `monoplan places [query] [--limit N] [--json]` lists the known
places newest first, `<label>  <address | @lat,lon>  (<count>)`. `place
<id> <label>` with no `--at` / `--address` and a label that folds equal
to a known place's writes that place whole; `--new` writes the label as
given. So the CLI, which has no geocoder, still gets coordinates for
anything the web client has looked up once.

## Web

- **Field.** `PlaceField` in the task dialog, under the deadline band.
  The input holds the label and is editable: Enter or blur with a
  changed label writes a label-only place (the old coordinates described
  the old label); emptying it clears. Typing runs the debounced lookup
  and lists results under the input; arrow down moves into them, and
  picking one writes the whole place. Known places (see "Reuse") show
  first: on focus when blank, narrowed as you type. With coordinates
  present, the address shows muted under the input with an "Open map"
  link to openstreetmap.org.
- **Badge.** `PlaceBadge` (pin glyph + label, address on hover) on rows
  beside the `when` badge, and on calendar rows. Muted on done / binned.
- **Copy, paste, duplicate** carry the place with the item.
- Not indexed by search (same call as dates).

## CLI

- `monoplan place <item_id> <label | -> [--at <lat>,<lon>] [--address <text>] [--new]`
- `monoplan places [query] [--limit N] [--json]`
- `ls` / `agenda` / `focus` rows append ` at:<label>`; `--json` adds a
  `place` object when set.
- No lookup from the CLI. The CLI sets a label and, if the user has
  them, coordinates; a label naming a known place takes its coordinates
  (see "Reuse").

## Deferred, with their extension points

- **Server-configured provider.** A `geocoder` section in `server.toml`
  and a proxy route; the web client's `searchPlaces` gets a provider
  behind it. The place shape is provider-neutral already.
- **Platform geocoder** on Apple clients: MapKit's local search
  completer; `ref` would carry an `apple:` hint.
- **Current location** via `navigator.geolocation` / Core Location: a
  label-only place gains coordinates with no third party.
- **Notifications.** A `trigger` register beside `place` (arrive /
  leave), geofences armed on-device from the synced doc, deduped by
  coordinates (iOS caps regions at 20). A `radius_m` field is additive
  when that lands.
- **Calendar placement by place** (a "where" grouping) is not planned.

## Testing

- Core: set / clear / canonicalise; rejection table; malformed raw
  values read as unset; export / import; two-peer convergence with
  concurrent writes resolving to one whole value. Suggestions
  (`places.rs`): `when` to millis, ranking by newest use then count,
  label folding, label-only folding into the coordinated bucket,
  distinct coordinates kept apart, bin skipped, query on label or
  address; a doc-level check through `set_item_place` / `set_item_when`.
- CLI: `parse_coords`; `known_place` needs a whole-label match; `places`
  row formatting.
- Web: `photonToPlace` mapping; clip round-trip; store event handling
  via the wasm doc test; `placeSuggestionsJson` round trip; the field's
  narrowing, exact match and lookup dedup (`placeSuggest.test.ts`).

## Phases

- **Phase 0 (core, CLI)** — built 2026-10-07.
- **Phase 1 (web)** — built 2026-10-07: field, Photon lookup, badge,
  clip.
- **Phase 2** — reuse of known places built 2026-10-08 (core query, CLI
  `places` + label resolution, web two-tier field). Current location
  still open.
- **Phase 3** — server-configured provider; platform geocoder on native
  clients.
