//! Thin-client side of `spec/daemon-plan.md`: connect to the profile's
//! daemon, starting it on demand, and run one request.
//!
//! Every invocation opens a fresh connection (a CLI process runs one
//! command). The hello exchange pins the build: a daemon from an older
//! build is asked to shut down and a fresh one is started from this
//! executable, so a half-upgraded install can never split-brain.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tokio::net::UnixStream;

use crate::commands::DocCmd;
use crate::config::Profile;
use crate::daemon;
use crate::ipc::{self, Reply, Request, SyncStatus, read_frame, write_frame};

/// Whether a missing daemon is started or reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spawn {
    OnDemand,
    Never,
}

/// How long to wait for a daemon that holds the lock but is not yet
/// answering (starting up, or wedged) before giving up.
const WAIT_FOR_SOCKET: Duration = Duration::from_secs(5);
/// How long `stop` waits for the lock to release.
const WAIT_FOR_STOP: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

/// Run a doc command and return what it printed, with the daemon's
/// staleness so the caller can say when the answer is offline.
pub async fn doc(
    profile: &Profile,
    cmd: DocCmd,
    spawn: Spawn,
) -> anyhow::Result<(String, SyncStatus)> {
    match call(profile, Request::Doc(cmd), spawn).await? {
        Reply::Output { stdout, sync } => Ok((stdout, sync)),
        Reply::Err { message } => Err(anyhow::anyhow!(message)),
        other => anyhow::bail!("unexpected daemon reply: {other:?}"),
    }
}

/// Push now; `Err` when the server is unreachable.
pub async fn sync(profile: &Profile, spawn: Spawn) -> anyhow::Result<()> {
    match call(profile, Request::Sync, spawn).await? {
        Reply::Synced { sync } if sync.online => Ok(()),
        Reply::Synced { .. } => anyhow::bail!("server unreachable"),
        Reply::Err { message } => Err(anyhow::anyhow!(message)),
        other => anyhow::bail!("unexpected daemon reply: {other:?}"),
    }
}

/// One request, one reply, on a fresh connection.
pub async fn call(profile: &Profile, req: Request, spawn: Spawn) -> anyhow::Result<Reply> {
    let mut stream = connect(profile, spawn).await?;
    write_frame(&mut stream, &req).await?;
    read_frame(&mut stream)
        .await?
        .ok_or_else(|| anyhow::anyhow!("daemon closed the connection without replying"))
}

/// Ask a running daemon to exit and wait until the profile is free.
/// `Ok(false)` when none was running.
pub async fn stop_if_running(profile_dir: &Path) -> anyhow::Result<bool> {
    if !daemon::is_running(profile_dir)? {
        return Ok(false);
    }
    let path = ipc::socket_path(profile_dir);
    if let Ok(mut stream) = UnixStream::connect(&path).await {
        let _ = write_frame(
            &mut stream,
            &Request::Hello {
                build_id: ipc::build_id(),
            },
        )
        .await;
        let _ = read_frame::<_, Reply>(&mut stream).await;
        let _ = write_frame(&mut stream, &Request::Shutdown).await;
        let _ = read_frame::<_, Reply>(&mut stream).await;
    }
    wait_until_stopped(profile_dir).await?;
    Ok(true)
}

async fn wait_until_stopped(profile_dir: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + WAIT_FOR_STOP;
    while daemon::is_running(profile_dir)? {
        if Instant::now() > deadline {
            anyhow::bail!("daemon did not stop within {WAIT_FOR_STOP:?}");
        }
        tokio::time::sleep(POLL).await;
    }
    Ok(())
}

async fn connect(profile: &Profile, spawn: Spawn) -> anyhow::Result<UnixStream> {
    let path = ipc::socket_path(&profile.dir);
    let mut spawned = false;
    let mut waiting_since: Option<Instant> = None;
    loop {
        match UnixStream::connect(&path).await {
            Ok(mut stream) => {
                let mine = ipc::build_id();
                write_frame(
                    &mut stream,
                    &Request::Hello {
                        build_id: mine.clone(),
                    },
                )
                .await?;
                match read_frame::<_, Reply>(&mut stream).await? {
                    Some(Reply::Hello { build_id, .. }) if build_id == mine => return Ok(stream),
                    Some(Reply::Hello { build_id, pid, .. }) => {
                        tracing::info!(
                            "daemon pid {pid} is build {build_id}, this is {mine}; restarting it"
                        );
                        write_frame(&mut stream, &Request::Shutdown).await?;
                        let _ = read_frame::<_, Reply>(&mut stream).await;
                        drop(stream);
                        wait_until_stopped(&profile.dir).await?;
                        // Fall through: nothing holds the lock now, so the
                        // next iteration spawns from this executable.
                        spawned = false;
                        continue;
                    }
                    other => anyhow::bail!("unexpected hello reply: {other:?}"),
                }
            }
            Err(_) if daemon::is_running(&profile.dir)? => {
                // Lock held, socket not answering: a daemon is starting
                // (or wedged). Wait, bounded.
                let since = *waiting_since.get_or_insert_with(Instant::now);
                if since.elapsed() > WAIT_FOR_SOCKET {
                    anyhow::bail!(
                        "a daemon holds this profile but is not answering on {}; \
                         try `monoplan daemon stop`",
                        path.display()
                    );
                }
                tokio::time::sleep(POLL).await;
            }
            Err(e) => {
                if spawned {
                    anyhow::bail!("started the daemon but cannot connect to it: {e}");
                }
                if spawn == Spawn::Never {
                    anyhow::bail!(
                        "monoplan daemon is not running (start it with `monoplan daemon`)"
                    );
                }
                spawn_daemon(profile)?;
                spawned = true;
                waiting_since = None;
            }
        }
    }
}

/// Start `monoplan daemon --detach` from this executable and wait for
/// its ready line. The child's stdout is the readiness pipe (one line:
/// `ready`, `already-running`, or `error: …`); its stderr goes to
/// `daemon.log` in the profile dir.
fn spawn_daemon(profile: &Profile) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(profile.dir.join("daemon.log"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon")
        .arg("--detach")
        .env("MONOPLAN_DATA_DIR", &profile.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and the closure does
        // nothing else; it runs in the forked child before exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("piped");
    // The readiness line is the first *recognised* line: anything else
    // on stdout before it (a stray log line from a dependency) is
    // skipped rather than mistaken for a failure. Bounded, so a child
    // that chatters forever still fails the start.
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    for _ in 0..64 {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let t = line.trim();
        if t == "ready" || t == "already-running" || t.starts_with("error: ") {
            break;
        }
        line.clear();
    }
    // The child is detached; never wait on it. Dropping `child` does
    // not kill it.
    match line.trim() {
        "ready" => {
            eprintln!(
                "started monoplan daemon (pid {}); run `monoplan daemon install` to start it at login",
                child.id()
            );
            Ok(())
        }
        "already-running" => Ok(()),
        "" => anyhow::bail!("daemon exited before signalling readiness; see daemon.log"),
        other => anyhow::bail!(
            "daemon failed to start: {}",
            other.trim_start_matches("error: ")
        ),
    }
}
