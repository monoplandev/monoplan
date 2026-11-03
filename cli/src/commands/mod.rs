use std::io::Write;

use clap::{Parser, Subcommand};
use dialoguer::{Confirm, Password};
use monoplan_core::Doc;
use serde::{Deserialize, Serialize};

use crate::client::{self, Spawn};
use crate::config::Profile;

pub const MIN_PASSWORD_LEN: usize = 10;

pub mod agenda;
pub mod bin;
pub mod db;
pub mod export;
pub mod focus;
pub mod items;
pub mod lists;
pub mod login;
pub mod logout;
mod password;
mod recover;
mod share;
pub mod signup;
pub mod status;

/// Where `signup` / `login` / `recover` go without `--server`: the
/// hosted alpha, the same default as the app's sign-in sheet. The dev
/// stack is `MONOPLAN_SERVER=http://127.0.0.1:8000`.
const DEFAULT_SERVER: &str = "https://alpha.monoplan.app";

/// `monoplan --version`: the crate version plus the commit it was built
/// from, "+" when the tree was dirty (`cli/build.rs`), matching the
/// app's Settings Build row.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("MONOPLAN_BUILD_GIT_SHA"),
    ")"
);

#[derive(Parser, Debug)]
#[command(name = "monoplan", version = VERSION, about = "Monoplan CLI")]
pub struct Cli {
    /// Fail instead of starting the daemon when none is running
    /// (scripts, CI). Equivalent to `MONOPLAN_NO_SPAWN=1`.
    #[arg(long)]
    no_spawn: bool,

    /// The workspace to act in: its name or doc id. Omitted means the
    /// Personal workspace (`personal` and `home` name it too). Root-only,
    /// like `--no-spawn`: it comes before the verb (`monoplan -w Work item
    /// ls`), so `auth` / `db` / `daemon` help never lists it.
    #[arg(short = 'w', long = "workspace", value_name = "WORKSPACE")]
    workspace: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Items: add, ls, set (every field as a flag), bin, restore.
    #[command(subcommand)]
    Item(ItemCmd),
    /// Manage lists: ls, add, rename, archive, unarchive. `lists` is a hidden alias.
    #[command(alias = "lists")]
    List(lists::ListsArgs),
    /// Starred items in curated order; `starred mv` reorders, `item set --star` / `--unstar` add and remove.
    Starred(focus::FocusArgs),
    /// Open items by day: Today (with overdue and past-dated folded in), then the coming days.
    Agenda(agenda::AgendaArgs),
    /// The bin: show, empty, rm.
    Bin(bin::BinArgs),
    /// Manage account and login.
    #[command(subcommand)]
    Auth(AuthCmd),
    /// Workspaces: sync status, sharing (invite links, members), JSON export.
    #[command(subcommand)]
    Workspace(WorkspaceCmd),
    /// Inspect the local database.
    Db(db::DbArgs),
    /// The resident sync process for this profile.
    Daemon(crate::daemon::DaemonArgs),
}

/// The account verbs, grouped under `monoplan auth` so the top level
/// reads as the task tool it is: these are typed once per device, the
/// item verbs daily. None of them touch the doc, so none go over IPC.
#[derive(Subcommand, Debug)]
enum AuthCmd {
    /// Create a new account on the given server; the local workspace becomes its Personal workspace.
    Signup(signup::Args),
    /// Log in to an existing account on a fresh device (a local-only workspace is discarded).
    Login(login::Args),
    /// Wipe local state. Does not revoke the device server-side.
    Logout,
    /// Use a recovery code to set a new password and bootstrap a fresh device.
    Recover(recover::Args),
    /// Change the password on the active account.
    Password,
    /// Show the account, device and server behind this install (local; never connects).
    Status(status::StatusArgs),
}

/// `monoplan workspace …`: the verbs that act on a workspace (doc) as a
/// whole rather than on its items. An accountless install has exactly
/// one workspace (`spec/auth.md` §One workspace per accountless
/// install): `status` and `export-json` work on it, `share` needs an
/// account.
#[derive(Subcommand, Debug)]
enum WorkspaceCmd {
    /// Show the workspace's sync state: doc id, last sync, acked seq, pending changes (local; never connects).
    Status(status::StatusArgs),
    /// Share a workspace: invite links, members.
    Share(share::ShareArgs),
    /// Export the workspace as semantic JSON.
    ExportJson(export::ExportArgs),
    /// Repair order and focus entries, and sweep the entries of items closed longer than --keep-days. Idempotent; changes nothing visible.
    Reconcile(ReconcileArgs),
}

#[derive(clap::Args, Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileArgs {
    /// Keep the order entries of items done, cancelled or binned within this many days; older ones are swept (a swept item that reopens lands at the end of its list).
    #[arg(long, default_value_t = 30, conflicts_with = "no_sweep")]
    pub keep_days: u32,
    /// Structural repair only: never sweep, whatever the age.
    #[arg(long)]
    pub no_sweep: bool,
}

impl ReconcileArgs {
    /// The sweep cutoff in unix millis, or `None` for `--no-sweep`.
    fn sweep_closed_before(&self, now_millis: i64) -> Option<i64> {
        (!self.no_sweep).then(|| now_millis - i64::from(self.keep_days) * 86_400_000)
    }
}

/// `monoplan item …`: everything about one item except capturing it.
#[derive(Subcommand, Debug)]
enum ItemCmd {
    /// Add an item.
    Add(items::AddArgs),
    /// List items.
    Ls(items::LsArgs),
    /// Set one or more fields on an item: --text, --list, --lifecycle, --when, --duration, --deadline, --place, --star, --unstar.
    Set(Box<items::SetArgs>),
    /// Send an item to the bin (its workflow state is preserved).
    Bin(items::IdArg),
    /// Restore an item from the bin (reveals its preserved workflow state).
    Restore(items::IdArg),
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
    Set(Box<items::SetArgs>),
    ItemBin(items::IdArg),
    Restore(items::IdArg),
    /// The bin namespace (`bin show` / `empty` / `rm`).
    Bin(bin::BinArgs),
    Agenda(agenda::AgendaArgs),
    Focus(focus::FocusArgs),
    Lists(lists::ListsArgs),
    ExportJson,
    /// `Doc::reconcile_with`: explicit maintenance sweep, never
    /// scheduled. `sweep_closed_before` is resolved in the client so the
    /// daemon's clock never enters into it.
    Reconcile {
        sweep_closed_before: Option<i64>,
    },
}

/// Execute a doc command. Pure over `doc` + `out`: no I/O of its own,
/// so the daemon can run it inline on its loop and the one-shot test
/// path can run it against any `Doc`.
pub fn run_doc(cmd: DocCmd, doc: &Doc, out: &mut dyn Write) -> anyhow::Result<()> {
    match cmd {
        DocCmd::Add { list, texts } => items::add(doc, &list, &texts, out),
        DocCmd::Ls(a) => items::ls(doc, a, out),
        DocCmd::Set(a) => items::set(doc, *a, out),
        DocCmd::ItemBin(a) => items::bin(doc, a, out),
        DocCmd::Restore(a) => items::restore(doc, a, out),
        DocCmd::Bin(a) => bin::run(doc, a, out),
        DocCmd::Agenda(a) => agenda::run(doc, a, out),
        DocCmd::Focus(a) => focus::run(doc, a, out),
        DocCmd::Lists(a) => lists::run(doc, a, out),
        DocCmd::ExportJson => {
            out.write_all(export::render(doc)?.as_bytes())?;
            Ok(())
        }
        DocCmd::Reconcile {
            sweep_closed_before,
        } => {
            let repairs = doc.reconcile_with(sweep_closed_before)?;
            if repairs == 0 {
                writeln!(out, "Nothing to repair.")?;
            } else {
                writeln!(out, "Repaired {repairs} entries.")?;
            }
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
        let workspace = self.workspace;
        // Only doc commands act in a workspace; the rest refuse the
        // flag rather than silently ignore it.
        let no_workspace = |verb: &str| -> anyhow::Result<()> {
            if workspace.is_some() {
                anyhow::bail!("`{verb}` does not take --workspace");
            }
            Ok(())
        };
        let doc_cmd = match self.cmd {
            Cmd::Auth(a) => {
                no_workspace("auth")?;
                return match a {
                    AuthCmd::Signup(a) => signup::run(a, spawn).await,
                    AuthCmd::Login(a) => login::run(a, spawn).await,
                    AuthCmd::Logout => logout::run().await,
                    AuthCmd::Recover(a) => recover::run(a, spawn).await,
                    AuthCmd::Password => password::run().await,
                    AuthCmd::Status(a) => status::run_auth(a).await,
                };
            }
            Cmd::Db(a) => {
                no_workspace("db")?;
                return db::run(a).await;
            }
            Cmd::Daemon(a) => {
                no_workspace("daemon")?;
                return crate::daemon::run(a).await;
            }
            Cmd::Workspace(WorkspaceCmd::Share(a)) => return share::run(a, workspace).await,
            Cmd::Workspace(WorkspaceCmd::Status(a)) => {
                return status::run_workspace(a, workspace).await;
            }
            Cmd::Workspace(WorkspaceCmd::ExportJson(a)) => {
                let profile = crate::bootstrap::open_or_init().await?;
                let (json, sync) =
                    client::doc(&profile, workspace, DocCmd::ExportJson, spawn).await?;
                export::deliver(json.trim_end_matches('\n'), a.out.as_deref())?;
                offline_notice(sync);
                return Ok(());
            }
            Cmd::Workspace(WorkspaceCmd::Reconcile(a)) => DocCmd::Reconcile {
                sweep_closed_before: a.sweep_closed_before(crate::sync::now_millis()),
            },
            Cmd::Item(ItemCmd::Add(a)) => DocCmd::Add {
                texts: items::collect_texts(&a.text)?,
                list: a.list,
            },
            Cmd::Item(ItemCmd::Ls(a)) => DocCmd::Ls(a),
            Cmd::Item(ItemCmd::Set(a)) => DocCmd::Set(a),
            Cmd::Item(ItemCmd::Bin(a)) => DocCmd::ItemBin(a),
            Cmd::Item(ItemCmd::Restore(a)) => DocCmd::Restore(a),
            Cmd::Bin(a) => DocCmd::Bin(a),
            Cmd::Agenda(a) => DocCmd::Agenda(a),
            Cmd::Starred(a) => DocCmd::Focus(a),
            Cmd::List(a) => DocCmd::Lists(a),
        };
        // A fresh install gets a local-only workspace here; no account
        // needed to capture (`crate::bootstrap`).
        let profile = crate::bootstrap::open_or_init().await?;
        let (stdout, sync) = client::doc(&profile, workspace, doc_cmd, spawn).await?;
        print!("{stdout}");
        offline_notice(sync);
        Ok(())
    }
}

/// `login` / `recover` from an install that holds a local-only
/// workspace: the account's doc replaces it (the web client does the
/// same; merging two seeded histories is not built), so say so and ask.
/// Nothing to say when there is no profile or it already has an account.
pub(crate) fn confirm_discard_local_workspace(profile: &Profile) -> anyhow::Result<()> {
    if crate::bootstrap::local_only_doc(profile)?.is_none() {
        return Ok(());
    }
    println!(
        "This install has a local-only workspace. Signing in to an existing account \
         discards it; `monoplan auth signup` would keep it instead, and \
         `monoplan workspace export-json` saves a copy first."
    );
    let proceed = Confirm::new()
        .with_prompt("Discard the local workspace and sign in?")
        .default(false)
        .interact()?;
    if !proceed {
        anyhow::bail!("aborted");
    }
    Ok(())
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

/// The `kind` this client registers under (`DeviceRegistration::kind`):
/// the CLI is the desktop app's front door, so it reports the host
/// platform rather than "cli".
pub fn device_kind() -> String {
    std::env::consts::OS.to_string()
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
