# Testing

Sqlite-only for now. No mocked database. No mocked server. The test pyramid skews heavy on E2E because the server-is-dumb thesis means there isn't much logic to unit-test in isolation — convergence and protocol behavior are emergent from the whole system.

## Layers

1. **Unit tests** (`#[test]` in each crate)
   - Encryption primitives (KEK derivation, wrap/unwrap, AEAD round-trip)
   - Loro mutation helpers
   - ID generation, prefix-matching
2. **CLI/system tests** (`cli/tests/`)
   - Primary home for happy-path end-to-end coverage
   - Real server + real sqlite + real `monoplan_cli` Rust client path (`net`, auth flows, `sync::Session`)
   - Covers account bootstrap and the normal multi-device lifecycle:
     - signup/login/recovery/password-change success paths
     - two-device sync and convergence
     - offline edits followed by catch-up
     - snapshot upload / snapshot bootstrap success paths
   - Reuse `cli/tests/support/mod.rs` as the shared harness for server startup, profile materialization, auth helpers, and bounded polling
   - Prefer long-lived `monoplan_cli::sync::Session` tests for snapshot/sync happy paths instead of ad hoc websocket clients
3. **Server integration tests** (`server/tests/`)
   - Real server, real sqlite (temp file or `:memory:`)
   - Drives HTTP + WS via thin ad hoc clients only where that is the point of the test
   - Narrowed to low-level server contract and adversarial/weird-client scenarios:
     - malformed or unexpected frames
     - auth rejection / unauthorized upgrade
     - handshake/version rejection
     - stale ack behavior
     - disconnect / timeout retry paths
     - direct broadcast / subscriber registry edge cases
     - exact wire-level protocol seams that are awkward to observe via `Session`

`core/tests/` stays focused on shared engine correctness and sans-IO sync behavior. Do not move whole-system server/CLI ownership questions into `core/tests/`.

E2E is the load-bearing surface. Required matrix:

- signup → add items → exit → re-login → items still there
- two clients live → mutations on A appear on B within bounded time
- A offline, makes mutations, comes online → mutations appear on B
- A and B both offline making mutations → both come online → both converge to same Loro state
- snapshot threshold reached → snapshot taken → fresh device joins → pulls snapshot + tail successfully
- recovery code flow → new password works → DEK preserved → items intact

Avoid duplicate happy-path coverage across `server/tests/` and `cli/tests/` unless the server-side test is proving a narrower wire-contract detail that the CLI-system test does not cover.

## Known coverage gaps (multi-doc transport)

The multiplexed connection (`sync-protocol.md` §Doc binding) is proven end to end against the real server: `server/tests/sync.rs` (two docs on one connection, per-doc broadcast isolation across three connections, per-doc lease release on disconnect) and `js/core/test/sync-e2e.test.ts` (two private docs, one socket per device, bootstrap and live broadcast without bleed). Three router behaviours are **not** exercised, because the real server never produces the condition and we chose not to add a scripted WebSocket server to fake it:

1. **One doc stalls, the others proceed.** Doc A stuck in `Pulling` (server never sends `complete: true`) must not block doc B's open, pull, push, or acks on the same socket. The engines are independent state machines by construction, so this holds by inspection, but nothing asserts it.
2. **Rejected handshake reconnects with backoff.** `HelloRejected` (or a version mismatch) must surface through `onError`, close the socket, and schedule a retry rather than bring any engine online. The real server only rejects an unknown protocol version, which the bridge never sends.
3. **Handshake timeout.** No `HelloAck` within `handshakeTimeoutMs` closes the socket and reconnects.

Ways to cover these without a fake server, in order of preference:

- **Make the real server produce the condition.** A test-only server config (environment variable, like `MONOPLAN_SNAPSHOT_THRESHOLD_BLOBS` today) that delays or withholds the terminal `UpdatesBatch` for a named doc, or that delays `HelloAck`, would cover 1 and 3 with the real wire. Case 2 can be reached today by letting the test override the protocol version the bridge advertises (a `codec` whose `helloFrame` encodes `supported_protocol_versions: [9999]`), which needs no server change at all.
- **Drive the bridge with a hand-held socket pair.** Bun's `WebSocketPair` is not available, but a bridge whose `socketFactory` returns an object implementing the four handlers the bridge touches (`onopen` / `onmessage` / `onclose` / `send`) is a dozen lines and tests only the router, with no server at all. This is a stub, not a server, and stays inside the test file.

If one of these cases ever bites, add the override first; do not introduce a standalone fake server.

## Helpers

- `TestServer` — RAII handle, starts a server on a random port with a temp sqlite, exposes URL, kills on drop.
- `cli/tests/support/mod.rs` — shared CLI-system harness for server startup, profile materialization, auth helpers, device registration, and bounded polling.
- `wait_for(predicate, timeout)` — bounded polling helper. **Avoid raw `sleep`s in tests.**

## Convergence assertions

`core/` exposes `doc_fingerprint(doc) -> [u8; 32]` — a hash over the **canonical logical state** (items + lists + ordering + bin contents, walked in a deterministic order). Two replicas that have observed the same ops produce equal fingerprints; replicas that diverge produce different ones.

Use the fingerprint for E2E and property-test convergence checks instead of comparing snapshot bytes. Snapshot bytes are **not** required to be stable across replicas — Loro's serialization carries per-replica metadata (peer-ids, internal ordering) that legitimately differs even at logical equality, and per-snapshot encryption nonces would mask byte-stability anyway. Logical-state fingerprinting is the right granularity for "did we converge."

**Future:** a merkle chain/tree over the encrypted update stream is a separate, complementary primitive — commits to *causal history* rather than logical state, enabling O(log n) sync-diff and tamper-evident audit of the server's op log. Out of scope for now.

## Determinism

Where possible, expose a server-side "test event stream" (e.g. `--test-events <unix-socket>`) emitting `op_persisted`, `snapshot_requested`, `snapshot_uploaded`. Tests subscribe and react instead of polling.

## Randomised invariant testing

`core/tests/order_schema.rs` implements the property-test idea without a
proptest dependency (deterministic LCG seeds, no shrinking):

- `randomized_multi_peer_convergence` — N docs, a random mutation stream
  (add / reorder / cross-list move / done / bin / delete / list churn /
  undo / redo / reconcile) interleaved with random pairwise syncs, then a
  full mesh sync. After **every** op it asserts the v2 projection
  invariants (`spec/data-model.md`) through the public API *and* replays
  the emitted `AppEvent`s into a naive consumer mirror (the JS-store
  contract: per-list Open arrays, remove-then-insert at `open_index`) that
  must never drift from the doc. Final fingerprints must converge.
  Default 6 seeds in CI; deepen locally with
  `MONOPLAN_FUZZ_SEEDS=50 cargo test -p monoplan-core --test order_schema --release`.
- `large_synthetic_history_many_peers_lists_moves_undos` — sequential
  fresh-peer sessions booted from accumulated oplog rows doing bulk
  adds, multi-select cross-list moves, undos and captures, then
  save/load and snapshot-bootstrap round-trips (the retired
  move-undo-multipeer repro shape).

What this stress-tests is *our integration* (event translation,
projection index, sync framing) — Loro already promises logical
convergence. Transport-level randomisation (partial syncs through the
real engine/server rather than snapshot exchange) remains open.
