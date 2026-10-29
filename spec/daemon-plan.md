# Daemon plan

Status: phases 1 to 3 built 2026-10-21 (`cli/src/daemon.rs` host + socket server, `cli/src/ipc.rs`, `cli/src/client.rs`, every doc verb ported, `-s` removed, `daemon stop|status`; tests in `cli/tests/daemon.rs` and `cli/tests/cli_bin.rs`). Phase 5's projection tier built 2026-10-26 (`Watch` / `Watching` / `Event`; first watcher is the macOS menubar app in `apple/MonoplanMac`). `daemon install|uninstall` built 2026-10-27 (`cli/src/service.rs`: the headless path for Linux and CLI-only Macs). Amended 2026-10-27 (see "Decision", amendment): on macOS the app hosts the loop in-process (`monoplan-ffi::Host`, `apple/MonoplanMac/Sources/MonoplanMac/HostController.swift`) for its whole life, the headless daemon is the alternative rather than a partner, and login uses the Login Items API; the mirror tier is no longer needed for the macOS window. Amended 2026-10-29 (`spec/mac-window-plan.md`): the mirror is an FFI surface on `Host`, never socket frames; the window is a webview running the web client over a mirror doc with its own peer; the tray moves to the FFI too, so the socket is the CLI's alone. Not built: the locked state, wake notifications. Written 2026-10-21. The built behaviour is documented in `spec/cli.md` "Process model"; this file keeps the reasoning and the remaining plan. Supersedes the deleted
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

1. **Liveness.** Reactive `UpdatesBroadcast`, deadline and `when`
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

GUI processes (menubar app, window, TUI) are clients of the same
socket, never hosts. Decided 2026-10-21: on desktop `monoplan daemon`
is the only host. The macOS app bundles the `monoplan` binary and runs
`daemon install` on first launch, the way the Syncthing and Tailscale
desktop apps manage their daemons; it does not embed a second host
loop. Reasons: a GUI that hosts takes the doc down when the user quits
it, leaving the CLI with nothing to talk to; and two implementations of
one host protocol is a permanent cross-check tax. A GUI with its own
full stack is rejected for the same reason as mixed mode: on one
profile it is a second owner of one doc, on two profiles it is two
devices on one laptop, each pinning the horizon and seeing the other's
edits only via the server. Mobile is the exception (see "Mobile hosts"):
iOS cannot run a daemon, so the iOS app embeds core as a full host.
Decided 2026-10-26: the menubar item and the macOS window app are one
process (accessory activation policy, status item always present, window
on demand), not two; the daemon makes this a packaging choice only. It
lives in `apple/MonoplanMac/` (`bun run mac`).

**Amended 2026-10-27: on macOS the normal path is one process.** The
app hosts the doc in-process: the same Rust host loop, split into
`daemon::start` / `daemon::serve` and exposed through `monoplan-ffi` as
`Host` (a background thread with its own tokio runtime). The tray and
the window are clients of the app's own `Host` through the FFI
(amended 2026-10-29; until then the tray was a socket client of its own
process). The socket is served for the CLI only. The window is a
webview running the web client over a mirror doc that `Host` seeds and
converges (`spec/mac-window-plan.md`); the mirror tier below is that
FFI surface and is never carried by the socket. The headless `monoplan daemon` remains the
host for CLI-only use, Linux and servers (`daemon install` writes the
systemd unit or launchd agent for that path) and is what the CLI
auto-spawns when nothing holds the profile. Rules:

The app hosts whenever it runs (revised 2026-10-27, replacing the
"alternatives, not partners" rules under which the app stayed a client
of a foreign daemon and refused `Shutdown`; that branch was code for a
case nobody needs, and the refusals it forced on the CLI were the only
user-visible friction). The app only ever talks to its own host.

1. Whenever its socket is down and the profile is signed in, the app
   starts its host. `Host.start` first takes a headless `monoplan
   daemon` over: asks it to exit over the socket, waits for the lock,
   then claims the profile (`client::take_over`). The headless daemon
   fills in on a Mac only while the app is not running, which is what
   the CLI's auto-spawn does after the app quits.
2. The app's host honours `Shutdown` like any host. `logout`, `login`,
   `recover`, `signup` and `password` from the terminal stop it and
   hold `daemon.lock` while they rewrite the profile (`client::claim`,
   dropped before anything that needs a host, such as `login`'s
   initial sync), so neither the app nor a client's auto-spawn boots a
   half-written profile; the app re-hosts once the lock is free.
3. The app never takes over another app's host (two copies taking turns
   would never settle), a host still starting, or a profile held by a
   command: `Host.start` reports `Busy`, the menu says "profile in use"
   and the client's reconnect backoff retries. `daemon install` refuses
   while the app hosts: the agent is for Macs without the app.
4. A client never restarts an embedded host for staleness. The build-id
   check applies to headless daemons only; against an embedded host the
   package version must match, else the command fails naming both
   versions. The bundled CLI, symlinked onto `PATH` from Settings, is
   how the terminal stays at the app's version.
5. Login item is `SMAppService.mainApp` (System Settings › Login
   Items), not a launchd agent. `daemon install` is the headless path
   and is not used on a Mac that runs the app.

Why the 2026-10-21 reasons no longer hold: "a GUI that hosts takes the
doc down when the user quits it" is covered by the CLI's auto-spawn;
"two implementations of one host protocol" does not arise because the
app embeds the Rust loop rather than rewriting it; and the mirror tier
was the larger cost, paid only to serve full-view GUI clients of an
out-of-process host. iOS embeds core the same way, so the two Apple
apps share one shape.

## Process topology

```
monoplan item add ──UDS──┐
monoplan item ls  ──UDS──┤
monoplan tui      ──UDS──┼──>  host loop ──WS──> server
tray / window     ──FFI──┘        │   (`monoplan daemon`, or the macOS app's
                  (in-process)    │    `monoplan-ffi::Host`; the FFI edge exists only there)
                                  ├── <profile>/monoplan.sqlite
                                  ├── <profile>/daemon.lock   (held: "a daemon owns this profile")
                                  ├── <profile>/peer-<n>.lock (held: its Loro peer id)
                                  └── <profile>/daemon.sock
```

- The socket lives in the profile dir, beside the db. The profile dir is
  already per-account and overridable with `MONOPLAN_DATA_DIR`, so test
  isolation comes for free. macOS caps `sun_path` at 104 bytes; if the
  profile path is too long (tempdirs in tests can be), fall back to
  `$TMPDIR/monoplan-<blake3(profile dir)[..16]>.sock` and record the
  path in `<profile>/daemon.sock.path` so clients find it.
- The daemon holds `daemon.lock` for its lifetime: the same kernel
  flock mechanism and rules as the peer lease (`spec/peer-id-plan.md`),
  released on any exit path, never unlinked. That lock, not a peer slot,
  is the election and the "is a daemon running" probe: the peer slot
  only identifies a Loro peer, and the daemon claims the lowest free one
  like any process. A client that finds `daemon.lock` held but the
  socket not answering is looking at a daemon that is still starting
  (or wedged), and waits with bounded retries rather than proceeding
  alone.
- Windows is out of scope (named pipes later; nothing here precludes
  it).

## Client dispatch

Top of `Cli::run`:

1. `daemon …` subcommands run the host (below).
2. `status` and `db status` stay direct, read-only sqlite reads (WAL
   mode allows a concurrent reader) and never spawn or contact the
   daemon, as `spec/cli.md` already requires. `db clear` refuses
   while `daemon.lock` is held.
3. Account bootstrap (`signup`, `login`, `recover`, `logout`,
   `password`) runs in the client process as today, because it works
   on the profile files, not the doc. Each first sends `Shutdown` to a
   running daemon and waits for `daemon.lock` to release; the next command
   respawns it. Pre-release this is simpler than teaching the daemon to
   hot-reload a profile. `login` and `recover` drop their "initial
   sync" step: the daemon syncs on start.
4. Everything else: connect to the socket. If nothing listens, spawn
   the daemon (below), then connect. Encode the command, send, print
   the reply (human or `--json` is a client-side rendering of the same
   reply type), exit with the reply's status.

`-s` / `--sync` / `MONOPLAN_SYNC` are removed, and so is `monoplan sync`
(2026-10-27). The daemon is always syncing when it can, and a daemon
restart is a fresh connection attempt. The IPC `Sync` request ("flush
now, answer when the attempt resolves, hard error when unreachable")
stays as the barrier `login` / `recover` and the tests use.

### Auto-spawn

The client spawns `current_exe() daemon --detach` as a child with stdin
nulled, stdout piped and stderr sent to `daemon.log`. The child's stdout
*is* the readiness pipe: it `setsid`s (done by the parent's `pre_exec`),
takes `daemon.lock`, binds the socket, opens the doc, then prints one
line (`ready`, `already-running`, or `error: …`) and never uses stdout
again. The client blocks on that line, so startup is deterministic and a
failure carries a message. No `fork` in the client: it is multithreaded
once tokio is up.

Two clients may race and spawn two daemons. The loser fails the
`daemon.lock` flock and exits 0 after printing `already-running`; its
parent connects normally. The lease is the election; nothing else is
needed.

First spawn prints one stderr line:
`started monoplan daemon (pid N); run 'monoplan daemon install' to start it at login`.

`--no-spawn` or `MONOPLAN_NO_SPAWN=1` turns a missing daemon into an
error (scripts, CI).

### Version skew

The hello exchange carries a build id: `CARGO_PKG_VERSION` plus the
executable's own mtime, so any rebuild changes it without a `build.rs`.
On mismatch the client sends `Shutdown`, waits for `daemon.lock`, and
respawns from its own executable. Pre-release there is no compatibility
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
    Subscribe,                  // this connection becomes a doc mirror
    Unlock { password: String },
    Shutdown,
}

enum Reply {
    Hello { build_id: String, locked: bool, sync: SyncStatus },
    Ok { payload: Payload, sync: SyncStatus },   // typed payload per request
    Err { code: ErrorCode, message: String },
    Snapshot { doc: Vec<u8>, sync: SyncStatus },  // Subscribe reply: full Loro export
    Event(AppEvent),            // only on subscribed connections
}

struct SyncStatus {
    online: bool,                // socket up and the doc open on it right now
    last_synced_at: Option<i64>, // unix millis of the last completed exchange,
                                 // persisted beside the sync cursor across runs
}

enum AppEvent {
    Update(Vec<u8>),            // Loro update bytes since the last event
    Sync(SyncStatus),           // online/offline transitions
    FullResync,                 // epoch change or trim: re-Subscribe
}
```

**Staleness is in every reply** (built). A daemon that lost its
WebSocket keeps serving the local doc, which is correct, but a one-shot
`-s` used to fail loudly where the daemon now succeeds quietly.
`SyncStatus` on `Hello`, `Output` and `Synced` is how a client tells the
difference. The CLI prints one stderr line when `online` is false
(`offline; last synced 3m ago`, or `offline; never synced`), for `--json`
runs too, since stderr stays out of parsed output. Carrying the struct
inside `--json` payloads is not done: every documented `--json` shape in
`spec/cli.md` is a bare array, and wrapping them is a contract change to
decide separately.

**Live clients mirror the doc, not projections.** Decided 2026-10-21,
amended 2026-10-29. A full-view client holds a `Doc` of its own, seeded
by a snapshot from the host and converged by the plaintext update bytes
of every later apply (local command, remote frame, another mirror's
import), and runs core's own read functions locally: board, agenda,
focus, search index, place suggestions. This keeps the host
view-agnostic (the alternative, shipping typed projections plus
per-view events, makes the protocol the view model and needs
incremental-apply code in every GUI) and keeps exactly one
implementation of every view, in core, shared by CLI, web, macOS and
iOS.

Two things changed on 2026-10-29 (`spec/mac-window-plan.md`). The
mirror is an FFI surface on the in-process `Host` (`subscribe`,
`import`), never socket frames: the only full-view client is the app's
own webview, the headless daemon has no such client, and the socket
stays the CLI's. And the mirror **writes**, under a peer id of its own
leased from the slot pool: the webview runs the web store, which is
built on a locally mutated `Doc` (synchronous AppEvents, its own
`UndoManager`, the notes `LoroText` bridge), so a read-only mirror with
intents over the boundary would rebuild the store's write half on the
host. The mirror's ops reach the host on every flush, are imported
tagged, and are captured, WALed and pushed by the existing VV-based
capture like anything the CLI committed. The 2026-10-21 text called a
mirror with its own peer "the escape hatch, not the default"; for a
webview it is the default. Cost: a second in-memory `Doc` per window,
one more peer slot, and a snapshot load per window open. The webview
holds no DEK, no storage and no socket.

The one-shot CLI keeps using projections: a process that prints a list
and exits should not import a doc to do it.

Lightweight live clients also use projections (built 2026-10-26). A
tray that shows today's events, a widget, or a notification scheduler
needs one projection plus a change signal, not a mirror: `Watch` turns
the connection into a watcher (`Reply::Watching`), after which the
daemon sends `Reply::Event(WatchEvent::Changed)` whenever the doc's
`AppEvent` queue was non-empty after a local command or a remote frame
(a burst is one event; a watcher that lags the 64-event buffer gets one
`Changed`, since that is all the stream says anyway) and
`Event(WatchEvent::Sync(SyncStatus))` on every status transition.
Requests still work on a watching connection; with one in flight at a
time the client tells replies and events apart by variant. The client
re-requests its projection, debounced, and on a timer. The mirror
(below) is the shape for full-view clients and lives on the FFI, not
here: socket watchers never mirror, as Tailscale's frontends run on the
equivalent of this tier (`WatchIPNBus`) and never mirror the backend.

Types live in `cli/src/ipc.rs` for now; they move to a `crates/ipc`
crate when the first non-CLI client (Swift, TUI) needs them.

Chattiness is the only real IPC cost (a round trip is tens of
microseconds). Rules: one request per CLI verb (never one per item);
live clients `Subscribe` once, import the `Snapshot`, then apply
`Update` events to their mirror. `FullResync` means drop the mirror and
`Subscribe` again.

### Locked state

If the DEK cannot be read non-interactively, the daemon starts
**locked**: socket up, doc not booted, `Hello` says `locked: true`.
Clients prompt for the password and send `Unlock`; the daemon derives
the KEK, unwraps the DEK, boots the doc, connects. Today `secrets.toml`
holds the DEK in cleartext so this state is never entered; the plumbing
exists so the keychain move is not a protocol change.

## Daemon host

`monoplan daemon [--detach]` (attached to the terminal without the
flag), plus `daemon stop`, `daemon status`, and `daemon install` /
`daemon uninstall` (built 2026-10-27, `cli/src/service.rs`), which
write or remove the launchd agent plist on macOS or the systemd user
unit on Linux; both run a plain `monoplan daemon` from the installing
executable's resolved path. Restart is on failure only, so `stop`
sticks until login. The agent and the CLI should be one file on disk
(the app bundle's copy, symlinked into `PATH`): the build-id check
below otherwise has a rebuilt CLI replace the agent's daemon with its
own, and the agent take over again at the next login.

The host is `Session` held open instead of torn down after one command
(`cli/src/sync.rs`). Concretely:

- `flush(self)` splits into `persist()` (the existing
  `persist_engine_state`, run on a tick) and `drain_outbox()`.
- One `select!` over: client requests (one task per connection feeds
  them through a channel; the loop runs each inline against the doc),
  the in-flight connect task, inbound WS frames, a ~1 s persist tick,
  the ping tick, the reconnect timer, and shutdown signals (`SIGTERM`,
  `SIGINT`, `Shutdown` request). Connecting is a spawned task, never
  awaited inline, so a slow server cannot stall commands.
- `Sync` answers immediately when idle online; otherwise it parks until
  the current connect attempt resolves (initial pull done → online,
  attempt failed → offline), kicking a connect if the host was in
  backoff.
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
  release `daemon.lock` (drop the `File`).

The daemon is headless. GUI run loops (NSApplication) belong to client
processes.

## Transport

The daemon holds the WebSocket open. Polling is the mobile compromise,
not the desktop design:

- The server already pushes `UpdatesBroadcast` on every seq advance, so a
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
true with one change of mechanism. `daemon::spawn(profile)` runs the
host as a task on the test's runtime and returns a `DaemonHandle` with
a `watch` of `DaemonStatus` (online, last contiguous seq) and a clean
`shutdown()`; tests wait on the status rather than sleeping. A
binary-level `spawn_daemon` that execs `monoplan daemon --ready-fd`
joins it in phase 2. CLI tests then drive the real binary (or the client
code path in-process) against it. The two tests that use
`Session::open` directly keep working: `Session` remains the library
type the daemon host wraps.

The convergence matrix in `spec/testing.md` is unchanged; "two clients
live" now means two daemons on two profiles, which is also more honest
about the product.

## Phases

1. **Host loop** (built). `monoplan daemon` runs `Session` in the
   select loop with WS reconnect, liveness ping and the persist tick,
   attached to the terminal. No socket yet. `Session::open` refuses
   with `DaemonRunning` while `daemon.lock` is held, so the mixed-mode
   hazard cannot occur during the transition. Wake notifications are
   not hooked yet; the pong timeout covers sleep for now.
2. **Socket and client** (built). `ipc.rs`, hello and build id,
   auto-spawn with the readiness line, `--no-spawn`.
3. **Port the verbs** (built). Every doc command is a pure function
   over `(doc, args, out)` (`commands::run_doc`); `-s` and
   `Session::open` are gone; account commands stop the daemon first;
   `db clear` refuses while it runs. Phases 2 and 3 had to land
   together: once the daemon auto-starts and holds the lock, any
   un-ported one-shot verb would simply be refused.
4. **Lifecycle** (partly built). `daemon stop|status|install|uninstall`
   exist. Not built: the locked state and `Unlock`, wake
   notifications.
5. **Subscriptions.** Projection tier built 2026-10-26: `Watch` and
   `Event` fan-out from the host after local commands and remote
   frames, tested in `cli/tests/daemon.rs`
   (`watcher_hears_sync_local_and_remote_changes`), consumed by the
   macOS menubar app over the socket until it moves to the FFI. The
   mirror tier is not a socket feature (decided 2026-10-29): it is
   `Host::subscribe` / `Host::import` on the FFI, planned and tested in
   `spec/mac-window-plan.md`, reached through an internal `HostCmd`
   enum the socket handler cannot construct.

Update `spec/cli.md` ("Sync lifecycle", "Status", the `-s` flag) as
each phase lands rather than in one rewrite.
