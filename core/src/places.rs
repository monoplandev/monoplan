//! Place suggestions (`spec/place-plan.md` "Reuse"): the places already
//! on items, deduped and ranked by recency, so a field can offer "Home"
//! or "Gym" before any geocoder is asked. There is no registry; the doc
//! is the cache. Every pick is stored on an item, encrypted and synced,
//! so the same list comes up on every device.
//!
//! A full scan per call: distinct places are few, and the view is
//! derived from `ItemView`s the doc already produces, so no index or
//! dirty-set is kept. `Doc::place_suggestions` is the entry point;
//! [`suggest_places`] is the pure function under it.

use std::collections::HashMap;

use serde::Serialize;

use crate::doc::{ItemView, Place};
use crate::search::{matches_name, tokenize};

/// One deduped place with its usage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceSuggestion {
    pub place: Place,
    /// Items (not binned) carrying this place.
    pub count: usize,
    /// Unix millis of the newest use: the latest of the carrying items'
    /// `created_at`, `lifecycle_at` and `when` (the planned day read as
    /// UTC midnight, a floating stand-in good enough to rank by).
    pub last_used: i64,
}

/// Coordinates rounded to ~4 decimals (~10 m) so two picks of one place
/// from the same or different providers fold together.
fn coord_bucket(lat: f64, lon: f64) -> (i64, i64) {
    ((lat * 1e4).round() as i64, (lon * 1e4).round() as i64)
}

/// The dedup key for a label: the search folding (`spec/search.md`
/// "Normalization"), so case, accents and punctuation never split one
/// place into two.
pub fn fold_label(label: &str) -> String {
    tokenize(label).join(" ")
}

/// Days since 1970-01-01 for a proleptic Gregorian civil date
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A stored `when` (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`, already
/// validated on write) as unix millis, read as UTC. Floating dates have
/// no instant; this is a ranking key, not a time. `None` on anything
/// malformed.
fn when_millis(when: &str) -> Option<i64> {
    let num = |s: &str| -> Option<i64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            None
        } else {
            s.parse().ok()
        }
    };
    if when.len() < 10 {
        return None;
    }
    let y = num(when.get(0..4)?)?;
    let m = num(when.get(5..7)?)? as u32;
    let d = num(when.get(8..10)?)? as u32;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut millis = days_from_civil(y, m, d) * 86_400_000;
    if when.len() == 16 {
        let h = num(when.get(11..13)?)?;
        let min = num(when.get(14..16)?)?;
        millis += (h * 60 + min) * 60_000;
    }
    Some(millis)
}

/// When an item last "used" its place: the newest of its creation, its
/// last lifecycle move and its planned day. A future `when` counts as
/// newest of all: the place is being planned around.
fn use_time(item: &ItemView) -> i64 {
    let mut t = item.created_at.max(item.lifecycle_at);
    if let Some(w) = item.when.as_deref().and_then(when_millis) {
        t = t.max(w);
    }
    t
}

struct Bucket {
    place: Place,
    count: usize,
    last_used: i64,
}

impl Bucket {
    fn absorb(&mut self, place: &Place, at: i64) {
        self.count += 1;
        if at > self.last_used {
            self.last_used = at;
            self.place = place.clone();
        }
    }
}

/// One folded label's uses: coordinated buckets keyed by rounded
/// coordinates, and the label-only bucket, if any.
type LabelGroup = (Vec<((i64, i64), Bucket)>, Option<Bucket>);

/// Whether a suggestion matches a typed query: every query token must
/// prefix a token of the label or the address, so "geo" finds both
/// "George St Gym" and a place at "1 George St". An empty query matches
/// everything.
pub fn place_matches(place: &Place, query: &str) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    match place.address.as_deref() {
        Some(addr) => matches_name(&format!("{} {addr}", place.label), query),
        None => matches_name(&place.label, query),
    }
}

/// Dedupe and rank the places on `items`, filtered by `query`, at most
/// `limit` (0 = unlimited).
///
/// - Binned items are skipped; Done ones count (last week's gym visit
///   still makes "Gym" current).
/// - Places group by folded label, then by coordinates rounded to ~4
///   decimals. Label-only uses fold into the most recent coordinated
///   bucket of the same label, so a CLI-typed "Gym" and a geocoded
///   "Gym" are one suggestion, the one with coordinates.
/// - Each bucket shows the most recently used value (its casing,
///   address, ref).
/// - Ordered newest use first, then most used, then by label.
pub fn suggest_places<'a>(
    items: impl IntoIterator<Item = &'a ItemView>,
    query: &str,
    limit: usize,
) -> Vec<PlaceSuggestion> {
    let mut groups: HashMap<String, LabelGroup> = HashMap::new();
    for item in items {
        if item.is_binned() {
            continue;
        }
        let Some(place) = item.place.as_ref() else {
            continue;
        };
        let key = fold_label(&place.label);
        if key.is_empty() {
            continue;
        }
        let at = use_time(item);
        let group = groups.entry(key).or_insert_with(|| (Vec::new(), None));
        match (place.lat, place.lon) {
            (Some(lat), Some(lon)) => {
                let bucket = coord_bucket(lat, lon);
                match group.0.iter_mut().find(|(b, _)| *b == bucket) {
                    Some((_, existing)) => existing.absorb(place, at),
                    None => group.0.push((
                        bucket,
                        Bucket {
                            place: place.clone(),
                            count: 1,
                            last_used: at,
                        },
                    )),
                }
            }
            _ => match group.1.as_mut() {
                Some(existing) => existing.absorb(place, at),
                None => {
                    group.1 = Some(Bucket {
                        place: place.clone(),
                        count: 1,
                        last_used: at,
                    })
                }
            },
        }
    }

    let mut out: Vec<PlaceSuggestion> = Vec::new();
    for (_, (mut coordinated, plain)) in groups {
        if let Some(plain) = plain {
            // Label-only uses lend their count and recency to the most
            // recent coordinated bucket, but never its value.
            match coordinated.iter_mut().max_by_key(|(_, b)| b.last_used) {
                Some((_, target)) => {
                    target.count += plain.count;
                    target.last_used = target.last_used.max(plain.last_used);
                }
                None => out.push(PlaceSuggestion {
                    place: plain.place,
                    count: plain.count,
                    last_used: plain.last_used,
                }),
            }
        }
        out.extend(coordinated.into_iter().map(|(_, b)| PlaceSuggestion {
            place: b.place,
            count: b.count,
            last_used: b.last_used,
        }));
    }
    out.retain(|s| place_matches(&s.place, query));
    out.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then(b.count.cmp(&a.count))
            .then_with(|| a.place.label.cmp(&b.place.label))
    });
    if limit > 0 {
        out.truncate(limit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::WorkflowState;

    fn item(id: &str, created_at: i64, place: Option<Place>) -> ItemView {
        ItemView {
            id: id.to_string(),
            text: id.to_string(),
            notes: String::new(),
            list_id: "inbox".into(),
            state: WorkflowState::Backlog,
            lifecycle_at: created_at,
            deadline: None,
            when: None,
            duration: None,
            place,
            created_at,
            started_at: None,
            done_at: None,
            binned_at: None,
        }
    }

    fn place(label: &str, coords: Option<(f64, f64)>) -> Place {
        Place {
            label: label.to_string(),
            lat: coords.map(|c| c.0),
            lon: coords.map(|c| c.1),
            address: None,
            reference: None,
        }
    }

    fn labels(s: &[PlaceSuggestion]) -> Vec<&str> {
        s.iter().map(|s| s.place.label.as_str()).collect()
    }

    #[test]
    fn when_millis_reads_dates_and_times() {
        assert_eq!(when_millis("1970-01-01"), Some(0));
        assert_eq!(
            when_millis("1970-01-02T01:30"),
            Some(86_400_000 + 90 * 60_000)
        );
        assert_eq!(when_millis("2000-03-01"), Some(951_868_800_000));
        assert_eq!(when_millis("2026-13-01"), None);
        assert_eq!(when_millis("nope"), None);
    }

    #[test]
    fn ranks_by_newest_use_then_count() {
        let items = [
            item("a", 100, Some(place("Gym", None))),
            item("b", 300, Some(place("Home", None))),
            item("c", 200, Some(place("Gym", None))),
            item("d", 50, Some(place("Office", None))),
            item("e", 10, None),
        ];
        let got = suggest_places(&items, "", 0);
        assert_eq!(labels(&got), ["Home", "Gym", "Office"]);
        assert_eq!(got[1].count, 2);
        assert_eq!(got[1].last_used, 200);
    }

    #[test]
    fn future_when_and_lifecycle_moves_count_as_use() {
        let mut planned = item("a", 100, Some(place("Cafe", None)));
        planned.when = Some("2030-01-01".into());
        let mut done = item("b", 200, Some(place("Gym", None)));
        done.lifecycle_at = 5_000_000_000_000; // moved to Done long after creation
        let fresh = item("c", 1_000_000_000_000, Some(place("Home", None)));
        let got = suggest_places(&[planned, done, fresh], "", 0);
        assert_eq!(labels(&got), ["Gym", "Cafe", "Home"]);
    }

    #[test]
    fn folds_case_accents_and_punctuation_in_labels() {
        let items = [
            item("a", 1, Some(place("Luigi's", None))),
            item("b", 2, Some(place("LUIGI'S", None))),
            item("c", 3, Some(place("Café Noir", None))),
            item("d", 4, Some(place("cafe noir", None))),
        ];
        let got = suggest_places(&items, "", 0);
        assert_eq!(labels(&got), ["cafe noir", "LUIGI'S"]);
        assert!(got.iter().all(|s| s.count == 2));
    }

    #[test]
    fn label_only_folds_into_the_coordinated_bucket() {
        let items = [
            item("a", 1, Some(place("Gym", Some((-33.8688, 151.2093))))),
            item("b", 9, Some(place("gym", None))),
            item("c", 2, Some(place("Gym", Some((-33.86881, 151.20929))))),
        ];
        let got = suggest_places(&items, "", 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].count, 3);
        assert_eq!(got[0].last_used, 9);
        // The value is the coordinated one, with the newest coordinated
        // casing, never the label-only "gym".
        assert_eq!(got[0].place.lat, Some(-33.86881));
        assert_eq!(got[0].place.label, "Gym");
    }

    #[test]
    fn distinct_coordinates_stay_separate_under_one_label() {
        let items = [
            item("a", 1, Some(place("Office", Some((1.0, 2.0))))),
            item("b", 2, Some(place("Office", Some((3.0, 4.0))))),
            item("c", 3, Some(place("Office", None))),
        ];
        let got = suggest_places(&items, "", 0);
        assert_eq!(got.len(), 2);
        // The label-only use went to the newer coordinated bucket.
        assert_eq!(got[0].place.lat, Some(3.0));
        assert_eq!(got[0].count, 2);
        assert_eq!(got[0].last_used, 3);
        assert_eq!(got[1].count, 1);
    }

    #[test]
    fn binned_items_are_skipped_and_query_filters_by_label_or_address() {
        let mut binned = item("a", 9, Some(place("Secret", None)));
        binned.binned_at = Some(10);
        let mut addressed = item("b", 2, Some(place("Work", Some((1.0, 1.0)))));
        addressed.place.as_mut().unwrap().address = Some("1 George St, Sydney".into());
        let items = [binned, addressed, item("c", 3, Some(place("Gym", None)))];
        assert_eq!(labels(&suggest_places(&items, "", 0)), ["Gym", "Work"]);
        assert_eq!(labels(&suggest_places(&items, "g", 0)), ["Gym", "Work"]);
        assert_eq!(labels(&suggest_places(&items, "geo", 0)), ["Work"]);
        assert_eq!(
            labels(&suggest_places(&items, "sec", 0)),
            Vec::<&str>::new()
        );
        assert_eq!(labels(&suggest_places(&items, "", 1)), ["Gym"]);
    }
}
