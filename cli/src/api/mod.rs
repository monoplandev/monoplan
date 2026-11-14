//! The typed layer under the commands (`spec/swift-bindings-plan.md`):
//! what a command computes or writes, as plain functions over a
//! [`Ctx`] returning owned rows, with the rendering left to the
//! caller. `commands/*` render them for the terminal and for `--json`;
//! `monoplan-ffi` exports them to Swift as they are, through
//! `LocalClient::call_blocking`.
//!
//! Row types derive `Serialize` (the `--json` shape) always and the
//! uniffi traits behind this crate's `uniffi` feature, which only
//! `monoplan-ffi` turns on. They are named for Swift, where they land
//! unprefixed in the app's module: nothing SwiftUI or MapKit exports
//! (`ItemState`, not `State`; `ListInfo`, not `List`), and nothing
//! `MonoplanMacKit` still declares while both are imported side by
//! side (`Item`, not its `ItemRow`; `AgendaGroup`, not its
//! `AgendaDay`; `FindHit`, not its `SearchHit`).
//!
//! A function runs against one workspace (`Ctx::doc`) unless it says
//! it spans them: those walk `Ctx::workspaces` and tag each row with
//! the doc it came from.
//!
//! Functions return `anyhow::Result`, as the commands they came out of
//! do; [`ApiError`] is what a caller outside the loop gets. A failure a
//! client has to branch on is raised as an `ApiError` inside the
//! `anyhow::Error` and comes back out as itself.

use monoplan_core::{Doc, DocId};

use crate::commands::focus::Lens;
use crate::workspace::Candidate;

pub mod agenda;
pub mod bin;
pub mod done;
pub mod find;
pub mod items;
pub mod lists;
pub mod places;
pub mod starred;

/// What a call runs against: the workspace it named, every open doc
/// (that one among them) for the account-scoped reads, and Starred as
/// seen from it. What `commands::run_doc` is given.
pub struct Ctx<'a> {
    pub doc: &'a Doc,
    pub open_docs: &'a [&'a Doc],
    pub lens: &'a Lens<'a>,
    /// Every open doc again, each with its workspace tag, Home first:
    /// what the reads that span workspaces walk and tag their rows
    /// with. Empty where the caller has bare docs and no workspaces to
    /// name (tests); those reads then cover `open_docs`, untagged.
    pub workspaces: &'a [(Candidate, &'a Doc)],
    /// The Home doc's id, when the caller knows its workspaces.
    pub home: Option<DocId>,
}

impl<'a> Ctx<'a> {
    /// A context over bare docs: no workspace tags, no Home.
    pub fn bare(doc: &'a Doc, open_docs: &'a [&'a Doc], lens: &'a Lens<'a>) -> Self {
        Ctx {
            doc,
            open_docs,
            lens,
            workspaces: &[],
            home: None,
        }
    }

    /// Every open doc with its tag, or untagged where there are none.
    pub(crate) fn sources(&self) -> Vec<(Option<&'a Candidate>, &'a Doc)> {
        if self.workspaces.is_empty() {
            self.open_docs.iter().map(|d| (None, *d)).collect()
        } else {
            self.workspaces.iter().map(|(c, d)| (Some(c), *d)).collect()
        }
    }

    /// The Home doc among the workspaces.
    pub(crate) fn home_doc(&self) -> anyhow::Result<&'a Doc> {
        self.home
            .and_then(|home| self.workspaces.iter().find(|(c, _)| c.doc_id == home))
            .map(|(_, d)| *d)
            .ok_or_else(|| anyhow::anyhow!("the Home workspace is not open"))
    }
}

/// A workspace tag as the rows carry it: the doc id, and the doc's
/// name (`None` for Home and for an unnamed doc).
pub(crate) fn doc_tag(tag: Option<&Candidate>) -> (Option<String>, Option<String>) {
    (
        tag.map(|c| c.doc_id.0.to_string()),
        tag.and_then(|c| c.name.clone()),
    )
}

/// A failed call, as its caller outside the loop sees it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// The loop has ended: nothing is answered until a new host runs.
    #[error("the host has stopped")]
    Stopped,
    /// The workspace selector matched no open doc (or several).
    #[error("{message}")]
    NoSuchWorkspace { message: String },
    /// Everything else, with the message a command would have printed.
    #[error("{message}")]
    Failed { message: String },
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast::<ApiError>() {
            Ok(api) => api,
            Err(e) => ApiError::Failed {
                message: format!("{e:#}"),
            },
        }
    }
}
