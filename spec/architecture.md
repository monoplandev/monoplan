# Architecture

## Crate layout

```
Cargo.toml         workspace root
crates/
  protocol/        shared wire types (serde; JSON on HTTP, rmp-serde on WS)
core/              rust lib — loro, e2ee, sync engine. native + wasm targets.
core/web/          wasm-bindgen surface (excluded from default-members)
server/            rust bin — http + ws, sqlite-backed, dumb relay
cli/               rust bin — depends on core, integration test surface
```

Single workspace. Always invoke wasm builds as `wasm-pack build core/` (never bare `cargo build --target wasm32-...` at root, which would try to build server/cli for wasm and choke).

**Split plan:** when `crates/protocol/` reaches a stable enough shape, extract `core/` + `crates/protocol/` into a separate FOSS repo; `server/` consumes via git submodule + path dep. Defer until then — the workspace's atomic cross-crate refactor benefits dominate during high-churn early development.

## Build targets

- `core` builds for: native (linux/mac/windows) and `wasm32-unknown-unknown`.
- WASM build via `wasm-pack`; web bindings live in `core/web/`.
- `server` and `cli` are native-only.

## Cross-platform client boundary

Monoplan's sync protocol state machine lives in shared Rust, not reimplemented per client. The shared `core::sync::SyncEngine` is **sans-IO**:

- it owns protocol state, doc application, encryption framing, and push/pull/ack sequencing
- it does **not** own the socket, timers, reconnect policy, auth transport, or debounce policy
- callers feed transport events in (`handle_connected`, inbound frame bytes, disconnects, timeouts) and drain outbound frame bytes / engine events out

This keeps Loro + crypto + protocol behavior identical across clients while letting each host platform own transport policy:

- CLI: native Rust + `tokio-tungstenite`
- Web: wasm-bindgen surface over the same engine, browser `WebSocket` owned by JS
- Future native clients: the same `core` crate exposed over UniFFI to Swift / Kotlin transport shells

Two boundary rules matter:

- **No push pipelining.** The engine serializes pushes: at most one durable in-flight push at a time, identified by its `push_id`. If local mutations happen while a push is in flight, the engine marks itself dirty and derives the next delta (from `server_known_vv`) only after the server ack arrives. This avoids duplicate export windows and keeps host adapters simple.
- **Auth stays outside the engine.** The engine starts from "socket is authenticated and usable". Whether that came from a bearer header, cookie-backed browser session, or future ticket exchange is a client / transport concern documented in `auth.md`.

Platform policy stays outside `core`:

- reconnect/backoff
- online/offline and visibility hooks
- mutation flush debounce
- worker placement for expensive client-side KDF work

## Local persistence

Persistence is **inside** the engine's contract, via one Rust trait — `core::LocalStorage` (`core/src/storage.rs`). Crash recovery and outbound sync are separate concerns (`spec/vv-wal-separation.md`): the engine appends an encrypted WAL row on each local commit (`capture_local_ops`) and each applied remote op, folds the WAL into a full-history snapshot when it crosses the host's threshold (regardless of sync state), and derives what to push from Loro history against `server_known_vv` — never from which WAL rows exist. It rebuilds the doc from snapshot + replay on boot. Storage is mandatory: there is no storage-less engine mode.

Two implementations satisfy the same semantics on different substrates:

- **CLI / server-side single-account flows:** `SqliteStorage` (`crates/storage-sqlite`, `rusqlite`, file on disk; synchronously durable).
- **Web:** `IdbStorage` (`js/core/src/storage/idb-storage.ts`) behind a wasm-bindgen `EngineStorage` extern (`core/web/src/lib.rs`). The trait is synchronous; IDB is async, so `IdbStorage` keeps a synchronous in-memory mirror the engine reads/writes immediately and flushes IDB in the background, signalling real durability back via `notify_oplog_durable`. The engine stays on the main thread (no Worker).

The rationale and history — including why web uses IDB rather than sqlite-wasm — live in `spec/local-storage.md`.

## Server-is-dumb thesis

The server **cannot**:
- read op contents (opaque encrypted blobs)
- run a Loro doc
- validate op semantics
- compute snapshots

The server **can**:
- authenticate accounts and devices
- assign per-doc monotonic, gap-free `seq`
- track per-(device, doc) `last_acked_seq`
- compute horizon = min(`last_acked_seq`) across all non-revoked devices that can open the doc
- decide when a snapshot is due (op-count threshold past last snapshot, triggering device caught up)
- ask any caught-up connected client to produce the snapshot (state frontier = `server_last_seq`, compaction floor = `max(horizon, prev snapshot's compaction_floor_seq)`)
- replace prior snapshots and prune updates up to the snapshot's `compaction_floor_seq`

This thesis is load-bearing for everything in `sync-protocol.md`.

### Direct device-to-device sync (out of scope, not foreclosed)

Not in this product's scope; it would be a separate product. Recorded
here (2026-10-17) so the door stays open. Every device is a full
replica, and the core already speaks Loro's native VV-to-VV sync
(`export_updates_since`), so a direct transport is two messages: your
VV, my VV, missing updates each way. Everything on that wire is sealed
under the DEK, so peer auth falls out of E2EE. The server-shaped part of
the client is confined to the engine's seq bookkeeping; a direct
transport would feed the same `Doc` and leave `server_known_vv` alone, so
reconnect behaves as today (pull, then push everything the server lacks;
duplicate rows from both sides pushing LAN-received updates are harmless and
GC'd). What it would need that does not exist: LAN discovery and first
pairing. What it would not give: durability; the server is also the
off-site copy. Trim applies the same rule on every transport (a trimmed
device accepts only ops above its root). Keep the seq; it is the cheapest
coordination there is, and the direct mode is additive to it.

## Wire encoding

Two encodings, chosen per transport:

- **HTTP bodies are JSON** (`Content-Type: application/json`), via `serde_json` / `axum::Json`. Byte fields (salts, nonces, wrapped keys; a few dozen bytes each) travel as standard padded base64 strings, marked `#[serde(with = "serde_b64")]` on the protocol types and mirrored by a field-name list in the web client. The REST surface is key management and account admin: small payloads, read by humans in a browser Network tab or `curl | jq` far more often than by anything that cares about bytes on the wire. (Switched from MessagePack 2026-10-29; the "no base64" argument never applied here because encrypted blobs do not cross HTTP.)
- **WebSocket frames are MessagePack** (`rmp-serde`), binary. Encrypted update and snapshot blobs ride this path, so native `bin` support matters, and the frame rate makes a compact encoding worth the opacity. Byte fields are `serde_bytes`.

Both use the same `#[derive(Serialize, Deserialize)]` types with string-keyed maps, so additive compat is free on either path (add a field → old decoders ignore it; remove a field → use `Option`). Malformed HTTP bodies (bad JSON, missing field, invalid base64) return the standard `400 bad_request` error envelope. The admin API (`admin.md`) was JSON from the start.

Not built: a `monoplan decode <file>` debug subcommand that pretty-prints a captured WS frame as JSON (and, given the DEK, decrypts the blobs inside).
