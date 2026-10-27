# Swift FFI prototype — Layers 1 & 2 (Rust FFI crate + SwiftPM package)

Status: built, then superseded. Milestones 0 and 2 to 4 stand: the
`crates/storage-sqlite` hoist, `apple/build-xcframework.sh`, the
`MonoplanCore` SwiftPM package and `apple/README.md`. Milestone 1's
`MonoplanStore` object (plus its `ItemView` / `ListView` records,
`MonoplanError` and `generate_dek`) was removed on 2026-10-27: it booted
a second doc under a fake doc id with a caller-held DEK and never saw the
account's doc or sync, and nothing but its smoke test used it. The FFI
crate now exports the profile lifecycle only (`login`, `logout`, `Host`;
`spec/daemon-plan.md`, amendment 2026-10-27). In-process doc access for
the native clients is the next plan, and it hangs off `Host`, not a
store. The rest of this file is kept as the record of the build pipeline.

Goal: prove the Rust↔Swift FFI boundary on macOS with an offline
capture/read round-trip, packaged so an Xcode app (layer 3, out of scope
here) can consume it as a local SwiftPM dependency.

**Non-goals for this plan:** no sync/websocket, no auth/HTTP, no UI, no
Keychain, no iOS app target. The `SyncEngine` is *not* exposed yet — the
FFI surface is offline doc + storage only. Sync plumbing is a follow-up
once the boundary is proven.

Read first: `spec/architecture.md`, `spec/data-model.md`,
`spec/local-storage.md`, `spec/encryption.md`. Reference implementations:
`core/web/src/` (how the wasm bindings wrap `Doc`/storage — mirror its
shape), `cli/src/storage.rs` + `cli/src/db.rs` (the native sqlite
storage this plan hoists).

## Constraints

- **Don't break existing consumers.** `bun run test`, `bun run build`,
  and `bun run build:wasm` must all stay green. `core/` must stay
  wasm-clean — no rusqlite or uniffi dependency lands in `monoplan-core`
  itself.
- **Migrations rule (CLAUDE.md):** exactly one migration file per
  database, edited in place. No incremental migrations.
- **Bundled sqlite:** the workspace `rusqlite` dep already uses
  `features = ["bundled"]` — keep that for the FFI build (one known
  sqlite version across CLI/server/app; don't link Apple's system
  libsqlite3).
- Match existing code style: doc comments explaining *why*, thiserror
  enums, the `&self` + interior-mutability pattern `LocalStorage`
  already documents.

## Milestone 0 — hoist sqlite storage out of the CLI

New workspace crate `crates/storage-sqlite` (package name
`monoplan-storage-sqlite`), added to `[workspace] members` but **not**
`default-members`.

Move from `cli/`:

- `cli/src/db.rs` — `open()`, pragmas, migration ledger.
- The `SqliteStorage` struct + its `impl LocalStorage` from
  `cli/src/storage.rs`.
- The generic parts of the CLI schema: `docs`, `wal`, `snapshots`
  tables move into this crate's own `migrations/001_init.sql`. The
  `account` table is CLI-specific and stays behind.
- The free functions at the bottom of `cli/src/storage.rs`
  (`boot_doc`, `seed_snapshot`, `load_doc`) are DEK-holding glue over
  the `LocalStorage` trait. If they're generic over the trait (or can
  trivially be made so), move them into `monoplan-core`'s `storage`
  module where they belong; otherwise into `crates/storage-sqlite`.

What stays in `cli/`: `Account`, `SyncCursor`, `open_storage(profile)`,
and the account-table queries. The CLI needs to run its `account` DDL
against the same database file, so the shared crate's open/migration
entry point must accept caller-supplied extra schema (e.g.
`open_with_extra(path, &[("001_cli", CLI_SQL)])` or expose
`SqliteStorage::from_connection(Arc<Mutex<Connection>>)` plus an
`apply_core_schema(&mut Connection)` — pick whichever reads cleaner,
but keep the migration-ledger `_migrations` table owned by one place).
Edit the CLI's `001_init.sql` in place to hold only the `account` DDL.

**Verify:** `bun run test` fully green (CLI integration tests exercise
this storage heavily), `bun run build:wasm` still succeeds, `bun run
lint` no *new* warnings (lint is already red on main — pre-existing).

## Milestone 1 — FFI crate

New crate `core/ffi` (package name `monoplan-ffi`), mirroring how
`core/web` sits next to `core`. Add to `[workspace] members`, not
`default-members`.

- `crate-type = ["staticlib", "cdylib"]` — staticlib is what ships in
  the XCFramework; the host cdylib exists so uniffi library-mode
  bindgen has something to read metadata from.
- **uniffi** (latest published release — check crates.io, don't assume
  a version from memory), proc-macro mode (`#[uniffi::export]`,
  `#[derive(uniffi::Record)]`, etc.), no `.udl` file. Add the
  standard `uniffi-bindgen` bin target
  (`fn main() { uniffi::uniffi_bindgen_main() }`).

API surface — deliberately small; one exported object owning
storage + doc + DEK, mirroring what `load_doc` does in the CLI:

- `MonoplanStore::open(dir: String, dek: Vec<u8>) -> Result<Arc<MonoplanStore>>`
  — opens/creates `<dir>/monoplan.sqlite` via `monoplan-storage-sqlite`,
  boots the doc (first boot = fresh doc). DEK is raw bytes; key
  storage is the caller's problem (Keychain in layer 3, a test
  fixture for now). Also export a free fn `generate_dek() -> Vec<u8>`.
- Mutations: `add_item(list_id, text) -> String`,
  `edit_item_text(item_id, text)`, `set_item_done(item_id, done)`,
  `set_item_binned(item_id, binned)`, `add_list(name) -> String`.
- Reads: `items_in_list(list_id) -> Vec<ItemView>`,
  `all_lists() -> Vec<ListView>`, `export_json_string() -> String`
  (debugging aid).
- `ItemView` / `ListView` cross as uniffi Records — flat mirror
  structs of the `monoplan_core` types (same pattern as `core/web`'s
  JS-facing views), converted with `From` impls. Errors: one
  `#[derive(uniffi::Error)]` enum wrapping `DocError`/`StorageError`
  with readable messages.

Known risk to resolve here, not paper over: uniffi-exported objects
must be `Send + Sync`. `SqliteStorage` is (`Arc<Mutex<Connection>>`),
but check `Doc` (LoroDoc interior mutability) — if it isn't `Sync`,
wrap the doc side in a `Mutex` inside `MonoplanStore` and note why.

**Verify:** `cargo test -p monoplan-ffi` with a native Rust round-trip
test (open in tempdir → add → drop → reopen → item present), and
`cargo build -p monoplan-ffi --target aarch64-apple-darwin --release`
succeeds.

## Milestone 2 — XCFramework build script

`apple/build-xcframework.sh` (invoked from repo root; add a
`bun run build:apple` script to the root `package.json` wrapping it):

1. `rustup target add` as needed: `aarch64-apple-darwin`,
   `aarch64-apple-ios`, `aarch64-apple-ios-sim`. Build the staticlib
   for all three (`--release`). iOS isn't consumed yet but proving it
   *compiles* now is cheap insurance.
2. Generate Swift bindings once via library mode from the host build:
   `cargo run -p monoplan-ffi --bin uniffi-bindgen -- generate --library
   <host cdylib> --language swift --out-dir <gen dir>`.
3. Assemble headers: the generated `.h` + modulemap (rename the
   generated `*.modulemap` to `module.modulemap`) into a headers dir
   per platform.
4. `xcodebuild -create-xcframework` over the three static libs →
   `apple/MonoplanCore/MonoplanCoreFFI.xcframework`.
5. Copy the generated `.swift` file(s) into
   `apple/MonoplanCore/Sources/MonoplanCore/Generated/`.

Build artifacts (`.xcframework`, `Generated/`) are build outputs:
gitignore them, and have the script be the single way to produce them.
Also gitignore `xcuserdata/` and `.build/` now.

**Verify:** script runs clean from a fresh checkout state (delete
outputs, re-run).

## Milestone 3 — SwiftPM package + smoke test

`apple/MonoplanCore/Package.swift`:

- `binaryTarget` `MonoplanCoreFFI` → local path to the XCFramework.
- Source target `MonoplanCore`: the generated bindings plus (optionally)
  a thin hand-written Swift facade if the raw generated API is
  awkward — keep it minimal, don't build an abstraction layer yet.
- Test target with the smoke test: create a temp directory, open a
  store with a generated DEK, add a list + items, mark one done, read
  views back and assert; close/reopen the store and assert
  persistence survived.
- Platforms: `.macOS(.v14)`, `.iOS(.v17)` (adjust to whatever the
  toolchain on this machine supports — verify, don't assume).

**Verify:** `swift test --package-path apple/MonoplanCore` passes on the
macOS host. This is the acceptance gate for the whole plan.

## Milestone 4 — docs

`apple/README.md`: prerequisites (rustup targets, Xcode CLT), the two
commands (`bun run build:apple`, `swift test`), the layering
(Rust staticlib → XCFramework → SwiftPM → future app), and a pointer
to this plan. One paragraph each — not a manual.

## Acceptance checklist

- [ ] `bun run test` green (CLI unaffected by the storage hoist)
- [ ] `bun run build:wasm` green (core stayed wasm-clean)
- [ ] `cargo test -p monoplan-ffi` green
- [ ] `bun run build:apple` produces the XCFramework + bindings from scratch
- [ ] `swift test --package-path apple/MonoplanCore` green, including a
      close-and-reopen persistence assertion
- [ ] No new files under `local/` or generated artifacts committed
