# CLI

Primary CLI client. Also the integration test surface for everything below the GUI.

## Binary

Single binary `monoplan`. Subcommands:

### Account
- `monoplan signup [--server URL]` — interactive: email, password, optional recovery code generation
- `monoplan login [--server URL]` — interactive: email, password
- `monoplan logout`
- `monoplan recover` — interactive: email, recovery code, set new password
- `monoplan password` — change password (logged in)

### Devices
- `monoplan devices` — list
- `monoplan devices revoke <device_id>`

### Sharing
- `monoplan share link <doc_id> [--expires-days N]` — print a single-use invite link for a workspace you own (`<server>/#invite_<secret>`; shown once, the server cannot rebuild it). Refused for the Personal workspace.
- `monoplan share pending <doc_id>` — unused links (id, created, expires)
- `monoplan share revoke <doc_id> <invite_id>`
- `monoplan share accept <link-or-secret>` — join the workspace behind a link; stores its DEK in `secrets.toml`. The CLI still syncs the Personal workspace only (`sharing-plan.md` Phase 4), so the joined workspace shows up in the web client first.
- `monoplan share members <doc_id>`

### Items
- `monoplan add <text> [--list <list>]` — `<text>` of `-` reads from stdin; one item per non-blank line. New items are created in **Backlog** (the workflow register is omitted).
- `monoplan ls [--list <list>]` — `--list events` lists events (`spec/events-plan.md`). Rows carry a trailing ` @<when>` (with `+<len>` glued on when a timed `when` has a duration, e.g. `@2026-09-12T14:00+1h30m`, or an all-day one spans more than a day, in whole days: `@2026-09-12+3d`), ` !<deadline>` and ` at:<place label>` when set; `--json` adds `when` / `duration` (minutes) / `deadline` / `place` (`{label, lat?, lon?, address?, ref?}`) fields (omitted when unset)
- `monoplan backlog <item_id>` — workflow → Backlog
- `monoplan todo <item_id>` — workflow → Todo
- `monoplan start <item_id>` — workflow → In Progress (stamps `started_at` on first entry)
- `monoplan review <item_id>` — workflow → Review
- `monoplan done <item_id>` — workflow → Done (stamps `done_at`)
- `monoplan cancel <item_id>` — workflow → Cancelled (closed like Done, but stamps nothing: not a completion)
- `monoplan bin <item_id>` — set the bin mask (`binned_at`); the workflow register is preserved
- `monoplan restore <item_id>` — clear the bin mask only; reveals the preserved workflow state (Backlog / Todo / In Progress / Review / Done / Cancelled)
- `monoplan mv <item_id> <list>` (`mv <item_id> events` makes the item an event; moving it back out makes it a task again, dates intact)
- `monoplan edit <item_id> <text>`
- `monoplan when <item_id> <YYYY-MM-DD[THH:MM] | ->` — set (all-day or timed, floating) or clear (`-`) the planned date; validation is the core's (`spec/calendar-plan.md`). A timed value on an item with no duration defaults it to 60 minutes; clearing also clears the duration.
- `monoplan duration <item_id> <minutes | [Nd][Nh][Nm] | ->` — set (`90`, `1h30m`, `2h`, `45m`, `3d`, `1d2h`) or clear (`-`) the duration in whole minutes; range (`1..=10080`) is the core's. Beside an all-day `when` the length counts whole days (`spec/calendar-plan.md`), so `3d` makes a three-day span; `ls` shows it as `+3d` only past one day.
- `monoplan deadline <item_id> <YYYY-MM-DD | ->` — set or clear the deadline
- `monoplan place <item_id> <label | -> [--at <lat>,<lon>] [--address <text>] [--new]` — set (a label, optionally with WGS84 decimal-degree coordinates and a display address) or clear (`-`) the place; validation is the core's (`spec/place-plan.md`). No lookup from the CLI, but a bare label that matches a known place (case-insensitively, whole label) takes that place whole, coordinates and address included; `--new`, `--at` or `--address` write exactly what was given.
- `monoplan places [query] [--limit N] [--json]` — the places already on items (not binned), deduped and newest use first (`spec/place-plan.md` "Reuse"): `<label>  <address | @lat,lon>  (<count>)`, the address and count only when there is one to show; `query` keeps rows whose label or address words start with its words; `--json` emits `[{ place, count, last_used }]`.
- `monoplan agenda [--days N] [--today YYYY-MM-DD] [--json]` — Open dated items by day: Today first (always shown, with overdue deadlines folded in, oldest first), then each non-empty day up to `N` days out (default 14). Rows carry the same `@` / `!` / `at:` tags as `ls` plus a trailing `(overdue)` / `(due today)` tone; a `when` before today places nothing, so an item with only a past `when` is not listed. Events place by the same rules; Open events with neither date print in an `Unscheduled` group after Today (in `--json`, a group with `day: null` whose rows omit `placed_by`). `--today` overrides the local date, for scripts and tests. `--json` emits `[{ day, today, rows: [{ id, text, list_id, state, when?, duration?, deadline?, place?, placed_by, tone }] }]`.

Lifecycle is the atomic `lifecycle` workflow register (`[state, at]`, states Backlog | Todo | In Progress | Review | Done | Cancelled) masked by the orthogonal `binned_at` bin flag — see `spec/data-model.md` "Lifecycle". Each workflow command writes the register `[state, now]` (and clears any bin mask) in a single commit; re-applying the current resolved state is a no-op. `ls` boxes carry a one-character state mark (` ` backlog, `-` todo, `>` in progress, `?` review, `x` done, `/` cancelled, `~` binned). `ls` hides closed items (Done and Cancelled) unless `--done`; `agenda` never shows cancelled items.

### Focus
The curated single-tier Focus lens (`spec/focus.md`). References items across lists; the item stays in its home list.

- `monoplan focus` — list the Focus view (Open referenced items, in curated order)
- `monoplan focus add <item_id> [pos]` — add a reference (default append); no-op if already focused
- `monoplan focus rm <item_id>` — remove the reference (item untouched)
- `monoplan focus mv <item_id> <pos>` — reorder within Focus

Marking a focused item `done` removes it from Focus automatically; binning/deleting drops it from the view.

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
- `monoplan bin show`
- `monoplan bin empty`
- `monoplan bin rm <item_id>`

### Status
- `monoplan status` — server URL, account email, device id, last successful sync timestamp, `last_acked_seq`, pending-push op count. Read-only against local state; never opens a WS and never starts the daemon.

### Cache
- `monoplan cache status` — profile directory and `monoplan.sqlite` size. Read-only; never opens a WS.
- `monoplan cache clear [--force]` — truncate the doc cache (`ops` + `snapshots`) and reset the per-doc sync cursor to 0, **keeping** account identity (so it does not log you out); the next command starts a daemon that rehydrates from the server. Refuses while a daemon is running. If there are unsynced local ops, prompts for confirmation (TTY) or refuses with an error (non-TTY) unless `--force` is passed.

### Export
- `monoplan export-json [--out PATH]` — semantic account export: built-in/user lists plus items/lifecycle/timestamps as regular JSON. Defaults to stdout; `--out` writes a file. This is a portability dump, not a CRDT backup/restore format.

### Sync
- `monoplan sync` — push any pending local ops now and report whether the server is reachable. The daemon pulls continuously while connected, so there is nothing separate to pull. Waits for an in-progress connection attempt (or starts one if the daemon is in backoff) rather than answering from stale state; an unreachable server exits non-zero.

### Daemon
- `monoplan daemon` — run the resident process attached to the terminal (`spec/daemon-plan.md`). Refuses if one already holds the profile.
- `monoplan daemon --detach` — what a client runs to start it: prints one readiness line on stdout (`ready` / `already-running` / `error: …`) and keeps running; stderr goes to `daemon.log` in the profile dir.
- `monoplan daemon stop` — ask it to exit and wait until the profile is released.
- `monoplan daemon status` — `not running`, or `running  pid N  build B` (noting when the build differs from this executable's).
- `monoplan daemon install` / `uninstall` — not built: launchd agent / systemd user unit.

## Process model

One binary, two roles. `monoplan daemon` owns the profile for as long as it runs: the sqlite file, the Loro doc, the DEK, the server WebSocket, and a Unix socket in the profile dir. Every other doc command is a thin client: it connects to that socket, sends the parsed command, prints the reply, and exits. Clients never open sqlite and never hold a key.

Dispatch, at the top of `Cli::run`:

- Doc commands (`add`, `ls`, workflow verbs, `mv`, `edit`, dates, `place`/`places`, `agenda`, `focus`, `lists`, `bin`, `export-json`) are forwarded as a `DocCmd` and their output comes back as text. `add -` reads stdin in the client; `export-json --out` writes the file in the client.
- `status` and `cache status` are direct, read-only sqlite reads and never contact or start the daemon. `cache clear` refuses while a daemon is running.
- `signup`, `login`, `recover`, `password` and `logout` work on the profile files, so each first stops a running daemon; `login` and `recover` then sync through a fresh one, which is what starts it.
- `share` is HTTP against the server and runs in the client.

### Starting the daemon

A client that finds no daemon starts one: it spawns `monoplan daemon --detach` from its own executable (`current_exe()`), in a new session (`setsid`), with the profile dir passed explicitly, and blocks on the child's first stdout line. Two clients racing to start it resolve through `daemon.lock`: the loser prints `already-running` and exits, and its parent connects normally. The first start prints one stderr line (`started monoplan daemon (pid N); …`).

A client that finds `daemon.lock` held but the socket not answering is looking at a daemon that is still starting or is wedged; it retries for a few seconds, then fails with a pointer to `daemon stop`.

`--no-spawn` (or `MONOPLAN_NO_SPAWN=1`) turns a missing daemon into an error, for scripts and CI.

Every connection starts with a hello carrying a build id (package version plus the executable's mtime). A daemon from a different build is asked to shut down and restarted from the client's executable, so a rebuilt CLI never talks to a stale daemon.

### Daemon behaviour

- Holds the server WebSocket open; reconnects with capped exponential backoff (0.5 s to 30 s, jitter), pings every 30 s and treats a missed pong as a disconnect (laptop sleep). Connecting runs as its own task, so commands are served while it is in progress. Offline is a state: commands keep working against the local doc and ship when the socket returns.
- Every reply carries whether the daemon is online and when it last completed an exchange. The CLI prints `offline; last synced 3m ago` (or `offline; never synced`) on stderr when a command was served without a server behind it, including under `--json`, whose stdout shapes are unchanged.
- Persists on a 1 s tick and immediately after any command that mutated, so a change is on disk before the client hears back and on the wire right after. Folds the WAL at the usual thresholds, and fully on exit.
- Exits on SIGINT/SIGTERM, on `daemon stop`, or when it notices the profile dir has been purged.
- There is no idle timeout: background sync is the point.
- Not built yet: the locked state (`Unlock` over the socket when the DEK is not readable non-interactively), `install`, wake notifications, and the subscription stream for live clients (`spec/daemon-plan.md`).

## Local state

Single account per install. One dir under XDG paths (`~/.local/share/monoplan/` on linux, equivalents elsewhere) — `logout` wipes it, signup/login re-creates it. Two side-by-side test accounts in dev: point `MONOPLAN_DATA_DIR` at distinct roots.

- `monoplan.sqlite` — doc cache (append-only `ops` + per-doc `snapshots`), the per-doc sync cursor (`docs.last_acked_server_seq` / `last_sync_at`), and the singleton `account` row (account/device/primary-doc ids + email). `primary_doc_id` is the server-assigned id of the account's Home doc, used to key local snapshot storage. See `spec/storage.md`.
- `daemon.lock` — flock held by the running daemon (never unlinked; see `spec/peer-id-plan.md` for the rules). `peer-<n>.lock` — the peer-id slot leases.
- `daemon.sock` — the daemon's Unix socket (mode 0600; falls back to a hash-named socket in the temp dir when the profile path is too long for `sun_path`). `daemon.log` — stderr of a detached daemon.
- `config.toml` — `{ server_url }`. Bootstrap input (needed before the db exists); operator-editable.
- `secrets.toml` — `device_token` plus one `[docs."<doc_id>"] dek_hex = "…"` table per doc the account is a member of, in cleartext. The DEK is a doc key (`encryption.md`): login/recover store one entry per membership (the primary's unwrapped with the password or recovery KEK, the rest with the primary's DEK — the Home DEK); `monoplan password` re-wraps only the primary entry, since every other server-side wrap is under the Home DEK. Also the "logged in" marker: its presence is what `monoplan status`/commands gate on.

Secrets in OS keychain (`security` on macOS, `libsecret` on linux):
- `monoplan:<account_id>:token` — device auth token
- `monoplan:<account_id>:dek` — DEK (only when "stay logged in" is chosen; otherwise re-derived from password each session)

Recovery code is **never** persisted by the client — shown once at signup, user records it themselves.

## Bootstrap UX

### First device
```
$ monoplan signup
Server: https://monoplan.example
Email: dan@example.com
Password: ********
Generate recovery code? [Y/n]
  → 12 words shown once, user must type them back to confirm
Device name [hostname]:
Done. Doc initialized.
```

### Second device
```
$ monoplan login
Server: https://monoplan.example
Email: dan@example.com
Password: ********
Device name [hostname]:
Syncing... (snapshot, then ops)
Done.
```

### Recovery
```
$ monoplan recover
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
