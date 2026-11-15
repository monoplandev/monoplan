//! JSON export / import of one workspace (`spec/data-model.md` "JSON
//! export / import"): the readable, lossy dump the web's workspace
//! menu saves and loads, for the apps' typed calls. The CLI's
//! `workspace export-json` renders the same export.

use serde::Serialize;

use super::Ctx;

/// The workspace as pretty-printed JSON: its lists and items, no CRDT
/// history.
pub fn export_json(cx: &Ctx) -> anyhow::Result<String> {
    Ok(cx.doc.export_json_string())
}

/// What an import added and what it left alone
/// (`monoplan_core::ImportSummary`, with counts a Swift `Int` holds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImportCounts {
    pub lists_added: u32,
    pub items_added: u32,
    /// Items dropped for empty text.
    pub items_skipped: u32,
    /// User lists already in the workspace by id, left untouched.
    pub lists_existing: u32,
    /// Items already in the workspace by id, left untouched.
    pub items_existing: u32,
}

/// Additive, idempotent import of an export into the workspace: rows
/// whose id is already there are skipped. An invalid file is refused
/// whole.
pub fn import_json(cx: &Ctx, json: &str) -> anyhow::Result<ImportCounts> {
    let s = cx.doc.import_json_str(json)?;
    let n = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    Ok(ImportCounts {
        lists_added: n(s.lists_added),
        items_added: n(s.items_added),
        items_skipped: n(s.items_skipped),
        lists_existing: n(s.lists_existing),
        items_existing: n(s.items_existing),
    })
}
