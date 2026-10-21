//! The thin-client path of `spec/daemon-plan.md`, driven through the
//! real `monoplan` executable: a command with no daemon running starts
//! one, later commands reuse it, a second device sees its writes, and
//! `daemon stop` releases the profile.

mod support;

use std::path::Path;
use std::process::Output;

use tokio::process::Command;

use monoplan_cli::config::Config;
use monoplan_cli::daemon;
use monoplan_cli::sync::Session;
use support::{TEST_PASSWORD, TestServer, login_device, materialize_profile, signup_account};

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
    let signup = signup_account(&server, "bin-a", false).await;
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
    let e = err(monoplan(dir, &["--no-spawn", "ls"]).await);
    assert!(e.contains("not running"), "{e}");
    assert!(!daemon::is_running(dir).unwrap());
    assert_eq!(
        ok(monoplan(dir, &["daemon", "status"]).await).trim(),
        "not running"
    );

    // First real command starts the daemon and prints the new id.
    let out = monoplan(dir, &["add", "hello from the cli"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let id = ok(out).trim().to_string();
    assert_eq!(id.len(), 32, "uuid v7 hex: {id}");
    assert!(stderr.contains("started monoplan daemon"), "{stderr}");
    assert!(daemon::is_running(dir).unwrap());

    // Later commands reuse it (no spawn line) and see the write.
    let out = monoplan(dir, &["ls"]).await;
    assert!(!String::from_utf8_lossy(&out.stderr).contains("started"));
    let ls = ok(out);
    assert!(
        ls.contains(&id) && ls.contains("hello from the cli"),
        "{ls}"
    );
    let status = ok(monoplan(dir, &["daemon", "status"]).await);
    assert!(status.starts_with("running  pid "), "{status}");

    // Errors from the doc come back as a non-zero exit with the message.
    let e = err(monoplan(dir, &["done", "nope"]).await);
    assert!(!e.trim().is_empty());

    // The daemon pushed; a second device sees the item via a one-shot.
    ok(monoplan(dir, &["sync"]).await);
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
    let session_b = Session::open_with_profile(profile_b, true).await.unwrap();
    assert!(session_b.is_online());
    assert_eq!(
        session_b.doc().get_item(&id).unwrap().text,
        "hello from the cli"
    );
    session_b.flush().await.unwrap();

    // Online replies carry no offline line.
    let out = monoplan(dir, &["ls"]).await;
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
/// every reply says so, and `sync` fails instead of pretending.
#[tokio::test]
async fn offline_daemon_serves_locally_and_says_so() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "bin-offline", false).await;
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

    let out = monoplan(dir, &["add", "captured offline"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let id = ok(out).trim().to_string();
    assert_eq!(id.len(), 32);
    // The notice may lag the very first reply by one failed connect
    // attempt; `ls` right after is deterministic.
    let _ = stderr;
    let out = monoplan(dir, &["ls"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = ok(out);
    assert!(stdout.contains("captured offline"), "{stdout}");
    assert!(stderr.contains("offline; never synced"), "{stderr}");

    let e = err(monoplan(dir, &["sync"]).await);
    assert!(e.contains("could not reach server"), "{e}");
    let status = ok(monoplan(dir, &["daemon", "status"]).await);
    assert!(status.contains("offline; never synced"), "{status}");

    assert_eq!(
        ok(monoplan(dir, &["daemon", "stop"]).await).trim(),
        "stopped"
    );
    assert!(!daemon::is_running(dir).unwrap());
}
