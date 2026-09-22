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

### Items
- `monoplan add <text> [--list <list>]` — `<text>` of `-` reads from stdin; one item per non-blank line. New items are created in **Backlog** (the workflow register is omitted).
- `monoplan ls [--list <list>]` — rows carry a trailing ` @<when>` (with `+<len>` glued on when a timed `when` has a duration, e.g. `@2026-09-12T14:00+1h30m`) and ` !<deadline>` when set; `--json` adds `when` / `duration` (minutes) / `deadline` fields (omitted when unset)
- `monoplan backlog <item_id>` — workflow → Backlog
- `monoplan todo <item_id>` — workflow → Todo
- `monoplan start <item_id>` — workflow → In Progress (stamps `started_at` on first entry)
- `monoplan review <item_id>` — workflow → Review
- `monoplan done <item_id>` — workflow → Done (stamps `done_at`)
- `monoplan bin <item_id>` — set the bin mask (`binned_at`); the workflow register is preserved
- `monoplan restore <item_id>` — clear the bin mask only; reveals the preserved workflow state (Backlog / Todo / In Progress / Review / Done)
- `monoplan mv <item_id> <list>`
- `monoplan edit <item_id> <text>`
- `monoplan when <item_id> <YYYY-MM-DD[THH:MM] | ->` — set (all-day or timed, floating) or clear (`-`) the planned date; validation is the core's (`spec/calendar-plan.md`). A timed value on an item with no duration defaults it to 60 minutes; clearing also clears the duration.
- `monoplan duration <item_id> <minutes | [Nh][Nm] | ->` — set (`90`, `1h30m`, `2h`, `45m`) or clear (`-`) the duration in whole minutes; range (`1..=10080`) is the core's. Only shown beside a timed `when`.
- `monoplan deadline <item_id> <YYYY-MM-DD | ->` — set or clear the deadline
- `monoplan agenda [--days N] [--today YYYY-MM-DD] [--json]` — Open dated items by day: Today first (always shown, with overdue deadlines and past planned dates folded in, oldest first), then each non-empty day up to `N` days out (default 14). Rows carry the same `@` / `!` tags as `ls` plus a trailing `(overdue)` / `(due today)` tone; a past `when` carries no tone. `--today` overrides the local date, for scripts and tests. `--json` emits `[{ day, today, rows: [{ id, text, list_id, state, when?, duration?, deadline?, placed_by, tone }] }]`.

Lifecycle is the atomic `lifecycle` workflow register (`[state, at]`, states Backlog | Todo | In Progress | Review | Done) masked by the orthogonal `binned_at` bin flag — see `spec/data-model.md` "Lifecycle". Each workflow command writes the register `[state, now]` (and clears any bin mask) in a single commit; re-applying the current resolved state is a no-op. `ls` boxes carry a one-character state mark (` ` backlog, `-` todo, `>` in progress, `?` review, `x` done, `~` binned).

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
- `monoplan lists archive <list>` — remove a list from the active workspace without touching its items, ordering, or metadata (`spec/data-model.md` "Archived lists"). Refuses for `inbox`.
- `monoplan lists unarchive <list>` — restore an archived list to the active workspace.

There is deliberately **no user-facing list delete**: archive is the only way to
remove a list from the workspace. The core's `delete_list` stays internal
(tests / future permanent deletion).

### Bin
- `monoplan bin show`
- `monoplan bin empty`
- `monoplan bin rm <item_id>`

### Status
- `monoplan status` — server URL, account email, device id, last successful sync timestamp, `last_acked_seq`, pending-push op count. Read-only against local state; never opens a WS.

### Cache
- `monoplan cache status` — profile directory and `monoplan.sqlite` size. Read-only; never opens a WS.
- `monoplan cache clear [--force]` — truncate the doc cache (`ops` + `snapshots`) and reset the per-doc sync cursor to 0, **keeping** account identity (so it does not log you out); the next `monoplan sync` rehydrates from the server. If there are unsynced local ops, prompts for confirmation (TTY) or refuses with an error (non-TTY) unless `--force` is passed.

### Export
- `monoplan export-json [--out PATH]` — semantic account export: built-in/user lists plus items/lifecycle/timestamps as regular JSON. Defaults to stdout; `--out` writes a file. This is a portability dump, not a CRDT backup/restore format.

### Sync
- `monoplan sync` — pull peer ops, push any pending local ops, then exit. Connect failure is a hard error.

## Sync lifecycle

CLI subcommands are one-shot and **offline by default**. Reads (`ls`, `status`, `lists ls`) and writes (`add`, `done`, `mv`, ...) operate against the local Loro doc only; mutations append to the `ops` table in `monoplan.sqlite` and ship on the next sync.

To hit the network, pass `-s` / `--sync` on any command (or set `MONOPLAN_SYNC=1`): open WS → version handshake → `PullOps { since_seq: last_acked_seq }` → apply → run the command → `PushOps` if anything changed → `Ack` → close. The dedicated `monoplan sync` command is the same path with no doc mutation.

A future TUI may hold the WS open while running and surface `OpsBroadcast` reactively in the same `monoplan` binary. The daemon question stays deferred until the TUI exists and proves it needs more than that.

### Connect behaviour

- Default: no network attempt. Local doc is authoritative. Mutations queue in the `ops` table of `monoplan.sqlite`.
- `-s` / `--sync` / `MONOPLAN_SYNC=1`: attempt WS connect with a ~2s timeout. On failure (no network, captive portal, server down), fall back to local-only and print a one-line stderr warning: `offline — sync deferred (<reason>)`. The command still runs against the local doc.
- `monoplan sync`: same connect attempt, but failure exits non-zero — there's no local work to fall back to.
- `monoplan login` and `monoplan recover` always attempt an initial sync after writing the profile; failure is a soft warning (they've already provisioned the account, the user can `monoplan sync` later).
- Pending local ops live in the `ops` table of `monoplan.sqlite`; the next sync pushes them as part of `PushOps`.

## Local state

Single account per install. One dir under XDG paths (`~/.local/share/monoplan/` on linux, equivalents elsewhere) — `logout` wipes it, signup/login re-creates it. Two side-by-side test accounts in dev: point `MONOPLAN_DATA_DIR` at distinct roots.

- `monoplan.sqlite` — doc cache (append-only `ops` + per-doc `snapshots`), the per-doc sync cursor (`docs.last_acked_server_seq` / `last_sync_at`), and the singleton `account` row (account/device/primary-doc ids + email). `primary_doc_id` is the server-assigned id of the account's Home doc, used to key local snapshot storage. See `spec/storage.md`.
- `config.toml` — `{ server_url }`. Bootstrap input (needed before the db exists); operator-editable.
- `secrets.toml` — `{ device_token, dek_hex }` in cleartext. Also the "logged in" marker: its presence is what `monoplan status`/commands gate on.

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

Item and list ids: full uuid v7 hex (32 chars), shown verbatim and required in full when an id is passed in. Built-in list `inbox` is the one literal id. (Earlier drafts of this spec proposed prefix matching; dropped because it adds parsing complexity for marginal ergonomic gain over shell completion / copy-paste.)
