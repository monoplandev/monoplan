//! The combined Inbox's mux (`spec/inbox-plan.md` "One Inbox"): every
//! open workspace keeps its own `inbox` bucket, and the user sees one
//! queue over them, narrowed by a workspace filter. The filter, the
//! rows each doc contributes and the merge live here so every client
//! (the FFI's `api::inbox`, the wasm `InboxMux`) shows the same queue.

use crate::doc::{Doc, ItemView, LIST_INBOX};

/// The workspaces the Inbox shows, from the open ones (in their order)
/// and the filter: the filter's open workspaces, or every open one
/// when the filter is empty ("All workspaces") or names none that is
/// open (a stored filter outlives a workspace it named).
pub fn scope<'a, K: PartialEq>(open: &'a [K], filter: &[K]) -> Vec<&'a K> {
    let picked: Vec<&K> = open.iter().filter(|k| filter.contains(k)).collect();
    if picked.is_empty() {
        open.iter().collect()
    } else {
        picked
    }
}

/// The queue's sort key from an item's registers: `inbox_at`, else
/// `created_at`, then `created_at` (`spec/inbox-plan.md` "Order"). The
/// one definition: a doc's own Inbox projection sorts on it too.
pub fn key(inbox_at: Option<i64>, created_at: i64) -> (i64, i64) {
    (inbox_at.unwrap_or(created_at), created_at)
}

/// An item's place in the queue ([`key`]).
pub fn queue_key(item: &ItemView) -> (i64, i64) {
    key(item.inbox_at, item.created_at)
}

/// What one doc's Inbox contributes: its listed rows (Open, not
/// binned, no date), plus any row of its Inbox `keep` names that is
/// closed but not binned (a client's just-ticked row lingering in
/// place).
pub fn doc_rows(doc: &Doc, keep: impl Fn(&str) -> bool) -> Vec<ItemView> {
    doc.items_in_list(LIST_INBOX, false)
        .into_iter()
        .filter(|i| i.is_listed() || (!i.is_open() && i.when.is_none() && keep(&i.id)))
        .collect()
}

/// Merge the docs' rows into one queue, oldest first: the queue key,
/// then the workspace key and the item id, so equal times still order
/// the same way on every client.
pub fn merge<K: Ord>(rows: &mut [(K, ItemView)]) {
    rows.sort_by(|(ka, a), (kb, b)| {
        queue_key(a)
            .cmp(&queue_key(b))
            .then_with(|| ka.cmp(kb))
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_the_filter_or_everything() {
        let open = ["home", "work", "club"];
        let all: Vec<&&str> = open.iter().collect();
        assert_eq!(scope(&open, &[]), all);
        assert_eq!(scope(&open, &["club", "work"]), vec![&"work", &"club"]);
        // A filter naming only closed workspaces reads as no filter.
        assert_eq!(scope(&open, &["gone"]), all);
        assert_eq!(scope(&open, &["gone", "home"]), vec![&"home"]);
    }

    #[test]
    fn rows_keep_lingering_closed_rows_only_when_asked() {
        let doc = Doc::new().unwrap();
        let open = doc.add_item(LIST_INBOX, "open").unwrap();
        let done = doc.add_item(LIST_INBOX, "done").unwrap();
        doc.set_item_done(&done, true).unwrap();
        let binned = doc.add_item(LIST_INBOX, "binned").unwrap();
        doc.set_item_done(&binned, true).unwrap();
        doc.set_item_binned(&binned, true).unwrap();
        let ids = |rows: Vec<ItemView>| rows.into_iter().map(|i| i.id).collect::<Vec<_>>();
        assert_eq!(ids(doc_rows(&doc, |_| false)), std::slice::from_ref(&open));
        assert_eq!(ids(doc_rows(&doc, |_| true)), [open, done]);
    }
}
