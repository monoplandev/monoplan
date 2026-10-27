//! Monoplan FFI — the Rust↔Swift boundary for the Apple clients.
//!
//! What crosses today is the profile's lifecycle, not the doc: `login`
//! and `logout` (the bodies of the CLI commands of the same names) and
//! `Host`, the daemon loop embedded in the app's process
//! (`spec/daemon-plan.md`, amended 2026-10-27). The macOS app reads the
//! doc as a client of its own host's socket. In-process doc access for
//! the native clients — typed views, a mutation surface, a change
//! callback on `Host`, and a host that boots without serving a socket —
//! is the next layer and does not exist yet. The earlier offline
//! `MonoplanStore` prototype (`spec/swift-ffi-plan.md`) was removed on
//! 2026-10-27: it booted a second doc under a fake doc id with a
//! caller-held DEK, and nothing but its smoke test used it.
//!
//! Everything here is proc-macro / library mode uniffi (no `.udl`):
//! `setup_scaffolding!` plus `#[uniffi::export]` / `#[derive(uniffi::…)]`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

// ---------------------------------------------------------------------
// Host: the daemon loop, embedded (spec/daemon-plan.md, amended
// 2026-10-27). On macOS the app is the host whenever it runs: it runs
// `daemon::start` / `daemon::serve` on its own thread and its UI is a
// client of the resulting socket, exactly as the CLI is. The headless
// `monoplan daemon` stays for CLI-only, Linux and servers, and fills in
// on a Mac only while the app is not running: `start` takes a headless
// daemon over, and the CLI's auto-spawn starts one again if the app
// has quit.
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HostError {
    /// No `secrets.toml` in the profile dir: nothing to host.
    #[error("not logged in")]
    NotLoggedIn,
    /// Something `start` will not take over holds `daemon.lock`:
    /// another app's host, a host still starting, or a CLI command
    /// rewriting the profile (`client::claim`). Try again later.
    #[error("another process holds this profile")]
    Busy,
    #[error("{message}")]
    Failed { message: String },
}

/// Where the embedded host's `tracing` output goes: `daemon.log` in
/// the profile dir, the file the headless daemon's stderr is appended
/// to, so there is one place to look on a Mac. A GUI process has no
/// terminal, and the host's connect failures (`warn`) would otherwise
/// vanish. `RUST_LOG` overrides the level. Set once per process; the
/// file is reopened per write, so it survives a sign-out purge and
/// comes back with the next profile.
fn init_log(profile_dir: &std::path::Path) {
    use tracing_subscriber::{EnvFilter, fmt::writer::MakeWriter};

    struct Append(PathBuf);
    impl<'a> MakeWriter<'a> for Append {
        type Writer = Box<dyn std::io::Write + 'a>;
        fn make_writer(&'a self) -> Self::Writer {
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.0)
            {
                Ok(f) => Box::new(f),
                Err(_) => Box::new(std::io::sink()),
            }
        }
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(Append(profile_dir.join("daemon.log")))
        .try_init();
}

/// Sign in: what `monoplan login` does once it has the credentials,
/// for the app's sign-in sheet. Blocks for the key derivation and two
/// round trips, so call it off the main thread. On success the profile
/// is written and released; the app's host controller then hosts it.
#[uniffi::export]
pub fn login(
    profile_dir: String,
    server: String,
    email: String,
    password: String,
    device_name: String,
) -> Result<(), HostError> {
    use monoplan_cli::{
        commands::login::{Credentials, login},
        config::Profile,
    };
    let profile =
        Profile::create_at(PathBuf::from(profile_dir)).map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
    rt.block_on(login(
        &profile,
        Credentials {
            server,
            email,
            password,
            device_name,
        },
    ))
    .map(|_| ())
    .map_err(|e| HostError::Failed {
        message: format!("{e:#}"),
    })
}

/// Sign out: stop whatever hosts the profile, hold it, wipe it. What
/// `monoplan logout` does, for the app's Settings. The app stops its
/// own host first and keeps its host controller from restarting one
/// until this returns; `NotLoggedIn` when there was no profile.
#[uniffi::export]
pub fn logout(profile_dir: String) -> Result<(), HostError> {
    use monoplan_cli::{commands::logout, config::Profile};
    let profile = Profile::active_at(PathBuf::from(profile_dir)).ok_or(HostError::NotLoggedIn)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
    rt.block_on(logout::logout(&profile))
        .map_err(|e| HostError::Failed {
            message: format!("{e:#}"),
        })
}

/// One running host loop. Dropping it does *not* stop the loop; call
/// `shutdown` (clean exit: persist, fold, close the socket, release the
/// lock). A client's `Shutdown` request (`daemon stop`, `logout`,
/// `login`, `password`) ends the loop too, after which `is_running` is
/// false and the app starts a new one once the profile is free.
#[derive(uniffi::Object)]
pub struct Host {
    shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[uniffi::export]
impl Host {
    /// Claim the profile (stopping a headless daemon that holds it),
    /// bind the socket, boot the doc, and serve on a background thread.
    /// Returns once the socket is listening (or the start failed), so
    /// call it off the main thread: booting a large doc takes a moment.
    #[uniffi::constructor]
    pub fn start(profile_dir: String) -> Result<Arc<Host>, HostError> {
        use monoplan_cli::{client, config::Profile, daemon};
        let profile =
            Profile::active_at(PathBuf::from(profile_dir)).ok_or(HostError::NotLoggedIn)?;
        init_log(&profile.dir);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), HostError>>();
        let thread = std::thread::Builder::new()
            .name("monoplan-host".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(HostError::Failed {
                            message: e.to_string(),
                        }));
                        return;
                    }
                };
                rt.block_on(async move {
                    match client::take_over(&profile.dir).await {
                        Ok(true) => {}
                        Ok(false) => {
                            let _ = ready_tx.send(Err(HostError::Busy));
                            return;
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(HostError::Failed {
                                message: format!("{e:#}"),
                            }));
                            return;
                        }
                    }
                    let started = match daemon::start(profile).await {
                        Ok(Some(s)) => s,
                        // Lost the lock to someone between the takeover
                        // and the claim; the app tries again later.
                        Ok(None) => {
                            let _ = ready_tx.send(Err(HostError::Busy));
                            return;
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(HostError::Failed {
                                message: format!("{e:#}"),
                            }));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    if let Err(e) = daemon::serve(started, shutdown_rx, true).await {
                        eprintln!("monoplan host: {e:#}");
                    }
                });
            })
            .map_err(|e| HostError::Failed {
                message: e.to_string(),
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Arc::new(Host {
                shutdown: Mutex::new(Some(shutdown_tx)),
                thread: Mutex::new(Some(thread)),
            })),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(HostError::Failed {
                message: "host thread exited before reporting".into(),
            }),
        }
    }

    /// Clean exit; blocks until the loop has released the profile.
    pub fn shutdown(&self) {
        if let Some(tx) = self.shutdown.lock().unwrap().take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }

    /// Still serving? False once a client's `Shutdown` ended the loop.
    pub fn is_running(&self) -> bool {
        self.thread
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|t| !t.is_finished())
    }
}
