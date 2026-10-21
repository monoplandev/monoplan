use std::io::Write;

use clap::{Parser, Subcommand};
use dialoguer::Password;
use monoplan_core::Doc;
use serde::{Deserialize, Serialize};

use crate::client::{self, Spawn};
use crate::config::Profile;

pub const MIN_PASSWORD_LEN: usize = 10;

pub mod agenda;
pub mod bin;
pub mod cache;
pub mod export;
pub mod focus;
pub mod items;
pub mod lists;
mod login;
mod logout;
mod password;
mod recover;
mod share;
mod signup;
pub mod status;
mod sync;

const DEFAULT_SERVER: &str = "http://127.0.0.1:8000";

#[derive(Parser, Debug)]
#[command(name = "monoplan", version, about = "Monoplan CLI")]
pub struct Cli {
    /// Fail instead of starting the daemon when none is running
    /// (scripts, CI). Equivalent to `MONOPLAN_NO_SPAWN=1`.
    #[arg(long, global = true)]
    no_spawn: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create a new account on the given server.
    Signup(signup::Args),
    /// Log in to an existing account on a fresh device.
    Login(login::Args),
    /// Wipe local state. Does not revoke the device server-side.
    Logout,
    /// Use a recovery code to set a new password and bootstrap a fresh device.
    Recover(recover::Args),
    /// Change the password on the active account.
    Password,
    /// Add an item.
    Add(items::AddArgs),
    /// List items.
    Ls(items::LsArgs),
    /// Move an item to the Backlog state.
    Backlog(items::IdArg),
    /// Move an item to the Todo state.
    Todo(items::IdArg),
    /// Move an item to In Progress (stamps started_at on first entry).
    Start(items::IdArg),
    /// Move an item to the Review state.
    Review(items::IdArg),
    /// Mark an item done.
    Done(items::IdArg),
    /// Cancel an item (closed like Done, but not a completion).
    Cancel(items::IdArg),
    /// Send an item to the bin (or operate on the bin namespace).
    Bin(bin::BinArgs),
    /// Restore an item from the bin (reveals its preserved lifecycle).
    Restore(items::IdArg),
    /// Move an item to a different list.
    Mv(items::MvArgs),
    /// Edit an item's text.
    Edit(items::EditArgs),
    /// Set (YYYY-MM-DD or YYYY-MM-DDTHH:MM) or clear (-) an item's planned date.
    When(items::DateArg),
    /// Set (minutes or 1h30m) or clear (-) an item's duration; shown beside a timed planned date.
    Duration(items::DateArg),
    /// Set (YYYY-MM-DD) or clear (-) an item's deadline.
    Deadline(items::DateArg),
    /// Set (a label, optionally --at lat,lon and --address) or clear (-) an item's place.
    Place(items::PlaceArgs),
    /// Places already on items, newest use first; an optional query narrows them.
    Places(items::PlacesArgs),
    /// Open items by day: Today (with overdue and past-dated folded in), then the coming days.
    Agenda(agenda::AgendaArgs),
    /// Curated Focus lens: list (default), add, rm, mv.
    Focus(focus::FocusArgs),
    /// Manage lists.
    Lists(lists::ListsArgs),
    /// Inspect the local cache.
    Cache(cache::CacheArgs),
    /// Export the current account as semantic JSON.
    ExportJson(export::ExportArgs),
    /// Share a workspace: invite links, members.
    Share(share::ShareArgs),
    /// Show local sync state.
    Status(status::StatusArgs),
    /// Push pending local ops now and report whether the server is reachable.
    Sync,
    /// The resident sync process for this profile (spec/daemon-plan.md).
    Daemon(crate::daemon::DaemonArgs),
}

/// A command that runs against the doc. Parsed by the client, shipped
/// to the daemon over IPC (`crate::ipc`), executed by [`run_doc`]
/// against the resident doc; its output comes back as text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DocCmd {
    /// `texts` already expanded from the argument or stdin.
    Add {
        list: String,
        texts: Vec<String>,
    },
    Ls(items::LsArgs),
    Backlog(items::IdArg),
    Todo(items::IdArg),
    Start(items::IdArg),
    Review(items::IdArg),
    Done(items::IdArg),
    Cancel(items::IdArg),
    Bin(bin::BinArgs),
    Restore(items::IdArg),
    Mv(items::MvArgs),
    Edit(items::EditArgs),
    When(items::DateArg),
    Duration(items::DateArg),
    Deadline(items::DateArg),
    Place(items::PlaceArgs),
    Places(items::PlacesArgs),
    Agenda(agenda::AgendaArgs),
    Focus(focus::FocusArgs),
    Lists(lists::ListsArgs),
    ExportJson,
}

/// Execute a doc command. Pure over `doc` + `out`: no I/O of its own,
/// so the daemon can run it inline on its loop and the one-shot test
/// path can run it against any `Doc`.
pub fn run_doc(cmd: DocCmd, doc: &Doc, out: &mut dyn Write) -> anyhow::Result<()> {
    match cmd {
        DocCmd::Add { list, texts } => items::add(doc, &list, &texts, out),
        DocCmd::Ls(a) => items::ls(doc, a, out),
        DocCmd::Backlog(a) => items::backlog(doc, a, out),
        DocCmd::Todo(a) => items::todo(doc, a, out),
        DocCmd::Start(a) => items::start(doc, a, out),
        DocCmd::Review(a) => items::review(doc, a, out),
        DocCmd::Done(a) => items::done(doc, a, out),
        DocCmd::Cancel(a) => items::cancel(doc, a, out),
        DocCmd::Bin(a) => bin::run(doc, a, out),
        DocCmd::Restore(a) => items::restore(doc, a, out),
        DocCmd::Mv(a) => items::mv(doc, a, out),
        DocCmd::Edit(a) => items::edit(doc, a, out),
        DocCmd::When(a) => items::when(doc, a, out),
        DocCmd::Duration(a) => items::duration(doc, a, out),
        DocCmd::Deadline(a) => items::deadline(doc, a, out),
        DocCmd::Place(a) => items::place(doc, a, out),
        DocCmd::Places(a) => items::places(doc, a, out),
        DocCmd::Agenda(a) => agenda::run(doc, a, out),
        DocCmd::Focus(a) => focus::run(doc, a, out),
        DocCmd::Lists(a) => lists::run(doc, a, out),
        DocCmd::ExportJson => {
            out.write_all(export::render(doc)?.as_bytes())?;
            Ok(())
        }
    }
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        let spawn = if self.no_spawn || env_flag("MONOPLAN_NO_SPAWN") {
            Spawn::Never
        } else {
            Spawn::OnDemand
        };
        let doc_cmd = match self.cmd {
            Cmd::Signup(a) => return signup::run(a).await,
            Cmd::Login(a) => return login::run(a, spawn).await,
            Cmd::Logout => return logout::run().await,
            Cmd::Recover(a) => return recover::run(a, spawn).await,
            Cmd::Password => return password::run().await,
            Cmd::Cache(a) => return cache::run(a).await,
            Cmd::Share(a) => return share::run(a).await,
            Cmd::Status(a) => return status::run(a).await,
            Cmd::Daemon(a) => return crate::daemon::run(a).await,
            Cmd::Sync => return sync::run(spawn).await,
            Cmd::ExportJson(a) => {
                let profile = Profile::require_active()?;
                let (json, sync) = client::doc(&profile, DocCmd::ExportJson, spawn).await?;
                export::deliver(json.trim_end_matches('\n'), a.out.as_deref())?;
                offline_notice(sync);
                return Ok(());
            }
            Cmd::Add(a) => DocCmd::Add {
                texts: items::collect_texts(&a.text)?,
                list: a.list,
            },
            Cmd::Ls(a) => DocCmd::Ls(a),
            Cmd::Backlog(a) => DocCmd::Backlog(a),
            Cmd::Todo(a) => DocCmd::Todo(a),
            Cmd::Start(a) => DocCmd::Start(a),
            Cmd::Review(a) => DocCmd::Review(a),
            Cmd::Done(a) => DocCmd::Done(a),
            Cmd::Cancel(a) => DocCmd::Cancel(a),
            Cmd::Bin(a) => DocCmd::Bin(a),
            Cmd::Restore(a) => DocCmd::Restore(a),
            Cmd::Mv(a) => DocCmd::Mv(a),
            Cmd::Edit(a) => DocCmd::Edit(a),
            Cmd::When(a) => DocCmd::When(a),
            Cmd::Duration(a) => DocCmd::Duration(a),
            Cmd::Deadline(a) => DocCmd::Deadline(a),
            Cmd::Place(a) => DocCmd::Place(a),
            Cmd::Places(a) => DocCmd::Places(a),
            Cmd::Agenda(a) => DocCmd::Agenda(a),
            Cmd::Focus(a) => DocCmd::Focus(a),
            Cmd::Lists(a) => DocCmd::Lists(a),
        };
        let profile = Profile::require_active()?;
        let (stdout, sync) = client::doc(&profile, doc_cmd, spawn).await?;
        print!("{stdout}");
        offline_notice(sync);
        Ok(())
    }
}

/// The reply was served from the local doc with no server behind it:
/// say so once, on stderr, where it stays out of parsed output.
fn offline_notice(sync: crate::ipc::SyncStatus) {
    if let Some(line) = sync.offline_notice(crate::sync::now_millis()) {
        eprintln!("{line}");
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

pub fn default_device_name() -> String {
    gethostname::gethostname()
        .into_string()
        .unwrap_or_else(|_| "monoplan-cli".to_string())
}

pub fn default_server() -> String {
    std::env::var("MONOPLAN_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_string())
}

/// Prompt for a new password with confirmation and the minimum-length rule.
pub fn prompt_new_password(prompt: &str) -> anyhow::Result<String> {
    let pw = Password::new()
        .with_prompt(format!("{prompt} ({MIN_PASSWORD_LEN}+ characters)"))
        .with_confirmation("Confirm password", "Passwords don't match")
        .validate_with(|input: &String| -> Result<(), String> {
            if input.chars().count() < MIN_PASSWORD_LEN {
                Err(format!(
                    "password must be at least {MIN_PASSWORD_LEN} characters"
                ))
            } else {
                Ok(())
            }
        })
        .interact()?;
    Ok(pw)
}
