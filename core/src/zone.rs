//! Time zones for `when` (`spec/calendar-plan.md` "Fixed instants").
//!
//! A stored `when` is floating (`YYYY-MM-DD`, `YYYY-MM-DDTHH:MM`) or
//! fixed: a timed value with an RFC 9557 bracketed IANA zone,
//! `2026-07-13T14:00[Europe/London]`, which names one instant. The doc
//! keeps the wall time and the zone, never the instant, so a rule change
//! moves the instant with it.
//!
//! Views never see the bracketed form. They get the value re-expressed
//! as wall time in the device's zone, which has the floating shape, so
//! sorting and day bucketing stay string compares. This module is the
//! one place that arithmetic lives; the tz rules come from `jiff`
//! (system tzdata on unix, a bundled copy on wasm and windows).
//!
//! Hours a zone skips or repeats resolve the way `jiff` calls
//! "compatible" (RFC 5545's rule): a skipped time moves forward by the
//! gap, a repeated time takes the first occurrence.

use std::cell::RefCell;

use jiff::civil::DateTime;
use jiff::tz::TimeZone;

/// Longest zone id accepted. The longest in the tzdb is about 30 bytes.
const MAX_ZONE_BYTES: usize = 64;

/// Length of the wall-clock part of a timed value, `YYYY-MM-DDTHH:MM`.
const WALL_LEN: usize = 16;

thread_local! {
    static DEVICE_OVERRIDE: RefCell<Option<TimeZone>> = const { RefCell::new(None) };
}

/// Pin the device zone for the calling thread (`None` goes back to the
/// system's). For tests, which must not depend on the machine's zone.
/// Panics on a zone the tzdb does not know.
#[doc(hidden)]
pub fn set_thread_device_zone(zone: Option<&str>) {
    let tz = zone.map(|z| TimeZone::get(z).expect("known zone"));
    DEVICE_OVERRIDE.with(|o| *o.borrow_mut() = tz);
}

/// The device's zone: the system's, or UTC when it cannot be told.
fn device() -> TimeZone {
    DEVICE_OVERRIDE
        .with(|o| o.borrow().clone())
        .unwrap_or_else(TimeZone::system)
}

/// The device zone's IANA id, when it has one.
pub fn device_zone() -> Option<String> {
    device().iana_name().map(str::to_string)
}

/// Split a stored `when` into its wall-clock part and its zone id.
/// Purely by shape: `None` for a floating value or anything that is not
/// `<16 chars>[<zone>]`.
pub fn split(stored: &str) -> (&str, Option<&str>) {
    let Some(rest) = stored.get(WALL_LEN..) else {
        return (stored, None);
    };
    match rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        Some(zone) if !zone.is_empty() => (&stored[..WALL_LEN], Some(zone)),
        _ => (stored, None),
    }
}

/// Whether `zone` has the shape of an IANA id: non-empty, bounded,
/// ASCII letters, digits and `_ - + /`. Says nothing about whether the
/// tzdb knows it; see [`is_known`].
pub fn is_well_formed(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= MAX_ZONE_BYTES
        && zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'/'))
}

/// Whether this device's tzdb knows `zone`. Devices can differ: a zone
/// added in a newer tzdata is unknown to an older one.
pub fn is_known(zone: &str) -> bool {
    is_well_formed(zone) && TimeZone::get(zone).is_ok()
}

/// Join a wall-clock part and a zone id into the stored form.
pub fn join(wall: &str, zone: &str) -> String {
    format!("{wall}[{zone}]")
}

fn parse_wall(wall: &str) -> Option<DateTime> {
    wall.parse().ok()
}

fn format_wall(dt: DateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute()
    )
}

/// The wall time in `to` at the instant `wall` names in `from`. `None`
/// when `wall` is malformed or the result leaves the four-digit years.
fn convert(wall: &str, from: &TimeZone, to: &TimeZone) -> Option<String> {
    let there = parse_wall(wall)?
        .to_zoned_datetime(from.clone())
        .ok()?
        .with_time_zone(to.clone())
        .datetime();
    (0..=9999)
        .contains(&there.year())
        .then(|| format_wall(there))
}

fn localize_in(stored: &str, device: &TimeZone) -> String {
    let (wall, Some(zone)) = split(stored) else {
        return stored.to_string();
    };
    TimeZone::get(zone)
        .ok()
        .and_then(|tz| convert(wall, &tz, device))
        .unwrap_or_else(|| wall.to_string())
}

/// A stored `when` as views show it: floating values as they are, a
/// fixed one as wall time in the device's zone. A zone this device does
/// not know cannot be converted, so its wall time is shown as written.
pub fn localize(stored: &str) -> String {
    localize_in(stored, &device())
}

/// The wall time in `zone` at the instant the device-local `local`
/// names: the inverse of [`localize`]. `None` when the zone is unknown.
pub fn wall_in_zone(local: &str, zone: &str) -> Option<String> {
    let tz = TimeZone::get(zone).ok()?;
    convert(local, &device(), &tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tz(name: &str) -> TimeZone {
        TimeZone::get(name).unwrap()
    }

    #[test]
    fn split_reads_the_bracketed_suffix_only() {
        assert_eq!(split("2026-07-13"), ("2026-07-13", None));
        assert_eq!(split("2026-07-13T14:00"), ("2026-07-13T14:00", None));
        assert_eq!(
            split("2026-07-13T14:00[Europe/London]"),
            ("2026-07-13T14:00", Some("Europe/London"))
        );
        // Not the shape: left whole, no zone.
        assert_eq!(split("2026-07-13T14:00[]").1, None);
        assert_eq!(split("2026-07-13T14:00[Europe/London").1, None);
        assert_eq!(split("2026-07-13[Europe/London]").1, None);
    }

    #[test]
    fn zone_ids_are_checked_by_shape_then_by_the_tzdb() {
        assert!(is_well_formed("America/Argentina/Buenos_Aires"));
        assert!(is_well_formed("Etc/GMT+5"));
        assert!(!is_well_formed(""));
        assert!(!is_well_formed("Europe/Lon don"));
        assert!(!is_well_formed("Europe/London]"));
        assert!(is_known("Europe/London"));
        assert!(!is_known("Europe/Atlantis"));
    }

    #[test]
    fn a_fixed_value_shows_in_the_device_zone() {
        let paris = tz("Europe/Paris");
        assert_eq!(
            localize_in("2026-07-13T14:00[Europe/London]", &paris),
            "2026-07-13T15:00"
        );
        // Across the date line the day moves with it.
        assert_eq!(
            localize_in(
                "2026-07-13T20:00[America/Los_Angeles]",
                &tz("Australia/Melbourne")
            ),
            "2026-07-14T13:00"
        );
        // The offset is the one in force on the date: London is on GMT
        // in January, an hour behind Paris all year.
        assert_eq!(
            localize_in("2026-01-13T14:00[Europe/London]", &tz("UTC")),
            "2026-01-13T14:00"
        );
        assert_eq!(
            localize_in("2026-07-13T14:00[Europe/London]", &tz("UTC")),
            "2026-07-13T13:00"
        );
    }

    #[test]
    fn floating_and_unknown_zones_pass_through() {
        let paris = tz("Europe/Paris");
        assert_eq!(localize_in("2026-07-13", &paris), "2026-07-13");
        assert_eq!(localize_in("2026-07-13T14:00", &paris), "2026-07-13T14:00");
        assert_eq!(
            localize_in("2026-07-13T14:00[Europe/Atlantis]", &paris),
            "2026-07-13T14:00"
        );
    }

    #[test]
    fn skipped_and_repeated_hours_resolve_compatibly() {
        let utc = tz("UTC");
        // 01:30 does not exist in London on 2026-03-29 (clocks go 01:00
        // to 02:00): it moves forward by the gap, to 02:30 BST, which
        // is 03:30 in Paris.
        assert_eq!(
            localize_in("2026-03-29T01:30[Europe/London]", &tz("Europe/Paris")),
            "2026-03-29T03:30"
        );
        // 01:30 happens twice on 2026-10-25: the first, still on BST.
        assert_eq!(
            localize_in("2026-10-25T01:30[Europe/London]", &utc),
            "2026-10-25T00:30"
        );
    }

    #[test]
    fn wall_in_zone_inverts_localize() {
        set_thread_device_zone(Some("Europe/Paris"));
        assert_eq!(
            wall_in_zone("2026-07-13T15:00", "Europe/London").as_deref(),
            Some("2026-07-13T14:00")
        );
        assert_eq!(
            localize("2026-07-13T14:00[Europe/London]"),
            "2026-07-13T15:00"
        );
        assert_eq!(wall_in_zone("2026-07-13T15:00", "Europe/Atlantis"), None);
        assert_eq!(device_zone().as_deref(), Some("Europe/Paris"));
        set_thread_device_zone(None);
    }
}
