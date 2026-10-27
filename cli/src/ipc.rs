//! Client ↔ daemon protocol (`spec/daemon-plan.md` "IPC protocol").
//!
//! A Unix domain socket in the profile dir carrying length-prefixed
//! MessagePack frames: `u32` big-endian payload length, then the
//! `rmp-serde` encoding of [`Request`] / [`Reply`]. The protocol is
//! the command surface, not raw ops: the daemon owns the doc, clients
//! send a parsed command and get back what the one-shot CLI would have
//! printed. Auth is the socket file's mode plus a peer-uid check on
//! accept.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::commands::DocCmd;

/// Upper bound on one frame; a doc export at the perf limits is a few
/// MB, so this is a sanity cap against a garbage length prefix, not a
/// budget.
pub const MAX_FRAME: u32 = 64 * 1024 * 1024;

const SOCKET_FILE: &str = "daemon.sock";
/// `sun_path` is 104 bytes on macOS (108 on Linux) including the NUL;
/// stay under the smaller one with margin for the fallback decision.
const MAX_SOCKET_PATH: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// First frame on every connection. The daemon answers with its own
    /// build id; a mismatch means the client restarts it.
    Hello { build_id: String },
    /// Run a doc command; the reply carries its rendered output.
    Doc(DocCmd),
    /// Push now and report whether the server is reachable.
    Sync,
    /// Who is logged in: the account row the daemon booted with. Read
    /// here rather than from the profile files so a client that holds
    /// no db (the menubar app) still gets the email.
    Account,
    /// Turn this connection into a watcher: the reply is `Watching`,
    /// and from then on the daemon also sends `Event` frames on it
    /// whenever the doc or the sync state changes. Ordinary requests
    /// still work on the same connection; a client with one request in
    /// flight at a time can tell replies and events apart by variant.
    /// This is the projection tier of `spec/daemon-plan.md`: a watcher
    /// re-requests what it shows, it holds no doc of its own.
    Watch,
    /// Exit cleanly after replying.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Reply {
    Hello {
        build_id: String,
        pid: u32,
        /// The host loop runs inside a GUI process (the macOS app)
        /// rather than a headless `monoplan daemon`. Clients never
        /// restart an embedded host for staleness (`client::connect`).
        embedded: bool,
        sync: SyncStatus,
    },
    /// The command ran; `stdout` is what to print, verbatim.
    Output {
        stdout: String,
        sync: SyncStatus,
    },
    Synced {
        sync: SyncStatus,
    },
    Account {
        account_id: String,
        email: String,
        device_id: String,
    },
    /// `Watch` accepted; events follow on this connection.
    Watching {
        sync: SyncStatus,
    },
    /// Unsolicited, only on a watching connection.
    Event(WatchEvent),
    ShuttingDown,
    Err {
        message: String,
    },
}

/// What a watcher is told. Deliberately payload-free for doc changes:
/// the watcher fetches the projection it shows (one request), which
/// keeps the protocol view-agnostic and coalesces bursts for free. A
/// watcher that falls behind the daemon's event buffer is sent one
/// `Changed` rather than the backlog, since that is all it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchEvent {
    /// The doc changed: a local command committed, or remote ops
    /// applied. Re-fetch.
    Changed,
    /// Online/offline flipped or `last_synced_at` moved.
    Sync(SyncStatus),
}

/// Staleness, on every successful reply. A daemon that lost its
/// WebSocket keeps serving the local doc, which is right, but the
/// one-shot `-s` used to fail loudly where the daemon succeeds quietly;
/// this is how a client tells the difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    /// Socket up and the doc open on it right now.
    pub online: bool,
    /// Unix millis of the last completed pull/push/ack exchange, on
    /// any run of the daemon (it is persisted beside the sync cursor).
    pub last_synced_at: Option<i64>,
}

impl SyncStatus {
    /// One stderr line for a human when the reply is stale; `None`
    /// while online.
    pub fn offline_notice(&self, now_millis: i64) -> Option<String> {
        if self.online {
            return None;
        }
        Some(match self.last_synced_at {
            None => "offline; never synced".to_string(),
            Some(at) => format!("offline; last synced {} ago", ago(now_millis - at)),
        })
    }
}

fn ago(millis: i64) -> String {
    let secs = (millis / 1000).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// Identity of the running code, so a client never talks to a daemon
/// built from different sources. Version plus the executable's own
/// mtime: any rebuild changes the latter, which is what matters in
/// development where the version string does not move.
/// The package-version half of a build id: what has to agree between a
/// CLI and an embedded host, where the executables legitimately differ.
pub fn version_of(build_id: &str) -> &str {
    build_id.split(':').next().unwrap_or(build_id)
}

pub fn build_id() -> String {
    let mtime = std::env::current_exe()
        .and_then(|p| p.metadata())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}:{mtime}", env!("CARGO_PKG_VERSION"))
}

/// Where the profile's daemon listens. Beside the db when the path
/// fits in `sun_path`; otherwise a hash-named socket in the temp dir,
/// computed the same way by both sides so nothing has to be recorded.
pub fn socket_path(profile_dir: &Path) -> PathBuf {
    let direct = profile_dir.join(SOCKET_FILE);
    if direct.as_os_str().len() <= MAX_SOCKET_PATH {
        return direct;
    }
    let hash = fnv1a(profile_dir.as_os_str().as_encoded_bytes());
    std::env::temp_dir().join(format!("monoplan-{hash:016x}.sock"))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("ipc: {0}")]
    Io(#[from] std::io::Error),
    #[error("ipc: frame of {0} bytes exceeds the limit")]
    TooLarge(u32),
    #[error("ipc: encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("ipc: decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("ipc: peer closed")]
    Closed,
}

pub async fn write_frame<W, T>(w: &mut W, value: &T) -> Result<(), FrameError>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let payload = rmp_serde::to_vec_named(value)?;
    let len = u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge(u32::MAX))?;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&payload).await?;
    w.flush().await?;
    Ok(())
}

/// `Ok(None)` on a clean close before any length byte; a close mid-frame
/// is an error.
pub async fn read_frame<R, T>(r: &mut R) -> Result<Option<T>, FrameError>
where
    R: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => FrameError::Closed,
            _ => FrameError::Io(e),
        })?;
    Ok(Some(rmp_serde::from_slice(&payload)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip_and_eof_is_clean() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        write_frame(&mut a, &Request::Sync).await.unwrap();
        let sync = SyncStatus {
            online: true,
            last_synced_at: Some(5),
        };
        write_frame(&mut a, &Reply::Synced { sync }).await.unwrap();
        drop(a);
        let req: Request = read_frame(&mut b).await.unwrap().unwrap();
        assert!(matches!(req, Request::Sync));
        let reply: Reply = read_frame(&mut b).await.unwrap().unwrap();
        assert!(matches!(
            reply,
            Reply::Synced {
                sync: SyncStatus {
                    online: true,
                    last_synced_at: Some(5)
                }
            }
        ));
        assert!(read_frame::<_, Reply>(&mut b).await.unwrap().is_none());
    }

    /// The wire shape foreign clients (the Swift menubar app) encode by
    /// hand: `rmp_serde::to_vec_named`, so struct fields are string-keyed
    /// maps, a unit variant is a bare string, and any other variant is a
    /// one-entry map from the variant name to its payload. Pinned here
    /// so a serde attribute change cannot silently strand them.
    #[test]
    fn wire_shape_is_named_maps_and_bare_unit_variants() {
        use crate::commands::{DocCmd, agenda::AgendaArgs};
        fn shape<T: Serialize>(v: &T) -> serde_json::Value {
            rmp_serde::from_slice(&rmp_serde::to_vec_named(v).unwrap()).unwrap()
        }
        assert_eq!(
            rmp_serde::to_vec_named(&Request::Watch).unwrap(),
            b"\xa5Watch"
        );
        assert_eq!(
            rmp_serde::to_vec_named(&Request::Account).unwrap(),
            b"\xa7Account"
        );
        assert_eq!(
            shape(&Reply::Account {
                account_id: "a".into(),
                email: "e".into(),
                device_id: "d".into(),
            }),
            serde_json::json!({"Account": {"account_id": "a", "email": "e", "device_id": "d"}})
        );
        assert_eq!(
            rmp_serde::to_vec_named(&Request::Hello {
                build_id: "x".into()
            })
            .unwrap(),
            b"\x81\xa5Hello\x81\xa8build_id\xa1x"
        );
        assert_eq!(
            shape(&Request::Doc(DocCmd::Agenda(AgendaArgs {
                days: 0,
                today: None,
                json: true,
            }))),
            serde_json::json!({"Doc": {"Agenda": {"days": 0, "today": null, "json": true}}})
        );
        assert_eq!(
            shape(&Request::Doc(DocCmd::Focus(
                crate::commands::focus::FocusArgs {
                    cmd: None,
                    json: true,
                }
            ))),
            serde_json::json!({"Doc": {"Focus": {"cmd": null, "json": true}}})
        );
        assert_eq!(
            shape(&Request::Doc(DocCmd::Lists(
                crate::commands::lists::ListsArgs {
                    cmd: crate::commands::lists::ListsCmd::Ls {
                        json: true,
                        archived: false,
                        all: true,
                    },
                }
            ))),
            serde_json::json!({"Doc": {"Lists": {"cmd": {"Ls": {"json": true, "archived": false, "all": true}}}}})
        );
        let sync = SyncStatus {
            online: false,
            last_synced_at: Some(7),
        };
        assert_eq!(
            shape(&Reply::Watching { sync }),
            serde_json::json!({"Watching": {"sync": {"online": false, "last_synced_at": 7}}})
        );
        assert_eq!(
            shape(&Reply::Event(WatchEvent::Changed)),
            serde_json::json!({"Event": "Changed"})
        );
        assert_eq!(
            shape(&Reply::Event(WatchEvent::Sync(sync))),
            serde_json::json!({"Event": {"Sync": {"online": false, "last_synced_at": 7}}})
        );
        assert_eq!(
            shape(&Reply::Output {
                stdout: "[]\n".into(),
                sync
            }),
            serde_json::json!({"Output": {"stdout": "[]\n", "sync": {"online": false, "last_synced_at": 7}}})
        );
    }

    #[test]
    fn offline_notice_reads_like_a_human_wrote_it() {
        let online = SyncStatus {
            online: true,
            last_synced_at: None,
        };
        assert!(online.offline_notice(0).is_none());
        let never = SyncStatus {
            online: false,
            last_synced_at: None,
        };
        assert_eq!(never.offline_notice(0).unwrap(), "offline; never synced");
        let stale = SyncStatus {
            online: false,
            last_synced_at: Some(1_000),
        };
        assert_eq!(
            stale.offline_notice(1_000 + 3 * 60_000).unwrap(),
            "offline; last synced 3m ago"
        );
        assert_eq!(
            stale.offline_notice(1_000 + 26 * 3_600_000).unwrap(),
            "offline; last synced 1d ago"
        );
    }

    #[test]
    fn long_profile_paths_fall_back_to_temp() {
        let short = Path::new("/tmp/p");
        assert_eq!(socket_path(short), Path::new("/tmp/p/daemon.sock"));
        let long = PathBuf::from(format!("/{}", "x".repeat(150)));
        let p = socket_path(&long);
        assert!(p.starts_with(std::env::temp_dir()));
        assert_eq!(p, socket_path(&long), "deterministic");
        // Pinned so a foreign client (the Swift menubar app) computing
        // the same fallback name can test against the same vector.
        assert_eq!(fnv1a(b"/tmp/p"), 0x6cc12addf2745004);
    }
}
