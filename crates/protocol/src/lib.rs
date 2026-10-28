//! Wire types shared across `monoplan-server`, `monoplan-core`, and `monoplan`.
//!
//! Two encodings, by transport (`spec/architecture.md` §Wire encoding):
//! `auth` types are JSON on HTTP, with byte fields as base64 strings
//! (`auth::serde_b64`); `sync` types are MessagePack (`rmp-serde`) on the
//! WebSocket, with byte fields as `serde_bytes` so the encrypted blobs
//! travel as native `bin` rather than a sequence of u8s.

pub mod auth;
pub mod sync;

pub use auth::*;
pub use sync::*;
