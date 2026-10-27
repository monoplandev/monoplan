# CLI

Primary CLI client. Also the integration test surface for everything below the GUI.

## Binary

Single binary `monoplan`. Subcommands:

### No account (local-only workspace, built 2026-10-27)
No account is needed to use the CLI. The first doc command on a fresh install creates a **local-only workspace** (`cli/src/bootstrap.rs`): a doc id and a DEK minted locally, the doc seeded with the same onboarding "Welcome" list a signup gets, and a profile written with no device token. Every command and the daemon then work against it exactly as for an account's device, minus the server: the host never connects, replies carry `local_only` and never an offline notice, `monoplan status` says `local-only workspace`, `daemon status` says `local only (no account)`. It is the CLI's counterpart of the web client's anonymous session (`auth.md` §Device pairing).

- `monoplan auth signup` from that state **adopts** the workspace: its DEK becomes the Home DEK and its doc id is sent as `primary_doc_id`, so nothing is copied or re-encrypted and the whole local history ships on the first connect. The command says so (`Your local workspace is now this account's Personal workspace.`) and syncs right after the recovery code is confirmed.
- `monoplan auth login` / `recover` from that state **discard** it (the account's doc replaces it; merging two seeded histories is not built, same as the web). They say so and ask for confirmation first, pointing at `signup` (keeps it) and `export-json` (saves a copy).
- `monoplan auth logout` on a local-only workspace does nothing (`Not logged in (the local workspace is kept).`); `monoplan db clear` is the way to erase it, and always asks (`--force` when not a TTY), then re-seeds a fresh one under the same id and key.
- Verbs that need a server (`share …`, `auth password`) fail with `no account on this install; …`.
- The macOS app hosts a CLI-made local-only profile like any other (its Settings get an `Err` to the `Account` request instead of an identity); it does not create one itself yet.

### Account (`monoplan auth …`)
- `monoplan auth signup [--server URL]` — interactive: email, password, recovery code shown once and typed back; then syncs. The body is `signup::signup` (`Credentials` → `Outcome { account_id, recovery_code, adopted }`), the same split as `login`
- `monoplan auth login [--server URL]` — interactive: email, password. `--server` defaults to `https://alpha.monoplan.app` (or `MONOPLAN_SERVER`); the dev stack is `MONOPLAN_SERVER=http://127.0.0.1:8000`. The body is `login::login`, shared with the app's sign-in sheet (`monoplan-ffi::login`)
- `monoplan auth logout`
- `monoplan auth recover` — interactive: email, recovery code, set new password
- `monoplan auth password` — change password (logged in)

### Devices
- `monoplan auth devices` — list
- `monoplan auth devices revoke <device_id>`

### Sharing
- `monoplan share link <doc_id> [--expires-days N]` — print a single-use invite link for a workspace you own (`<server>/#invite_<secret>`; shown once, the server cannot rebuild it). Refused for the Personal workspace.
- `monoplan share pending <doc_id>` — unused links (id, created, expires)
- `monoplan share revoke <doc_id> <invite_id>`
- `monoplan share accept <link-or-secret>` — join the workspace behind a link; stores its DEK in `secrets.toml`. The CLI still syncs the Personal workspace only (`sharing-plan.md` Phase 4), so the joined workspace shows up in the web client first.
- `monoplan share members <doc_id>`
- `monoplan share remove <doc_id> <email>` — owner removes a member (resolved through the member list; never yourself, never the last owner). Forward-only: they keep what they already synced.

### Items
- `monoplan item add <text> [--list <list>]` — `<text>` of `-` reads from stdin; one item per non-blank line. New items are created in **Backlog** (the workflow register is omitted).
- `monoplan item ls [--list <list>] [--done] [--json]` — `--list events` lists events (`spec/events-plan.md`). Rows carry a trailing ` @<when>` (with `+<len>` glued on when a timed `when` has a duration, e.g. `@2026-09-12T14:00+1h30m`, or an all-day one spans more than a day, in whole days: `@2026-09-12+3d`), ` !<deadline>` and ` at:<place label>` when set; `--json` adds `when` / `duration` (minutes) / `deadline` / `place` (`{label, lat?, lon?, address?, ref?}`) fields (omitted when unset)
- `monoplan item set <item_id> <flags…>` — every per-item field as a flag, so one call can set several; at least one is required. Each flag is its own core setter and its own commit, applied in the order below; the first error stops the sequence and the writes before it stand. Duration and coordinates are parsed before anything is written; dates are validated by the core at write time.
  - `--text <text>`
  - `--list <list>` — move (append) to that list; `--list events` makes the item an event, moving it back out makes it a task again, dates intact
  - `--lifecycle <backlog | todo | in_progress | review | done | cancelled>` — set the workflow state (`in-progress` is accepted for `in_progress`). `in_progress` stamps `started_at` on first entry; `done` stamps `done_at`; `cancelled` is closed like Done but stamps nothing (not a completion). The names are the core's canonical ones, the same `state` that `--json` outputs print.
  - `--when <YYYY-MM-DD[THH:MM] | ->` — set (all-day or timed, floating) or clear (`-`) the planned date; validation is the core's (`spec/calendar-plan.md`). A timed value on an item with no duration defaults it to 60 minutes; clearing also clears the duration.
  - `--duration <minutes | [Nd][Nh][Nm] | ->` — set (`90`, `1h30m`, `2h`, `45m`, `3d`, `1d2h`) or clear (`-`) the duration in whole minutes; range (`1..=10080`) is the core's. Beside an all-day `when` the length counts whole days (`spec/calendar-plan.md`), so `3d` makes a three-day span; `ls` shows it as `+3d` only past one day.
  - `--deadline <YYYY-MM-DD | ->` — set or clear the deadline
  - `--place <label | -> [--coords <lat>,<lon>] [--address <text>] [--new-place]` — set (a label, optionally with WGS84 decimal-degree coordinates and a display address) or clear (`-`) the place; validation is the core's (`spec/place-plan.md`). No lookup from the CLI, but a bare label that matches a known place (case-insensitively, whole label) takes that place whole, coordinates and address included; `--new-place`, `--coords` or `--address` write exactly what was given. The three companions require `--place`.
  - `--star [pos]` — add a Starred reference (default append; `pos` is 1-based, `end` appends); no-op if already starred or not Open. Conflicts with `--unstar`.
  - `--unstar` — remove the Starred reference (item untouched)
- `monoplan item bin <item_id>` — set the bin mask (`binned_at`); the workflow register is preserved
- `monoplan item restore <item_id>` — clear the bin mask only; reveals the preserved workflow state (Backlog / Todo / In Progress / Review / Done / Cancelled)
- `monoplan places [query] [--limit N] [--json]` — the places already on items (not binned), deduped and newest use first (`spec/place-plan.md` "Reuse"): `<label>  <address | @lat,lon>  (<count>)`, the address and count only when there is one to show; `query` keeps rows whose label or address words start with its words; `--json` emits `[{ place, count, last_used }]`.
- `monoplan agenda [--days N] [--today YYYY-MM-DD] [--json]` — Open dated items by day: Today first (always shown, with overdue deadlines folded in, oldest first), then each non-empty day up to `N` days out (default 14). Rows carry the same `@` / `!` / `at:` tags as `ls` plus a trailing `(overdue)` / `(due today)` tone; a `when` before today places nothing, so an item with only a past `when` is not listed. Events place by the same rules; Open events with neither date print in an `Unscheduled` group after Today (in `--json`, a group with `day: null` whose rows omit `placed_by`). `--today` overrides the local date, for scripts and tests. `--json` emits `[{ day, today, rows: [{ id, text, list_id, state, when?, duration?, deadline?, place?, placed_by, tone }] }]`.

Lifecycle is the atomic `lifecycle` workflow register (`[state, at]`, states Backlog | Todo | In Progress | Review | Done | Cancelled) masked by the orthogonal `binned_at` bin flag — see `spec/data-model.md` "Lifecycle". `item set --lifecycle` writes the register `[state, now]` (and clears any bin mask) in a single commit; re-applying the current resolved state is a no-op. `ls` boxes carry a one-character state mark (` ` backlog, `-` todo, `>` in progress, `?` review, `x` done, `/` cancelled, `~` binned). `ls` hides closed items (Done and Cancelled) unless `--done`; `agenda` never shows cancelled items.

### Focus
The curated single-tier Focus lens (`spec/focus.md`), named "Starred" on every user-facing surface including the CLI (since 2026-10-26). References items across lists; the item stays in its home list.

- `monoplan starred` — list the Starred view (Open referenced items, in curated order); `--json` for machine output
- `monoplan item set <item_id> --star [pos]` / `--unstar` — add or remove a reference (see "Items")
- `monoplan starred mv <item_id> <pos>` — reorder within Starred

`monoplan focus [add|rm|mv]` is the hidden pre-rename spelling and keeps working; the IPC frame is still `DocCmd::Focus`.

Marking a starred item `done` removes it from Starred automatically; binning/deleting drops it from the view.

### Lists
- `monoplan lists ls` — active lists only by default; `--archived` shows only archived lists, `--all` shows both. `--json` output includes `archived_at` (null for active lists).
- `monoplan lists add <name>`
- `monoplan lists rename <list> <name>`
- `monoplan lists archive <list>` — remove a list from the active workspace without touching its items, ordering, or metadata (`spec/data-model.md` "Archived lists"). Refuses for `inbox` and `events`.
- `monoplan lists unarchive <list>` — restore an archived list to the active workspace.

There is deliberately **no user-facing list delete**: archive is the only way to
remove a list from the workspace. The core's `delete_list` stays internal
(tests / future permanent deletion).

### Bin
Binning and restoring an item are `monoplan item bin` / `item restore` (see "Items"); the namespace is the bin itself.

- `monoplan bin show`
- `monoplan bin empty`
- `monoplan bin rm <item_id>`

### Status
- `monoplan status` — server URL, account email, device id, last successful sync timestamp, `last_acked_seq`, pending-push op count. Read-only against local state; never opens a WS and never starts the daemon. On a local-only workspace: `Account: none — local-only workspace` plus the doc id; `--json` has `local_only: true` and null identity / server fields.

### Db
- `monoplan db status` — profile directory and `monoplan.sqlite` size. Read-only; never opens a WS.
- `monoplan db clear [--force]` — truncate the doc store (`wal` + `snapshots`) and reset the per-doc sync cursor to 0, **keeping** account identity (so it does not log you out); the next command starts a daemon that rehydrates from the server. Refuses while a daemon is running. If there are unsynced local ops, prompts for confirmation (TTY) or refuses with an error (non-TTY) unless `--force` is passed. On a local-only workspace everything is unsynced, so it always asks, and afterwards re-seeds the onboarding doc under the same id and key rather than leaving an empty one.

### Export
- `monoplan export-json [--out PATH]` — semantic account export: built-in/user lists plus items/lifecycle/timestamps as regular JSON. Defaults to stdout; `--out` writes a file. This is a portability dump, not a CRDT backup/restore format.

### Daemon
- `monoplan daemon` — run the resident process attached to the terminal (`spec/daemon-plan.md`). Refuses if one already holds the profile.
- `monoplan daemon --detach` — what a client runs to start it: prints one readiness line on stdout (`ready` / `already-running` / `error: …`) and keeps running; stderr goes to `daemon.log` in the profile dir.
- `monoplan daemon stop` — ask it to exit and wait until the profile is released.
- `monoplan daemon status` — `not running`, or `running  pid N  build B` followed by `online`, the offline notice, or `local only (no account)` (and noting when the build differs from this executable's).
- `monoplan daemon install` / `uninstall` — run the daemon at login: a launchd agent (`~/Library/LaunchAgents/app.monoplan.daemon.plist`, `gui/<uid>` domain) on macOS, a systemd user unit (`~/.config/systemd/user/monoplan.service`) on Linux. Both run a plain `monoplan daemon` from this executable's resolved path, pinned to the active profile via `MONOPLAN_DATA_DIR`, stderr appended to `daemon.log`. Restart on failure only, so `daemon stop` sticks until the next login; `install` is idempotent and also (re)starts it now. Stops a hand-started daemon first so the agent's instance owns the profile. `logout` uninstalls (a purged profile would fail at every login). `daemon status` notes `installed` when stopped. This is the headless path (Linux, CLI-only Macs); on macOS the app hosts the profile itself whenever it runs (`spec/daemon-plan.md`, amendment 2026-10-27) and takes an agent's daemon over at launch, so `install` refuses while the app hosts.

## Process model

One binary, two roles; and on macOS a third host, the app, which runs the same loop in-process (`monoplan-ffi::Host`) and reports `embedded` in its hello. `monoplan daemon` owns the profile for as long as it runs: the sqlite file, the Loro doc, the DEK, the server WebSocket, and a Unix socket in the profile dir. Every other doc command is a thin client: it connects to that socket, sends the parsed command, prints the reply, and exits. Clients never open sqlite and never hold a key.

Dispatch, at the top of `Cli::run`:

- Doc commands (`item …`, `places`, `agenda`, `starred`, `lists`, `bin`, `export-json`) open the profile through `bootstrap::open_or_init`, which creates the local-only workspace on a fresh install (holding `daemon.lock` while it writes, so a racing second command waits and finds it), then are forwarded as a `DocCmd` and their output comes back as text. `item add -` reads stdin in the client; `export-json --out` writes the file in the client. A hand-run `monoplan daemon` on a fresh install creates the workspace the same way.
- `status` and `db status` are direct, read-only sqlite reads and never contact or start the daemon. `db clear` refuses while a daemon is running.
- `signup`, `login`, `recover`, `password` and `logout` work on the profile files, so each first stops a running host and holds `daemon.lock` until the files are written (`client::claim`), keeping the app and any auto-spawn out meanwhile; `login` and `recover` then sync through a fresh host, which is what starts it (or the app, if it re-hosted first).
- `share` is HTTP against the server and runs in the client.

### Starting the daemon

A client that finds no daemon starts one: it spawns `monoplan daemon --detach` from its own executable (`current_exe()`), in a new session (`setsid`), with the profile dir passed explicitly, and blocks on the child's first stdout line. Two clients racing to start it resolve through `daemon.lock`: the loser prints `already-running` and exits, and its parent connects normally. The first start prints one stderr line (`started monoplan daemon (pid N); …`).

A client that finds `daemon.lock` held but the socket not answering is looking at a daemon that is still starting or is wedged; it retries for a few seconds, then fails with a pointer to `daemon stop`.

`--no-spawn` (or `MONOPLAN_NO_SPAWN=1`) turns a missing daemon into an error, for scripts and CI.

Every connection starts with a hello carrying a build id (package version plus the executable's mtime). A headless daemon from a different build is asked to shut down and restarted from the client's executable, so a rebuilt CLI never talks to a stale daemon. An embedded host (the macOS app) is never restarted: its executable is never the CLI's, so only the package version has to agree, and a mismatch fails the command naming both versions. `daemon status` says `hosted by Monoplan.app` for it.

### Daemon behaviour

- Holds the server WebSocket open; reconnects with capped exponential backoff (0.5 s to 30 s, jitter), pings every 30 s and treats a missed pong as a disconnect (laptop sleep). Connecting runs as its own task, so commands are served while it is in progress. Offline is a state: commands keep working against the local doc and ship when the socket returns. A local-only workspace is a third state: the host never starts a connect, `Sync` requests answer at once, and `SyncStatus.local_only` (absent on the wire when false) tells clients not to call the reply offline.
- Every reply carries whether the daemon is online, when it last completed an exchange, and, while offline, why the last connect failed or the socket dropped (`offline_reason`). The CLI prints `offline; last synced 3m ago` (or `offline; never synced`), with the reason in parentheses when there is one, on stderr when a command was served without a server behind it, including under `--json`, whose stdout shapes are unchanged. The host logs each distinct reason once at `warn`; the embedded host writes its log to `daemon.log` in the profile dir, the file the headless daemon's stderr is appended to.
- Persists on a 1 s tick and immediately after any command that mutated, so a change is on disk before the client hears back and on the wire right after. Folds the WAL at the usual thresholds, and fully on exit.
- Exits on SIGINT/SIGTERM, on `daemon stop`, or when it notices the profile dir has been purged. The embedded host does the same; the app starts a fresh one as soon as the profile is free again, so `daemon stop` against the app is a restart, not a stop (quit the app to stop syncing on a Mac).
- There is no idle timeout: background sync is the point.
- `Watch` turns a connection into a watcher: the daemon replies `Watching` and then sends `Event(Changed)` whenever the doc changes (a local command committed, remote ops applied; a burst is one event) and `Event(Sync(..))` when online/offline or `last_synced_at` moves. Requests still work on a watching connection; replies and events are told apart by variant. The macOS menubar app (`apple/MonoplanMac`) is the first watcher: it re-fetches `agenda --days 0 --json` on `Changed`. No CLI verb exposes `Watch` yet.
- Not built yet: the locked state (`Unlock` over the socket when the DEK is not readable non-interactively), wake notifications, and the doc-mirror subscription for full-view clients (`spec/daemon-plan.md`).

## Local state

Single account per install. One dir under XDG paths (`~/.local/share/monoplan/` on linux, equivalents elsewhere) — `logout` wipes it, signup/login re-creates it. Two side-by-side test accounts in dev: point `MONOPLAN_DATA_DIR` at distinct roots.

- `monoplan.sqlite` — doc store (append-only `wal` + per-doc `snapshots`), the per-doc sync cursor (`docs.last_acked_server_seq` / `last_sync_at`), and the singleton `account` row (primary-doc id, plus account/device ids + email, all three NULL for a local-only workspace). `primary_doc_id` is the id of the Home doc, used to key local snapshot storage: server-assigned at login/recover, minted locally for a local-only workspace and kept through signup. See `spec/storage.md`.
- `daemon.lock` — flock held by the running daemon (never unlinked; see `spec/peer-id-plan.md` for the rules). `peer-<n>.lock` — the peer-id slot leases.
- `daemon.sock` — the daemon's Unix socket (mode 0600; falls back to a hash-named socket in the temp dir when the profile path is too long for `sun_path`). `daemon.log` — stderr of a detached daemon.
- `config.toml` — `{ server_url }`. Written by signup/login/recover; absent on a local-only workspace, which has no server to name. Operator-editable.
- `secrets.toml` — optional `device_token` plus one `[docs."<doc_id>"] dek_hex = "…"` table per doc this install can open, in cleartext. The DEK is a doc key (`encryption.md`): login/recover store one entry per membership (the primary's unwrapped with the password or recovery KEK, the rest with the primary's DEK — the Home DEK); a local-only workspace holds its one minted doc; `monoplan auth password` re-wraps only the primary entry, since every other server-side wrap is under the Home DEK. Its presence is the "profile exists" marker (`Profile::active`), and the token the "has an account" one (`Secrets::has_account`): server-backed verbs fail with `ConfigError::NoAccount` without it.

Secrets in OS keychain (`security` on macOS, `libsecret` on linux):
- `monoplan:<account_id>:token` — device auth token
- `monoplan:<account_id>:dek` — DEK (only when "stay logged in" is chosen; otherwise re-derived from password each session)

Recovery code is **never** persisted by the client — shown once at signup, user records it themselves.

## Bootstrap UX

### No account
```
$ monoplan item add "first thought"
created a local workspace in ~/.local/share/monoplan; `monoplan auth signup` syncs it across devices
started monoplan daemon (pid 4242); run `monoplan daemon install` to start it at login
0193…
```

### First device
```
$ monoplan auth signup
Server: https://monoplan.example
Email: dan@example.com
Password: ********
Device name [hostname]:
  → recovery code: 12 words shown once, user must type them back to confirm
Syncing…
Done. (An existing local workspace is adopted as the account's Personal workspace; otherwise the doc is seeded fresh.)
```

### Second device
```
$ monoplan auth login
Server: https://monoplan.example
Email: dan@example.com
Password: ********
Device name [hostname]:
Syncing... (snapshot, then updates)
Done.
```

### Recovery
```
$ monoplan auth recover
Server: https://monoplan.example
Email: dan@example.com
Recovery code (12 words): ...
New password: ********
Device name [hostname]:
Syncing...
Done.
```

## Output

Default output: human-readable. `--json` flag on every read command emits machine-parseable JSON for tests and scripting.

Item and list ids: full uuid v7 hex (32 chars), shown verbatim and required in full when an id is passed in. The built-in lists `inbox` and `events` (`spec/events-plan.md`) are the two literal ids, accepted wherever `<list>` is. (Earlier drafts of this spec proposed prefix matching; dropped because it adds parsing complexity for marginal ergonomic gain over shell completion / copy-paste.)
