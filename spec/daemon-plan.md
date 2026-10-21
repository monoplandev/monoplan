# Daemon plan

Status: plan, not built. Written 2026-10-21. Supersedes the deleted
`spec/tui-plan.md` (its "no daemon" conclusion rested on boot time,
which is no longer the question) and answers the daemon question that
`spec/cli.md` "Sync lifecycle" deferred.

Read first: `spec/architecture.md` (sans-IO engine, host
responsibilities), `spec/peer-id-plan.md` (the slot lease this plan
reuses as its election), `spec/vv-wal-separation.md` (what a host must
persist and when), `spec/shared-worker.md` (the web's version of the
same shape: one runtime, N mirrors). Reference implementation of a
long-lived host: `core/web/src/lib.rs` plus `js/core/`.

## Decision

One binary, two roles. `monoplan daemon` is a resident process that
owns the profile: sqlite, the Loro doc, the DEK, the WebSocket, and a
Unix socket it serves. Every other `monoplan` invocation is a thin
client: connect, send one command, print the reply, exit. The client
never opens sqlite and never holds a key.

The one-shot CLI (open, mutate, flush, exit) was a proof of concept and
is removed, not kept as a fallback. There is no in-process mode for
commands. Reasons, in order:

1. **Liveness.** Reactive `OpsBroadcast`, deadline and `when`
   notifications, background sync, and reliably serving the
   snapshot-producer role all need a process that outlives a command.
2. **Single ownership.** `spec/peer-id-plan.md` already requires one
   live doc per peer id. A resident owner is the cleanest way to
   satisfy it; the slot pool becomes the contention path, not the norm.
3. **Key containment.** The DEK is unlocked once into daemon memory.
   Clients never see it. (The cleartext `secrets.toml` stays for now;
   moving it behind the keychain is a separate step the daemon makes
   possible.)
4. **Mixed mode is a hazard, not a convenience.** A one-shot process
   writing WAL rows beside a live daemon does not corrupt anything (the
   flock pool prevents the peer-id collision) but the daemon's
   in-memory doc never sees those rows: they are stranded until the
   daemon restarts. Strict client mode removes the hazard entirely.

What this is not about: speed. With WAL folding independent of sync
state (`spec/vv-wal-separation.md`), a one-shot boot is single-digit
milliseconds. The daemon buys liveness and ownership, not startup time.

Future GUI hosts (menubar app, window, TUI) are clients of the same
socket. On macOS the resident process may later be the menubar app
embedding core via the Swift FFI (`spec/swift-ffi-plan.md`) and serving
the same protocol; the slot-0 lease makes it and `monoplan daemon`
mutually exclusive, and clients connect to whichever is listening.
Design the protocol so any host can serve it; decide the macOS owner
later.

## Process topology

```
monoplan add "x"  ──UDS──┐
monoplan ls       ──UDS──┤
monoplan tui      ──UDS──┼──>  monoplan daemon ──WS──> server
(tray / window)   ──UDS──┘        │
                                  ├── <profile>/monoplan.sqlite
                                  ├── <profile>/peer-0.lock   (held)
                                  └── <profile>/daemon.sock
```

- The socket lives in the profile dir, beside the db. The profile dir is
  already per-account and overridable with `MONOPLAN_DATA_DIR`, so test
  isolation comes for free. macOS caps `sun_path` at 104 bytes; if the
  profile path is too long (tempdirs in tests can be), fall back to
  `$TMPDIR/monoplan-<blake3(profile dir)[..16]>.sock` and record the
  path in `<profile>/daemon.sock.path` so clients find it.
- The daemon claims **slot 0** of the peer lease and holds it for its
  lifetime. Nothing else ever runs with slot 0 held: a client that finds
  slot 0 taken but the socket not answering is looking at a daemon that
  is still starting (or wedged), and waits with bounded retries. It does
  not take slot 1 and proceed.
- Windows is out of scope (named pipes later; nothing here precludes
  it).

## Client dispatch

Top of `Cli::run`:

1. `daemon …` subcommands run the host (below).
2. `status` and `cache status` stay direct, read-only sqlite reads (WAL
   mode allows a concurrent reader) and never spawn or contact the
   daemon, as `spec/cli.md` already requires. `cache clear` refuses
   while slot 0 is held.
3. Account bootstrap (`signup`, `login`, `recover`, `logout`,
   `password`) runs in the client process as today, because it works
   on the profile files, not the doc. Each first sends `Shutdown` to a
   running daemon and waits for slot 0 to release; the next command
   respawns it. Pre-release this is simpler than teaching the daemon to
   hot-reload a profile. `login` and `recover` drop their "initial
   sync" step: the daemon syncs on start.
4. Everything else: connect to the socket. If nothing listens, spawn
   the daemon (below), then connect. Encode the command, send, print
   the reply (human or `--json` is a client-side rendering of the same
   reply type), exit with the reply's status.

`-s` / `--sync` / `MONOPLAN_SYNC` are removed. The daemon is always
syncing when it can; `monoplan sync` becomes "ask the daemon to flush
now and report whether it is online", with the same hard-error
semantics when the server is unreachable.

### Auto-spawn

The client spawns `current_exe() daemon --detach` as a child with stdio
nulled, passing one inherited pipe fd (`--ready-fd N`). The child calls
`setsid`, claims slot 0, binds the socket, then writes a single `0` byte
to the pipe and closes it; on failure it writes the error text instead.
The client blocks on the pipe, so startup is deterministic and a
failure carries a message. No `fork` in the client: it is multithreaded
once tokio is up.

Two clients may race and spawn two daemons. The loser fails the slot-0
flock (or the bind) and exits 0 after writing a distinguishable
"already running" byte; its parent connects normally. The lease is the
election; nothing else is needed.

First spawn prints one stderr line:
`started monoplan daemon (pid N); run 'monoplan daemon install' to start it at login`.

`--no-spawn` or `MONOPLAN_NO_SPAWN=1` turns a missing daemon into an
error (scripts, CI).

### Version skew

The hello exchange carries a build id (`CARGO_PKG_VERSION` plus
`git_hash`/build timestamp baked in by `build.rs`). On mismatch the
client sends `Shutdown`, waits for slot 0, respawns from its own
executable, and retries once. Pre-release there is no compatibility
window.

## IPC protocol

Unix domain socket, length-prefixed MessagePack frames (`u32` big-endian
length, then `rmp-serde` payload), the same serde types as everything
else. Auth is the socket file's mode (0600) plus peer credentials
(`getpeereid` / `SO_PEERCRED`) checked against the daemon's uid.

The protocol is the **command surface**, not raw ops. The daemon owns
the `Doc`; clients send intents and read projections.

```rust
enum Request {
    Hello { build_id: String },
    // one variant per CLI verb, carrying the parsed args:
    AddItems { list: String, texts: Vec<String> },
    Ls { list: String, done: bool },
    SetWorkflow { id: String, state: WorkflowState },
    …
    Sync,                       // flush now; reply says online/offline
    Subscribe,                  // this connection becomes a mirror
    Unlock { password: String },
    Shutdown,
}

enum Reply {
    Hello { build_id: String, locked: bool },
    Ok(Payload),                // typed payload per request
    Err { code: ErrorCode, message: String },
    Event(AppEvent),            // only on subscribed connections
}
```

Types live in `cli/src/ipc.rs` for now; they move to a `crates/ipc`
crate when the first non-CLI client (Swift, TUI) needs them.

Chattiness is the only real IPC cost (a round trip is tens of
microseconds). Rules: one request per CLI verb (never one per item);
live clients `Subscribe` once, take a full projection on attach, then
apply `AppEvent`s to a local mirror. `AppEvent::FullResync` means
re-fetch.

### Locked state

If the DEK cannot be read non-interactively, the daemon starts
**locked**: socket up, doc not booted, `Hello` says `locked: true`.
Clients prompt for the password and send `Unlock`; the daemon derives
the KEK, unwraps the DEK, boots the doc, connects. Today `secrets.toml`
holds the DEK in cleartext so this state is never entered; the plumbing
exists so the keychain move is not a protocol change.

## Daemon host

`monoplan daemon [--foreground | --detach] [--ready-fd N]`, plus
`daemon stop`, `daemon status`, `daemon install` / `daemon uninstall`
(writes or removes the launchd agent plist on macOS, the systemd user
unit on Linux; both run `monoplan daemon --foreground`).

The host is `Session` held open instead of torn down after one command
(`cli/src/sync.rs`). Concretely:

- `flush(self)` splits into `persist()` (the existing
  `persist_engine_state`, run on a tick) and `drain_outbox()`.
- One `select!` over: accepted socket connections and their requests,
  inbound WS frames, a ~1 s persist tick, a reconnect timer, and
  shutdown signals (`SIGTERM`, `SIGINT`, `Shutdown` request).
- Persist cadence mirrors the web host: `capture_local_ops` on the tick
  and after every request that mutated, `snapshot_if_wal_exceeds` with
  the existing thresholds on the hot path, and a fold on clean exit.
- Remote frames apply as they arrive; `drain_events` after each
  apply fans out to subscribed connections.
- Offline is a state, not an error: requests keep working against the
  local doc. Transport policy is in the next section.
- No idle timeout. Background sync is the point. `daemon stop` is the
  way out, and `install` makes the question moot.
- On clean exit: persist, fold, close the WS, unlink the socket,
  release slot 0 (drop the `File`).

The daemon is headless. GUI run loops (NSApplication) belong to client
processes.

## Transport

The daemon holds the WebSocket open. Polling is the mobile compromise,
not the desktop design:

- The server already pushes `OpsBroadcast` on every seq advance, so a
  connected daemon sees other devices' changes within a round trip.
- Snapshot production requires a connected, caught-up client
  (`spec/sync-protocol.md`). An always-connected daemon is the natural
  producer, and its continuous acks keep the horizon moving.
- One idle socket with a ping every 30 to 60 s is negligible on a
  laptop battery; periodic HTTP would cost more and arrive later.

Reconnect is host policy (`spec/architecture.md`): capped exponential
backoff, roughly 1 s doubling to 60 s with jitter, reset on a
successful handshake. While the server is unreachable this *is* a slow
poll; there is no second mechanism. Liveness: the daemon pings on an
interval and treats a missed pong as a disconnect, since after laptop
sleep the socket is usually dead before the OS says so. Where the host
exposes a wake notification (macOS `NSWorkspace` wake, systemd-logind
on Linux) the daemon drops and reconnects immediately on wake so a
lid-open is synced in about a second rather than after a ping timeout.

### Mobile hosts (context, not built)

iOS suspends backgrounded apps within seconds and permits no persistent
socket; `BGAppRefreshTask` and silent push are both budgeted and
best-effort. The mobile host therefore syncs as a short exchange on
every launch and return-to-foreground (hello, pull since acked seq,
push, ack, close), exactly the one-shot `Session` exchange, which stays
the library primitive. Reminders are local notifications scheduled from
synced state. Server-initiated push (APNs/FCM poke, content-free, dumb-
server compatible) is deferred; it would be additive, and its real
cost is the publisher-run gateway self-hosters would need. An HTTP
variant of the same exchange (one msgpack request/response) is worth
adding for mobile so iOS can use a background `URLSession`; on desktop
its only role would be a fallback when a proxy blocks WebSocket
upgrades. An epoch change (`spec/trim-minimal.md`) surfaces to a mobile
host as "full refresh on next open".

## Testing

`spec/testing.md` makes the CLI the end-to-end surface; that stays
true with one change of mechanism. The harness gains
`spawn_daemon(profile) -> DaemonHandle` that runs
`monoplan daemon --foreground --ready-fd` on a scratch profile and
kills it on drop. CLI tests then drive the real binary (or the client
code path in-process) against it. The two tests that use
`Session::open` directly keep working: `Session` remains the library
type the daemon host wraps.

The convergence matrix in `spec/testing.md` is unchanged; "two clients
live" now means two daemons on two profiles, which is also more honest
about the product.

## Phases

1. **Host loop.** `monoplan daemon --foreground` runs `Session` in the
   select loop with WS reconnect and the persist tick. No socket yet.
   Commands gain a guard: refuse with "daemon running" when slot 0 is
   held, so the mixed-mode hazard cannot occur during the transition.
2. **Socket and client.** `ipc.rs`, hello and build id, auto-spawn with
   the readiness pipe, `--no-spawn`. Port the item verbs first (`add`,
   `ls`, workflow transitions); delete their one-shot bodies as each
   moves.
3. **Port the rest.** Lists, focus, agenda, places, bin, share, export,
   `sync`. Remove `-s` and `Session::open(sync)`'s sync flag. Account
   commands gain the shutdown-first step.
4. **Lifecycle.** `daemon stop|status|install|uninstall`, the locked
   state and `Unlock`.
5. **Subscriptions.** `Subscribe` and event fan-out. Land this with the
   first live client (TUI or tray), not before: there is nothing to
   test it against until then.

Update `spec/cli.md` ("Sync lifecycle", "Status", the `-s` flag) as
each phase lands rather than in one rewrite.
