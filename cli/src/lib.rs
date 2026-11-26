//! Internals shared between the `monoplan` binary and integration tests.

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

/// `eprintln!` that cannot panic: for anything the engine can reach.
/// `eprintln!` panics when the write fails, and stderr can be dead:
/// an iOS app launched from Xcode has a pty to the debugger as its
/// stderr, and once the debugger is gone every write is EIO. The panic
/// ended the engine loop on each connect, and the app restarted it
/// into the same panic (2026-11-22).
#[macro_export]
macro_rules! note {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stderr(), $($arg)*);
    }};
}

pub mod api;
pub mod bootstrap;
pub mod client;
pub mod commands;
pub mod config;
pub mod daemon;
pub mod files;
pub mod ipc;
#[cfg(target_vendor = "apple")]
pub mod keychain;
pub mod keystore;
pub mod net;
pub mod service;
pub mod storage;
pub mod sync;
pub mod undo;
pub mod workspace;
