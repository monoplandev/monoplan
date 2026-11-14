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
//! side (`Item`, not its `ItemRow`).
//!
//! Functions return `anyhow::Result`, as the commands they came out of
//! do; [`ApiError`] is what a caller outside the loop gets. A failure a
//! client has to branch on is raised as an `ApiError` inside the
//! `anyhow::Error` and comes back out as itself.

use monoplan_core::Doc;

use crate::commands::focus::Lens;

pub mod items;
pub mod lists;

/// What a call runs against: the workspace it named, every open doc
/// (that one among them) for the account-scoped reads, and Starred as
/// seen from it. The same three things `commands::run_doc` is given.
pub struct Ctx<'a> {
    pub doc: &'a Doc,
    pub open_docs: &'a [&'a Doc],
    pub lens: &'a Lens<'a>,
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
