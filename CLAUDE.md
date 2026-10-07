## Product Thesis

Monoplan is a low-friction, FOSS, single-human-user/small-team, E2EE, multi-device capture/clarify/organise tool for ideas, intents, goals, projects. It is flexible but built with particular regard to improving users' productivity & focus.

Workflow: a reserved primary capture list ("Inbox", id `inbox`), any number of user-created lists (each viewable as a fixed Backlog/Live/Done board), and a bin. Items move between lists, transition through the lifecycle (Backlog, Live, Done, Binned), can be restored, and deleted.

## Architecture

- **Rust core** (`core/`) — Loro CRDT, E2EE, sync engine. Compiles to native (CLI, server) and WASM (web) via `core/web/`.
- **Rust server** (`server/`) — sqlite-backed, sequenced encrypted-blob store + auth + WS relay. The server is *dumb*: it cannot read op contents, cannot run a Loro doc, cannot validate semantics. Its job is auth, ordering, durability, frontier tracking, snapshot orchestration. TODO: Compaction may put an asterisk on "DUMB".
- **CLI** (`cli/`) — Monoplan CLI
- **Web** (`js/web/`, consuming `core/web/` wasm via `js/core/`) — browser client; multi-device proof spans CLI ↔ web.
- **iOS / Android / native macOS** — future clients

E2EE: password-derived KEK wraps a randomly-generated DEK. DEK encrypts every op blob. Server has no key. Recovery via a user-held recovery code (independent wrap of DEK) is implemented; server-assisted escrow (Vault-backed, opt-in) is future work.

Sync: WebSocket per device. Auth on upgrade. Ops are append-only encrypted blobs with server-assigned monotonic ids. Each device tracks its `last_acked_op_id`; the minimum across active devices is the compaction horizon. Snapshots are produced by the most-acked active client on server request.

Migrations: while pre-release, keep exactly one migration file per database (`001_init.sql`) and edit it in place — never add incremental or legacy-bridge migrations.

## Source of truth

- `spec/*.md` — the contract for what's being built. **Before implementing anything, read the relevant spec.**
- `architecture.md`, `auth.md`, `cli.md`, `data-model.md`, `encryption.md`, `storage.md`, `sync-protocol.md`, `testing.md`

## Specs

| File | Concern |
|---|---|
| [`spec/architecture.md`](spec/architecture.md) | Workspace layout, crate graph, server-is-dumb thesis |
| [`spec/encryption.md`](spec/encryption.md) | DEK / KEK / recovery code, wrap-states, password change |
| [`spec/crypto.md`](spec/crypto.md) | Cryptographic primitive inventory; companion to `encryption.md` |
| [`spec/auth.md`](spec/auth.md) | HTTP signup / login / recover, device register / revoke, token model |
| [`spec/admin.md`](spec/admin.md) | Operator-only JSON API, authentication, deployment exposure |
| [`spec/sync-protocol.md`](spec/sync-protocol.md) | WS framing, push / pull / ack, frontier, snapshot orchestration |
| [`spec/storage.md`](spec/storage.md) | Sqlite schema, indexes, compaction policy |
| [`spec/data-model.md`](spec/data-model.md) | Loro doc layout, Item / ListMeta, lifecycle semantics |
| [`spec/vv-wal-separation.md`](spec/vv-wal-separation.md) | Local WAL (crash recovery) vs VV-derived sync separation; push_id retry contract |
| [`spec/peer-id-plan.md`](spec/peer-id-plan.md) | Stable per-device Loro peer id: flock-leased slot pool, VV-width bound |
| [`spec/board.md`](spec/board.md) | Board view: fixed Backlog / Live / Done lanes driven by item lifecycle |
| [`spec/focus.md`](spec/focus.md) | Focus lens: curated single-tier list-by-reference, FocusRef grammar, auto-remove-on-Done |
| [`spec/calendar-plan.md`](spec/calendar-plan.md) | Plan (not built): `when` date/datetime register beside `deadline`, agenda + month grid lenses, floating-only with reserved zone suffix, no recurrence |
| [`spec/events-plan.md`](spec/events-plan.md) | Events (built): reserved `events` list whose items surface only on the calendar, ticking optional, Unscheduled section, no item kind; recurrence unsolved |
| [`spec/place-plan.md`](spec/place-plan.md) | Place (built 2026-10-07): optional atomic `place` register `{label, lat?, lon?, address?, ref?}` shared by events and tasks; web lookup via Komoot's public Photon, debounced on input (not E2EE); notifications deferred |
| [`spec/search.md`](spec/search.md) | Local search index + command palette query contract |
| [`spec/urls.md`](spec/urls.md) | Fragment URLs for items / lists / views, history rules, internal links |
| [`spec/cli.md`](spec/cli.md) | Commands, local key storage, device bootstrap UX |
| [`spec/sharing-plan.md`](spec/sharing-plan.md) | Future (not built): multi-doc + sharing design + implementation plan |
| [`spec/pwa-plan.md`](spec/pwa-plan.md) | PWA conversion plan: manifest, service worker, `/api/session` probe |
| [`spec/notes-plan.md`](spec/notes-plan.md) | Plan (not built): `notes` / `text` as mergeable `LoroText`, wasm delta bridge, flat rich text, images by reference |
| [`spec/subtasks-plan.md`](spec/subtasks-plan.md) | Exploration (not built): subtasks as items located under a parent item, lifecycle cascade, "which part is next" rule |
| [`spec/tui-plan.md`](spec/tui-plan.md) | CLI load-time assessment (measured) + offline compaction fixes + TUI host plan |
| [`spec/testing.md`](spec/testing.md) | Integration test pattern, CLI driver |

## Build & run

Root `package.json` is a Bun workspace (`js/*`) with thin script wrappers — there is no JS to build, the scripts just front cargo and the config generator:

- `bun run config` — render `local/server.toml` from `js/config/templates/` (see `js/config/README.md`); `local/` holds gitignored dev artifacts
- `bun run server` / `bun run cli` — `cargo run -p monoplan-server --` / `cargo run -p monoplan --` (debug build; faster compile, slower sync on real-sized docs). Pass flags after the script name (e.g. `bun run server -- --bind 0.0.0.0:8000`). `bun run cli:prod` for the release build when you need real sync perf.
- `bun run build` / `bun run test` / `bun run fmt` / `bun run lint` — cargo equivalents
- `bun run build:wasm` — `wasm-pack build core/`. Always use this from the workspace root; bare `cargo build --target wasm32-...` will try to build `server`/`cli` for wasm and fail.
- `bun run typecheck` — `tsc --noEmit -p js/config`
