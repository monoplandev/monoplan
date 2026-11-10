//! The thin-client path of `spec/daemon-plan.md`, driven through the
//! real `monoplan` executable: a command with no daemon running starts
//! one, later commands reuse it, a second device sees its writes, and
//! `daemon stop` releases the profile.

mod support;

use std::path::Path;
use std::process::Output;

use tokio::process::Command;

use monoplan_cli::config::{Config, Secrets};
use monoplan_cli::daemon;
use monoplan_cli::sync::Engine;
use support::{
    TEST_PASSWORD, TestServer, create_doc, login_device, materialize_profile, signup_account,
    wait_for_updates,
};
use uuid::Uuid;

/// Async so the in-process test server keeps being served while the
/// CLI (and the daemon it starts) talk to it.
async fn monoplan(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_monoplan"))
        .args(args)
        .env("MONOPLAN_DATA_DIR", data_dir)
        .env_remove("MONOPLAN_NO_SPAWN")
        .output()
        .await
        .expect("spawn monoplan")
}

fn ok(out: Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn err(out: Output) -> String {
    assert!(
        !out.status.success(),
        "expected failure, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8(out.stderr).unwrap()
}

#[tokio::test]
async fn commands_start_and_share_one_daemon() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "bin-a").await;
    let tmp = tempfile::tempdir().unwrap();
    materialize_profile(
        tmp.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &signup.device_id,
        &signup.device_token,
        &signup.dek,
        &signup.email,
        true,
    )
    .await;
    let dir = tmp.path();

    // Nothing running: --no-spawn is an error, and no daemon appears.
    let e = err(monoplan(dir, &["--no-spawn", "item", "ls"]).await);
    assert!(e.contains("not running"), "{e}");
    assert!(!daemon::is_running(dir).unwrap());
    assert_eq!(
        ok(monoplan(dir, &["daemon", "status"]).await).trim(),
        "not running"
    );

    // First real command starts the daemon and prints the new id.
    let out = monoplan(dir, &["item", "add", "hello from the cli"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let id = ok(out).trim().to_string();
    assert_eq!(id.len(), 32, "uuid v7 hex: {id}");
    assert!(stderr.contains("started monoplan daemon"), "{stderr}");
    assert!(daemon::is_running(dir).unwrap());

    // Later commands reuse it (no spawn line) and see the write.
    let out = monoplan(dir, &["item", "ls"]).await;
    assert!(!String::from_utf8_lossy(&out.stderr).contains("started"));
    let ls = ok(out);
    assert!(
        ls.contains(&id) && ls.contains("hello from the cli"),
        "{ls}"
    );
    let status = ok(monoplan(dir, &["daemon", "status"]).await);
    assert!(status.starts_with("running  pid "), "{status}");

    // Errors from the doc come back as a non-zero exit with the message.
    let e = err(monoplan(dir, &["item", "set", "nope", "--lifecycle", "done"]).await);
    assert!(!e.trim().is_empty());

    // Wait for the daemon's push to land on the server; a second device
    // then sees the item via a one-shot. (Not an in-process IPC call:
    // this test binary is a different executable from the daemon's, so
    // the build-id handshake would restart the daemon.)
    let primary_doc_id = Uuid::parse_str(&signup.primary_doc_id).unwrap();
    wait_for_updates(&server, primary_doc_id, 1).await;
    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "bin-b")
        .await
        .unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &login.account_id,
        &login.primary_doc_id,
        &login.device_id,
        &login.device_token,
        &login.dek,
        &signup.email,
        false,
    )
    .await;
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    assert!(engine_b.is_online());
    assert_eq!(
        engine_b.doc().get_item(&id).unwrap().text,
        "hello from the cli"
    );
    engine_b.flush().await.unwrap();

    // Online replies carry no offline line.
    let out = monoplan(dir, &["item", "ls"]).await;
    assert!(!String::from_utf8_lossy(&out.stderr).contains("offline"));

    // Stop releases the profile; the socket file is gone.
    assert_eq!(
        ok(monoplan(dir, &["daemon", "stop"]).await).trim(),
        "stopped"
    );
    assert!(!daemon::is_running(dir).unwrap());
    assert!(!monoplan_cli::ipc::socket_path(dir).exists());
    assert_eq!(
        ok(monoplan(dir, &["daemon", "stop"]).await).trim(),
        "not running"
    );
}

/// A daemon whose server is unreachable still serves the local doc, but
/// every reply says so.
#[tokio::test]
async fn offline_daemon_serves_locally_and_says_so() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "bin-offline").await;
    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_profile(
        tmp.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &signup.device_id,
        &signup.device_token,
        &signup.dek,
        &signup.email,
        true,
    )
    .await;
    // Point the profile at a port nothing listens on before any daemon
    // exists for it.
    profile
        .write_config(&Config {
            server_url: "http://127.0.0.1:1".into(),
        })
        .unwrap();
    let dir = tmp.path();

    let out = monoplan(dir, &["item", "add", "captured offline"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let id = ok(out).trim().to_string();
    assert_eq!(id.len(), 32);
    // The notice may lag the very first reply by one failed connect
    // attempt; `ls` right after is deterministic.
    let _ = stderr;
    let out = monoplan(dir, &["item", "ls"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = ok(out);
    assert!(stdout.contains("captured offline"), "{stdout}");
    assert!(stderr.contains("offline; never synced"), "{stderr}");

    let status = ok(monoplan(dir, &["daemon", "status"]).await);
    assert!(status.contains("offline; never synced"), "{status}");

    assert_eq!(
        ok(monoplan(dir, &["daemon", "stop"]).await).trim(),
        "stopped"
    );
    assert!(!daemon::is_running(dir).unwrap());
}

/// A fresh install needs no account (`cli/src/bootstrap.rs`): the
/// first command mints a local-only workspace, replies never call it
/// offline, and the verbs that need a server say what is missing.
#[tokio::test]
async fn fresh_install_works_without_an_account() {
    let tmp = tempfile::tempdir().unwrap();
    // Not even the directory exists yet.
    let dir = tmp.path().join("fresh");

    let out = monoplan(&dir, &["item", "add", "no account yet"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let id = ok(out).trim().to_string();
    assert_eq!(id.len(), 32, "uuid v7 hex: {id}");
    assert!(stderr.contains("created a local workspace"), "{stderr}");
    assert!(stderr.contains("started monoplan daemon"), "{stderr}");
    assert!(!stderr.contains("offline"), "{stderr}");
    assert!(daemon::is_running(&dir).unwrap());

    // Served from the local doc, seeded like a signup would be, and
    // never reported as offline: there is nothing to be offline from.
    let out = monoplan(&dir, &["item", "ls"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let ls = ok(out);
    assert!(ls.contains("no account yet"), "{ls}");
    assert!(ls.contains("Welcome to Monoplan"), "{ls}");
    assert!(!stderr.contains("offline"), "{stderr}");
    // The maintenance sweep goes over the same IPC and finds a fresh
    // doc clean; a second run is equally a no-op.
    let swept = ok(monoplan(&dir, &["workspace", "reconcile"]).await);
    assert_eq!(swept.trim(), "Nothing to repair.", "{swept}");
    let swept = ok(monoplan(&dir, &["workspace", "reconcile"]).await);
    assert_eq!(swept.trim(), "Nothing to repair.", "{swept}");

    let status = ok(monoplan(&dir, &["daemon", "status"]).await);
    assert!(status.contains("local only"), "{status}");
    let status = ok(monoplan(&dir, &["auth", "status"]).await);
    assert!(status.contains("local-only"), "{status}");
    let json = ok(monoplan(&dir, &["auth", "status", "--json"]).await);
    let compact: String = json.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("\"local_only\":true"), "{json}");
    assert!(compact.contains("\"email\":null"), "{json}");
    let status = ok(monoplan(&dir, &["workspace", "status"]).await);
    assert!(status.contains("local-only"), "{status}");
    assert!(status.contains("Doc:"), "{status}");
    let json = ok(monoplan(&dir, &["workspace", "status", "--json"]).await);
    let compact: String = json.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("\"local_only\":true"), "{json}");
    assert!(compact.contains("\"last_sync_at\":null"), "{json}");

    // Server-backed verbs name the gap instead of failing obscurely.
    let e = err(monoplan(&dir, &["-w", "Work", "workspace", "share", "pending"]).await);
    assert!(e.contains("no account"), "{e}");
    // One workspace per accountless install: any other `-w` names
    // what there is.
    let e = err(monoplan(&dir, &["-w", "Work", "item", "ls"]).await);
    assert!(e.contains("this install has: Personal"), "{e}");
    let ls = ok(monoplan(&dir, &["-w", "personal", "item", "ls"]).await);
    assert!(ls.contains("no account yet"), "{ls}");
    let e = err(monoplan(&dir, &["auth", "password"]).await);
    assert!(e.contains("no account"), "{e}");

    // Logging out of nothing keeps the workspace and its host.
    let out = ok(monoplan(&dir, &["auth", "logout"]).await);
    assert!(out.contains("local workspace is kept"), "{out}");
    assert!(daemon::is_running(&dir).unwrap());
    let ls = ok(monoplan(&dir, &["item", "ls"]).await);
    assert!(ls.contains("no account yet"), "{ls}");

    assert_eq!(
        ok(monoplan(&dir, &["daemon", "stop"]).await).trim(),
        "stopped"
    );
}

/// `monoplan workspace create <name>` (`spec/cli.md` §Workspaces): the
/// running daemon registers a doc with the server, seeds and names it,
/// opens it on its live socket so the seed is pushed, and `-w <name>`
/// selects it from the next command on. A second device that logs in
/// afterwards receives the membership and the name. `-w` is refused,
/// and so is a blank name.
#[tokio::test]
async fn workspace_create_opens_a_new_doc_on_the_running_daemon() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "bin-create").await;
    let tmp = tempfile::tempdir().unwrap();
    materialize_profile(
        tmp.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &signup.device_id,
        &signup.device_token,
        &signup.dek,
        &signup.email,
        true,
    )
    .await;
    let dir = tmp.path();

    // Start the daemon with an ordinary command, then create.
    ok(monoplan(dir, &["item", "ls"]).await);
    let out = ok(monoplan(dir, &["workspace", "create", "  Work "]).await);
    assert!(out.starts_with("Created workspace Work ("), "{out}");
    let work_id = out
        .trim()
        .trim_start_matches("Created workspace Work (")
        .trim_end_matches(").")
        .to_string();
    let work_uuid = Uuid::parse_str(&work_id).expect("a doc id in the output");

    // The daemon opened it without a restart: it is selectable by name
    // and by id, and Home is untouched.
    let work_item = ok(monoplan(dir, &["-w", "Work", "item", "add", "in work"]).await)
        .trim()
        .to_string();
    let ls = ok(monoplan(dir, &["-w", &work_id, "item", "ls"]).await);
    assert!(ls.contains(&work_item), "{ls}");
    let ls = ok(monoplan(dir, &["item", "ls"]).await);
    assert!(!ls.contains(&work_item), "{ls}");
    let e = err(monoplan(dir, &["-w", "Play", "item", "ls"]).await);
    assert!(e.contains("this install has: Personal, Work"), "{e}");

    // The seed (name included) and the item reached the server's doc.
    wait_for_updates(&server, work_uuid, 1).await;

    // Refusals.
    let e = err(monoplan(dir, &["-w", "Work", "workspace", "create", "Play"]).await);
    assert!(e.contains("does not take --workspace"), "{e}");
    let e = err(monoplan(dir, &["workspace", "create", "   "]).await);
    assert!(e.contains("needs a name"), "{e}");

    // A second device logging in afterwards gets the membership with
    // its key, and reads the name out of the synced doc.
    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "bin-create-b")
        .await
        .unwrap();
    let work_dek = login
        .doc_keys
        .iter()
        .find(|(id, _)| id.0 == work_uuid)
        .map(|(_, dek)| dek.clone())
        .expect("the created doc is a membership, unwrapped with the Home DEK");
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &login.account_id,
        &login.primary_doc_id,
        &login.device_id,
        &login.device_token,
        &login.dek,
        &signup.email,
        false,
    )
    .await;
    let home_doc_id = monoplan_core::DocId(Uuid::parse_str(&signup.primary_doc_id).unwrap());
    profile_b
        .write_secrets(&Secrets::new(
            Some(login.device_token.clone()),
            [
                (home_doc_id, &login.dek),
                (monoplan_core::DocId(work_uuid), &work_dek),
            ],
        ))
        .unwrap();
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    assert!(engine_b.is_online());
    let work_b = engine_b
        .doc_for(monoplan_core::DocId(work_uuid))
        .expect("the second device opened the created doc");
    assert_eq!(work_b.get_settings().doc_name.as_deref(), Some("Work"));
    assert_eq!(work_b.get_item(&work_item).unwrap().text, "in work");
    engine_b.flush().await.unwrap();

    // Delete: refused without `-w` and on Personal, then gone from the
    // running daemon (no restart) and from the server, so a fresh
    // login holds the Home key alone.
    let e = err(monoplan(dir, &["workspace", "delete", "--yes"]).await);
    assert!(e.contains("name the workspace to delete"), "{e}");
    let e = err(monoplan(dir, &["-w", "personal", "workspace", "delete", "--yes"]).await);
    assert!(e.contains("cannot be deleted"), "{e}");
    let out = ok(monoplan(dir, &["-w", "Work", "workspace", "delete", "--yes"]).await);
    assert_eq!(out.trim(), format!("Deleted workspace {work_id}."));
    let e = err(monoplan(dir, &["-w", "Work", "item", "ls"]).await);
    assert!(
        e.contains("this install has: Personal\n")
            || e.trim_end().ends_with("this install has: Personal"),
        "{e}"
    );
    let ls = ok(monoplan(dir, &["item", "ls"]).await);
    assert!(!ls.contains(&work_item), "{ls}");
    let login_c = login_device(&server.base, &signup.email, TEST_PASSWORD, "bin-create-c")
        .await
        .unwrap();
    assert_eq!(login_c.doc_keys.len(), 1, "only Home is left");

    ok(monoplan(dir, &["daemon", "stop"]).await);
}

/// `-w <workspace>` (`spec/cli.md` §Workspaces): a second doc the
/// account owns is selected by name or id, an omitted `-w` stays on
/// Home, the daemon pushes each doc's ops to the right server doc, and
/// a second device with no local rows for the shared doc is filled
/// from the server on first open.
#[tokio::test]
async fn workspace_flag_selects_a_second_doc() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "bin-w").await;
    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_profile(
        tmp.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &signup.device_id,
        &signup.device_token,
        &signup.dek,
        &signup.email,
        true,
    )
    .await;
    let dir = tmp.path();

    // A second doc, created the way the web client does, named "Work"
    // in its own settings and seeded locally as a membership login
    // would leave it.
    let (work_id, work_dek) = create_doc(&server.base, &signup.device_token, &signup.dek)
        .await
        .unwrap();
    let work_doc_id = monoplan_core::DocId(Uuid::parse_str(&work_id).unwrap());
    let home_doc_id = monoplan_core::DocId(Uuid::parse_str(&signup.primary_doc_id).unwrap());
    let storage = monoplan_cli::storage::open_storage(&profile).unwrap();
    let work = monoplan_core::Doc::new().unwrap();
    work.set_doc_name(Some("Work")).unwrap();
    monoplan_cli::storage::seed_snapshot(&storage, &work_dek, work_doc_id, &work).unwrap();
    profile
        .write_secrets(&Secrets::new(
            Some(signup.device_token.clone()),
            [(home_doc_id, &signup.dek), (work_doc_id, &work_dek)],
        ))
        .unwrap();

    // Name, case-insensitive; id; alias for Home; unknown lists what
    // there is.
    let work_item = ok(monoplan(dir, &["-w", "Work", "item", "add", "in work"]).await)
        .trim()
        .to_string();
    let home_item = ok(monoplan(dir, &["item", "add", "in home"]).await)
        .trim()
        .to_string();
    let ls = ok(monoplan(dir, &["-w", "work", "item", "ls"]).await);
    assert!(ls.contains(&work_item) && !ls.contains(&home_item), "{ls}");
    let ls = ok(monoplan(dir, &["-w", &work_id, "item", "ls"]).await);
    assert!(ls.contains(&work_item), "{ls}");
    let ls = ok(monoplan(dir, &["-w", "home", "item", "ls"]).await);
    assert!(ls.contains(&home_item) && !ls.contains(&work_item), "{ls}");
    let e = err(monoplan(dir, &["-w", "Play", "item", "ls"]).await);
    assert!(e.contains("this install has: Personal, Work"), "{e}");
    let e = err(monoplan(dir, &["-w", "Work", "auth", "status"]).await);
    assert!(e.contains("does not take --workspace"), "{e}");

    // `workspace status` resolves without the daemon and names the doc.
    let json = ok(monoplan(dir, &["-w", "Work", "workspace", "status", "--json"]).await);
    assert!(json.contains(&work_id), "{json}");
    let json = ok(monoplan(dir, &["workspace", "status", "--json"]).await);
    assert!(json.contains(&signup.primary_doc_id), "{json}");

    // The export is per workspace too.
    let export = ok(monoplan(dir, &["-w", "Work", "workspace", "export-json"]).await);
    assert!(
        export.contains("in work") && !export.contains("in home"),
        "{export}"
    );

    // `agenda --all` is the one merged read: both docs' rows in one
    // placement, tagged; `-w` with it is refused.
    ok(monoplan(
        dir,
        &[
            "-w",
            "Work",
            "item",
            "set",
            &work_item,
            "--when",
            "2030-01-02T09:00",
        ],
    )
    .await);
    ok(monoplan(dir, &["item", "set", &home_item, "--when", "2030-01-02"]).await);
    let json = ok(monoplan(
        dir,
        &[
            "agenda",
            "--all",
            "--json",
            "--today",
            "2030-01-01",
            "--days",
            "3",
        ],
    )
    .await);
    let days: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = &days[1]["rows"];
    assert_eq!(rows.as_array().map(Vec::len), Some(2), "{json}");
    // All-day Home row ahead of the timed Work one.
    assert_eq!(rows[0]["id"], home_item, "{json}");
    assert_eq!(rows[0]["doc_id"], signup.primary_doc_id, "{json}");
    assert!(rows[0].get("doc_name").is_none(), "{json}");
    assert_eq!(rows[1]["id"], work_item, "{json}");
    assert_eq!(rows[1]["doc_id"], work_id, "{json}");
    assert_eq!(rows[1]["doc_name"], "Work", "{json}");
    let text = ok(monoplan(
        dir,
        &["agenda", "--all", "--today", "2030-01-01", "--days", "3"],
    )
    .await);
    assert!(
        text.contains("in work @2030-01-02T09:00+1h  [Work]")
            && text.contains("in home @2030-01-02  [Personal]"),
        "{text}"
    );
    let single = ok(monoplan(
        dir,
        &["agenda", "--json", "--today", "2030-01-01", "--days", "3"],
    )
    .await);
    assert!(
        single.contains(&home_item) && !single.contains(&work_item) && !single.contains("doc_id"),
        "{single}"
    );
    let e = err(monoplan(dir, &["-w", "Work", "agenda", "--all"]).await);
    assert!(e.contains("takes no -w"), "{e}");

    // `starred --all` is Home's lens with its tags (foreign refs have
    // no CLI verb yet; their resolution is unit-tested in `focus.rs`).
    ok(monoplan(dir, &["item", "set", &home_item, "--star"]).await);
    let json = ok(monoplan(dir, &["starred", "--all", "--json"]).await);
    let rows: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(rows.as_array().map(Vec::len), Some(1), "{json}");
    assert_eq!(rows[0]["id"], home_item, "{json}");
    assert_eq!(rows[0]["doc_id"], signup.primary_doc_id, "{json}");
    assert_eq!(rows[0]["list_name"], "Inbox", "{json}");
    assert!(rows[0].get("doc_name").is_none(), "{json}");
    let text = ok(monoplan(dir, &["starred", "--all"]).await);
    assert!(text.contains("in home  [Personal]"), "{text}");
    let plain = ok(monoplan(dir, &["starred", "--json"]).await);
    assert!(
        plain.contains(&home_item) && !plain.contains("doc_id"),
        "{plain}"
    );
    let e = err(monoplan(dir, &["-w", "Work", "starred", "--all"]).await);
    assert!(e.contains("takes no -w"), "{e}");
    let e = err(monoplan(dir, &["starred", "--all", "mv", &home_item, "1"]).await);
    assert!(e.contains("only lists"), "{e}");

    // Each doc's ops reached its own server doc.
    wait_for_updates(&server, Uuid::parse_str(&work_id).unwrap(), 1).await;
    wait_for_updates(&server, Uuid::parse_str(&signup.primary_doc_id).unwrap(), 1).await;

    // A second device logs in (both keys) and has no local rows for
    // Work: its engine boots the doc empty and the server fills it.
    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "bin-w-b")
        .await
        .unwrap();
    assert_eq!(login.doc_keys.len(), 2, "both memberships come back");
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &login.account_id,
        &login.primary_doc_id,
        &login.device_id,
        &login.device_token,
        &login.dek,
        &signup.email,
        false,
    )
    .await;
    profile_b
        .write_secrets(&Secrets::new(
            Some(login.device_token.clone()),
            login.doc_keys.iter().map(|(id, dek)| (*id, dek)),
        ))
        .unwrap();
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    assert!(engine_b.is_online());
    assert_eq!(engine_b.doc().get_item(&home_item).unwrap().text, "in home");
    let work_b = engine_b.doc_for(work_doc_id).expect("work doc open");
    assert_eq!(work_b.get_item(&work_item).unwrap().text, "in work");
    assert_eq!(work_b.get_settings().doc_name.as_deref(), Some("Work"));
    assert!(engine_b.doc().get_item(&work_item).is_none());
    assert_eq!(engine_b.resolve(Some("work")).unwrap(), work_doc_id);
    engine_b.flush().await.unwrap();

    assert_eq!(
        ok(monoplan(dir, &["daemon", "stop"]).await).trim(),
        "stopped"
    );
}
