//! Reminders: what a device may notify about (`spec/reminders.md`).
//! The read only says which items have a time still ahead; when each
//! one fires, and whether it does, is the device's own setting.

use chrono::Local;
use serde::Serialize;

use super::{Ctx, doc_tag};

/// One item a device may remind about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Reminder {
    pub id: String,
    pub text: String,
    /// The list the item is in, for a client that opens the item
    /// where its row is.
    pub list_id: String,
    /// `YYYY-MM-DDTHH:MM`, floating: always timed.
    pub when: String,
    /// An event (`spec/events-plan.md`); otherwise a scheduled task.
    /// The device has a setting for each.
    pub event: bool,
    /// The item's workspace; absent where the caller has bare docs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
}

/// Whether a `when` carries a time (`YYYY-MM-DDTHH:MM`) and is not
/// all-day (`YYYY-MM-DD`).
fn timed(when: &str) -> bool {
    when.len() == 16
}

/// Every open workspace's Open items (any open workflow state, not
/// binned) with a timed `when` at or after `now`, soonest first. All
/// workspaces, the ones hidden from the calendar included, and whoever
/// the item is assigned to. `now` is `YYYY-MM-DDTHH:MM` local, the
/// clock's when `None`.
pub fn reminders(cx: &Ctx, now: Option<&str>) -> anyhow::Result<Vec<Reminder>> {
    let now = match now {
        Some(n) => n.to_string(),
        None => Local::now().format("%Y-%m-%dT%H:%M").to_string(),
    };
    let mut rows: Vec<Reminder> = cx
        .sources()
        .into_iter()
        .flat_map(|(tag, doc)| {
            let doc_id = doc_tag(tag).0;
            doc.all_items()
                .into_iter()
                .filter(|i| i.is_open())
                .filter_map(|i| {
                    // Both are `YYYY-MM-DDTHH:MM`, so they order as text.
                    let when = i.when.filter(|w| timed(w) && w.as_str() >= now.as_str())?;
                    Some(Reminder {
                        id: i.id,
                        text: i.text,
                        list_id: i.list_id,
                        when,
                        event: i.event,
                        doc_id: doc_id.clone(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    rows.sort_by(|a, b| (&a.when, &a.doc_id, &a.id).cmp(&(&b.when, &b.doc_id, &b.id)));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use monoplan_core::{Doc, LIST_INBOX};

    #[test]
    fn open_timed_items_ahead_of_now_across_docs_soonest_first() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let add = |doc: &Doc, text: &str, when: Option<&str>| {
            let id = doc.add_item(LIST_INBOX, text).unwrap();
            doc.set_item_when(&id, when).unwrap();
            id
        };
        let later = add(&home, "later", Some("2026-12-02T09:00"));
        let sooner = add(&work, "sooner", Some("2026-12-01T14:30"));
        let event = add(&work, "event", Some("2026-12-01T18:00"));
        work.set_item_event(&event, true).unwrap();
        let at_now = add(&home, "at now", Some("2026-12-01T12:00"));
        add(&home, "all-day", Some("2026-12-03"));
        add(&home, "undated", None);
        add(&home, "gone by", Some("2026-12-01T11:59"));
        let done = add(&work, "done", Some("2026-12-04T10:00"));
        let binned = add(&work, "binned", Some("2026-12-04T10:00"));
        work.set_item_done(&done, true).unwrap();
        work.set_item_binned(&binned, true).unwrap();

        let lens = Lens::home(&home);
        let docs = [&home, &work];
        let cx = Ctx::bare(&home, &docs, &lens);
        let rows = reminders(&cx, Some("2026-12-01T12:00")).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, [&at_now, &sooner, &event, &later]);
        let events: Vec<bool> = rows.iter().map(|r| r.event).collect();
        assert_eq!(events, [false, false, true, false]);
    }
}
