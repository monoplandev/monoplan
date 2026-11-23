// A `when` fixed to a time zone, as the Mac panel's and the iOS
// editor's date and time fields read and write it
// (`spec/calendar-plan.md` "Fixed instants"; the web's `zoneWall` /
// `fixedWhen` / `localWhen` / `storedWhen` in `js/web/src/zones.ts`).
//
// Every row and lens shows a fixed item at the device's wall time,
// which is what `Item.when` carries. The fields that edit the date are
// the one place that reads the zone's own clock: with a zone they show
// `wall(...)` and write what is typed there back as `fixed(...)`.
// Every conversion is the core's (`whenInZone`, `whenLocalized`), so
// the hours a zone skips or repeats resolve as they do everywhere else.

import Foundation

public enum WhenZone {
    /// A fixed `when` as its own zone's clock reads it. The
    /// device-local value where there is no zone, no time, or the zone
    /// cannot be converted (the core shows such a value as written, so
    /// that is the zone's clock too).
    public static func wall(_ when: String, zone: String?) -> String {
        zone.flatMap { whenInZone(when: when, zone: $0) } ?? when
    }

    /// The register value for a time read on `zone`'s own clock: the
    /// inverse of `wall`, for writing an edit of those fields. Plain
    /// (the core drops the zone for an all-day value, and keeps one it
    /// cannot convert) when there is no zone to name.
    public static func fixed(_ wall: String, zone: String?) -> String {
        guard let zone, whenInZone(when: wall, zone: zone) != nil else { return wall }
        return "\(wall)[\(zone)]"
    }

    /// The device-local time of a value read on `zone`'s clock.
    public static func local(_ wall: String, zone: String?) -> String {
        whenLocalized(stored: fixed(wall, zone: zone))
    }

    /// The register value for a device-local `when` fixed to `zone`:
    /// what `Engine.setWhen` takes to write both at once (a capture,
    /// putting a date back). The floating value where it cannot be
    /// converted.
    public static func stored(_ when: String, zone: String?) -> String {
        guard let zone, let wall = whenInZone(when: when, zone: zone) else { return when }
        return "\(wall)[\(zone)]"
    }

    /// A register value's wall-clock part and its zone id:
    /// `2026-07-13T14:00[Europe/Paris]` is the time and "Europe/Paris";
    /// anything else is itself and no zone.
    public static func split(_ stored: String) -> (wall: String, zone: String?) {
        guard stored.count > 18, stored.hasSuffix("]") else { return (stored, nil) }
        let wall = stored.prefix(16)
        let rest = stored.dropFirst(16)
        guard rest.hasPrefix("[") else { return (stored, nil) }
        return (String(wall), String(rest.dropFirst().dropLast()))
    }

    /// Whether the core's tzdb knows `zone`, so a pick of it can be
    /// written.
    public static func knows(_ zone: String) -> Bool {
        whenInZone(when: "2000-01-01T12:00", zone: zone) != nil
    }
}
