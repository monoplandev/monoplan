//! Internals shared between the `monoplan` binary and integration tests.

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub mod api;
pub mod bootstrap;
pub mod client;
pub mod commands;
pub mod config;
pub mod daemon;
pub mod ipc;
pub mod keystore;
pub mod net;
pub mod peer;
pub mod service;
pub mod storage;
pub mod sync;
pub mod workspace;
