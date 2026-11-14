//! Done: what got finished in the workspace, newest first
//! (`commands::done` for the selection rule and the terminal's days).

use super::{Ctx, items::Item};
use crate::commands::done::select_done;

/// The workspace's Done, not-binned items across every list, newest
/// first by the moment each entered Done; `since` (unix millis) and
/// `limit` bound them, `None` being unbounded. A full `limit` back
/// means there may be more.
pub fn done(cx: &Ctx, since: Option<i64>, limit: Option<usize>) -> anyhow::Result<Vec<Item>> {
    let items = cx.doc.all_items();
    Ok(select_done(&items, since, limit)
        .into_iter()
        .map(Item::from)
        .collect())
}

/// How many items [`done`] would return with no `limit`.
pub fn done_count(cx: &Ctx, since: Option<i64>) -> anyhow::Result<u32> {
    let items = cx.doc.all_items();
    Ok(select_done(&items, since, None).len() as u32)
}
