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
    /// Exit cleanly after replying.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Reply {
    Hello {
        build_id: String,
        pid: u32,
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
    ShuttingDown,
    Err {
        message: String,
    },
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
    }
}
