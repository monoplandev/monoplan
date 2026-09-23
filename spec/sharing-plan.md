# Sharing — design + implementation plan

**Status:** Planned, not implemented. v1 ships single-user only (one primary doc per account).

This document preserves the multi-doc / sharing design discussed during early planning. Pre-release, we made a deliberate scoping decision: introduce the `docs` entity + `accounts.primary_doc_id` pointer now (so the eventual migration is incremental), but defer everything else — wire protocol changes, membership tables, per-doc DEKs, sharing UX — until there is real conviction that shared lists are part of the product.

Read this when:
- Considering enabling sharing as a product direction.
- Designing any change that *would* affect sharing (e.g. snapshot orchestration, key management) — to avoid painting ourselves into a corner.

---

## Why this isn't shipping in v1

The product thesis (see top of `AGENTS.md`) is "single-human-user." Sharing contradicts that thesis. A shared-list-for-couples product (Apple Reminders shared lists, but E2EE) is a genuine market gap, but it's a different product — needing kanban/board UI, real-time presence, conflict UX, mobile apps, invite flows. The protocol/encryption changes documented below are maybe 10% of that work. Doing them now without conviction is spec churn that compounds into a slower-to-ship product. The decision: stay solo, revisit if and when (a) the maintainer or a user actually wants the shared list, or (b) the rest of a shared-tasks product feels worth building.

What the v1 bare-minimum gives us:
- `docs` is a first-class entity with its own id and lifetime.
- `accounts.primary_doc_id` names the doc-of-Home explicitly in code paths.
- Future migration to multi-doc storage is incremental, not a flag-day.

---

## Target design

### Membership model

A doc is a first-class entity (`docs` table). Access is granted by inserting a row into a planned `doc_members` table:

```sql
doc_members(
  doc_id, account_id,
  role,                              -- 'owner' | 'member'
  wrapped_dek, wrapped_dek_nonce,    -- the doc's DEK, wrapped with this member's KEK
  recovery_wrapped_dek, …,           -- iff member opted into recovery
  added_at, removed_at
)
```

Every account has exactly one membership where `role='owner'` and `doc_id = accounts.primary_doc_id`. Shared docs add additional memberships per inviter and invitee. The wrap is **per-membership**, so each member's KEK protects their own copy of the doc's DEK — no symmetric key derivation, no per-member secret leaves the inviter's machine.

Constraints:

- **At least one owner per doc, always.** Removing the last owner is rejected; transferring ownership is a separate operation.
- **A member can leave any doc except their primary.** The primary doc is non-leavable, non-deletable; it's the account's Home.
- **Member removal is forward-only** (see "Known limitations").

**Forward hook already in place:** the Focus lens (`spec/focus.md`) encodes
references as `FocusRef = "<doc_id>:<item_id>"`, with the bare `"<item_id>"` form
meaning the local doc. The parser accepts both forms today; only the emitter is
local-only. When sharing lands, a shared item can be pulled into a member's Focus
by emitting the cross-doc form — no migration, no schema change to the focus
container.

### Per-doc DEK

DEK is generated **at doc creation** (32 bytes random). Encrypts every op + snapshot blob for that doc. Different docs have independent, unrelated DEKs. The DEK wrap is per-membership, on `doc_members`, not on `accounts`. The account row holds password material only.

Server is blind to which DEK encrypted any given blob; `doc_id` on the wire is solely a routing key.

### Sharing flow (sketch — crypto choice deferred)

Two candidates, decision deferred until sharing is built:

**Invite codes (leading candidate).** Inviter generates a 32-byte unlock key locally, wraps the doc's DEK with it, posts the wrap to the server, shares a URL whose fragment carries the unlock key. Fragment never reaches the server, so the wrap on its own is useless. Invitee opens URL, fetches wrap, decrypts with fragment key, signs up or logs in, re-wraps with own KEK, accepts.

**Per-account X25519 public keys.** Each account publishes a public key. Inviter encrypts the doc DEK directly to the invitee's public key. More moving parts (new key material on every account, key rotation story, key directory) but the invite UX is fully one-shot.

#### Invite-code flow (illustrative)

1. **Inviter (existing member):**
   - Generates a 32-byte unlock key locally.
   - Wraps the doc's DEK with the unlock key (XChaCha20-Poly1305, fresh nonce).
   - `POST /api/docs/:doc_id/invites { wrapped_dek, wrapped_dek_nonce, expires_in }` → returns `{ invite_token }`.
   - Shares URL `https://monoplan.example/invite/{token}#unlock={base64_key}` out-of-band (Signal, email, paper).

2. **Invitee:**
   - Opens URL; client extracts `token` and `unlock_key` from fragment.
   - `GET /api/invites/:token` → returns `{ doc_id, wrapped_dek, wrapped_dek_nonce, issued_by: { email_hint } }`. Unauthenticated; bearer is the token + fragment.
   - Decrypts wrap with unlock key → has the doc's DEK.
   - If not logged in: sign up or log in. Client now has invitee's `kek`.
   - Re-wraps DEK with `kek`.
   - `POST /api/invites/:token/accept { wrapped_dek, wrapped_dek_nonce }` → server inserts `doc_members` row, consumes invite, returns `{ doc_id }`.
   - Client opens (or reuses) WS connection, begins subscribing to the new doc.

Server never sees DEK plaintext, never sees unlock key, never sees either party's KEK.

### Ownership transfer

`POST /api/docs/:doc_id/transfer-ownership { to_account_id }` (current owner only): atomic UPDATE setting target's role to `'owner'`. Old owner stays a member unless they also `DELETE .../members/:self` in the same client flow. `accounts.primary_doc_id` is **not** affected — primary doc ≠ owned doc. (You can't transfer ownership of your primary doc; it's not shared.)

### Member removal

- **Owner removes member.** `DELETE /api/docs/:doc_id/members/:account_id`. Sets `removed_at`. Server stops accepting any sync frames for `(doc_id, removed_account)`, excludes the removed account's devices from horizon calc, severs broadcast subscriptions.
- **Member leaves voluntarily.** `DELETE /api/docs/:doc_id/members/:self`. Same effects. Rejected if the member is sole owner — must transfer first.

Local client of the removed/leaving account should drop the local copy (snapshot, oplog, DEK entry). Can't enforce — if they took a backup, they keep it.

### Planned endpoints

```
GET    /api/docs                                          -- list memberships + wraps
POST   /api/docs                                          -- create a doc; body: { wrapped_dek, wrapped_dek_nonce }
DELETE /api/docs/:doc_id                                  -- delete a doc; owner only

POST   /api/docs/:doc_id/invites                          -- issue invite
DELETE /api/docs/:doc_id/invites/:invite_id               -- revoke invite
GET    /api/invites/:token                                -- fetch invite payload (unauth)
POST   /api/invites/:token/accept                         -- authed; body: { wrapped_dek, wrapped_dek_nonce }

POST   /api/docs/:doc_id/transfer-ownership               -- owner only
DELETE /api/docs/:doc_id/members/:account_id              -- remove or leave
```

### Planned schema additions

```sql
CREATE TABLE doc_members (
  doc_id                       BLOB NOT NULL REFERENCES docs(id),
  account_id                   BLOB NOT NULL REFERENCES accounts(id),
  role                         TEXT NOT NULL,
  wrapped_dek                  BLOB NOT NULL,
  wrapped_dek_nonce            BLOB NOT NULL,
  recovery_wrapped_dek         BLOB,
  recovery_wrapped_dek_nonce   BLOB,
  added_at                     INTEGER NOT NULL,
  removed_at                   INTEGER,
  PRIMARY KEY (doc_id, account_id)
);
CREATE INDEX doc_members_account_id_idx ON doc_members (account_id) WHERE removed_at IS NULL;

CREATE TABLE doc_invites (
  token_hash                BLOB PRIMARY KEY,
  doc_id                    BLOB NOT NULL REFERENCES docs(id),
  issued_by_account_id      BLOB NOT NULL REFERENCES accounts(id),
  wrapped_dek               BLOB NOT NULL,
  wrapped_dek_nonce         BLOB NOT NULL,
  expires_at                INTEGER NOT NULL,
  consumed_at               INTEGER
);
CREATE INDEX doc_invites_doc_id_idx ON doc_invites (doc_id);

-- Existing tables also re-key:
--   ops:          (account_id, seq) → (doc_id, seq)
--   snapshots:    account_id        → doc_id
--   account_sequences → doc_sequences
--   devices.last_acked_seq is removed in favour of:
CREATE TABLE device_doc_frontiers (
  device_id        BLOB NOT NULL REFERENCES devices(id),
  doc_id           BLOB NOT NULL REFERENCES docs(id),
  last_acked_seq   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (device_id, doc_id)
);

-- accounts.wrapped_dek + recovery_wrapped_dek + nonces are removed
-- (moved to doc_members).
```

### Wire protocol changes

Bump to **protocol version 2**. Every payload-carrying frame gets `doc_id`:

```
PushOps       { doc_id, ops }
PullOps       { doc_id, since_seq }
Ack           { doc_id, last_acked_seq }
PushSnapshot  { doc_id, up_to_seq, compaction_floor_seq, blob }
PullSnapshot  { doc_id }

OpsAck            { doc_id, assigned_seqs }
OpsBatch          { doc_id, ops, complete }
OpsBroadcast      { doc_id, ops }
SnapshotRequest   { doc_id, up_to_seq, compaction_floor_seq }
Snapshot          { doc_id, up_to_seq, blob }
SnapshotRequired  { doc_id, up_to_seq }
```

WS connection stays `(account, device)`-bound at upgrade. One connection **multiplexes** sync for every doc the account is a member of. Implicit subscription: a doc is "subscribed" the first time any frame for it appears on the connection. Server checks `doc_members(doc_id, account_id)` on every payload frame and rejects if not a current member.

Per-doc seq is dense, gap-free. Per-doc snapshot coordinator state. Broadcast registry re-keyed as `HashMap<DocId, Vec<ConnectionRef>>`.

### Auth flows (memberships shape)

Login / recover / password-change responses carry a `memberships: [{ doc_id, wrapped_dek, wrapped_dek_nonce, ... }]` array. For v1 with bare-minimum denormalisation, the array always has length 1 (just the primary doc). Sharing makes the array length variable.

Signup: server creates `docs` row, `accounts` row, `doc_members` row in one tx (see `spec/storage.md` for insertion order).

Recovery: returns recovery wraps for **every** doc the account is a member of. One recovery code, N unwraps. Same for password change: client re-wraps every DEK with the new KEK and uploads the batch.

---

## Implementation plan (when this is actually built)

Premise at the time of writing: rip-and-replace (no v1 transition window) — pre-release, no production data to preserve. If the codebase has shipped to real users by the time sharing is built, this plan needs a transition-window section added (server speaks both v1 and v2 simultaneously, drop v1 after one release).

### Pre-flight

- Confirm no production deployments need data preserved.
- Tag current `main` so a known-good pre-sharing commit is easy to reference.
- Skim CLI and web local storage to confirm migration paths drop-and-recreate cleanly.
- Convert item `notes` from a plain-string register to a `LoroText` container (see "Text fields must be mergeable before sharing" below; `text` stays a register, per `notes-plan.md`). This is a doc-layout change and is far cheaper before any doc is shared.

### Phase 1 — Protocol types + server schema (foundation)

Files: protocol crate (`PushOps`, `PullOps`, `Ack`, etc.), `server/migrations/001_init.sql`.

- Add `doc_id: DocId` to every payload-carrying frame. Bump `PROTOCOL_VERSION` to 2.
- Replace `migrations/001_init.sql` with the full multi-doc schema. (Drop `002_*` add-on path; rip and replace.)
- Introduce `DocId` newtype (uuid v7).

Compile breaks downstream — fix in Phase 2.

### Phase 2 — Server

**2a — Queries.** `server/src/sync/queries.rs` (~380 lines). Every `account_id` → `doc_id`. New helper `is_member(doc_id, account_id) -> bool`. Frontier reads/writes against `device_doc_frontiers`. Horizon: `min(last_acked_seq WHERE doc_id = ? AND device.account_id IN active_members(doc_id))`.

**2b — Session + WS routing.** `server/src/sync/ws.rs`, `sessions.rs`. `Session` gains `subscribed_docs: HashSet<DocId>`. Per-frame: extract `doc_id`, membership check, insert into subscribed_docs if first frame, dispatch to per-doc handler. Broadcast registry: `HashMap<DocId, Vec<ConnectionRef>>`, fan-out per-doc.

**2c — Snapshot coordinator.** `server/src/sync/snapshot.rs`. State map keyed by `DocId`. Triggers, candidate selection, all log lines tagged with `doc_id`.

**2d — HTTP endpoints.** `server/src/account/*`. Signup: insert docs + accounts + doc_members in one tx. Login response: `{ primary_doc_id, memberships: [...], recovery_present, device_token? }`. Recovery / password-reset / password-change: accept and return memberships arrays.

**2e — Tests.** Update fixture builders. Add a "two docs on one account" test (seed second `doc_members` row directly) — proves the WS routing and broadcast registry actually work multi-doc even before doc-creation endpoints exist.

### Phase 3 — Core (minimal)

- `core/src/sync.rs`: engine signatures gain `doc_id` (stamped on outbound frames). Engine internals unchanged.
- `core/src/crypto/`: no changes. DEK is still 32 bytes; wrap/unwrap unchanged.
- HTTP body construction (if any in core): memberships shape.

### Phase 4 — CLI

- **Local storage** (`cli/migrations/001_init.sql`, `cli/src/db.rs`): replace `doc_snapshot` single-row table with a `docs` table keyed by `doc_id` (snapshot blob, last_acked_seq, DEK).
- **Secrets / config** (`cli/src/config.rs`): `Secrets.dek_hex` → `Secrets.docs: HashMap<DocId, DocSecrets>`. One entry in v1, schema-versioned.
- **Sync integration** (`cli/src/sync.rs`): introduce a `DocRouter` that owns the WS and a `HashMap<DocId, SyncEngine>`. Inbound demuxed by `doc_id`; outbound stamped before send. v1 = always one engine in the map.
- **Auth flows**: signup generates primary-doc DEK locally, wraps with KEK, posts in wrap fields. Login receives memberships array, unwraps each, stores in `Secrets.docs`. Recovery / password change loops over memberships.
- **Tests**: update integration tests to seed new schema.

### Phase 5 — Web

- **IndexedDB schema** (`js/core/src/storage/web-db.ts`): bump IDB version. `onupgradeneeded` drops old stores, creates new with composite keyPaths: `ops[account_id, doc_id, oplog_seq]`, `snapshot_meta[account_id, doc_id]`. Wrap-key store: one entry per `doc_id` (non-extractable WebCrypto AES-GCM wrap).
- **SyncBridge → DocRouter** (`js/core/src/sync-bridge.ts`): bridge owns WS + reconnect; new `DocRouter` holds `Map<DocId, SyncEngine>`; routes inbound/outbound. v1 = primary doc engine only.
- **App boot** (`js/web/src/App.tsx`): on login, unwrap each membership DEK with KEK, persist to IndexedDB. Boot the primary doc (`primary_doc_id`).
- **Auth flows**: memberships-shaped bodies on relevant endpoints.
- **Verification**: signup → add items → reload → still works. Two devices on same account → real-time sync.

### Phase 6 — Cleanup & docs

- Delete orphaned per-account symbols left from v1.
- Update `spec/testing.md` if integration patterns changed.
- Update `spec/cli.md` if commands or config changed visibly.
- Confirm `bun run build:wasm` clean.

### Test-coverage emphasis (write before sharing UI ships)

- **Server: two-doc multiplexing.** Seed second `doc_members` row directly via DB. Open one WS, push to doc A, push to doc B, verify both get correct per-doc seqs and a third connection sees broadcasts only for its subscribed doc.
- **Core: per-engine gap handling independence.** Two engines on same connection; gap on engine A's doc must not block engine B's ack progress.
- **HTTP: memberships array round-trip.** Login returns 1-element array; same code paths handle N. Never special-case length 1.

### Recommended PR cadence

Two-PR split:

1. **PR 1 — Server + protocol + schema.** Phases 1, 2. Server internally consistent, tests pass; no client can talk to it yet.
2. **PR 2 — Clients.** Phases 3, 4, 5 + cleanup. CLI and web brought up together (they share protocol crate).

Or split into 5 small PRs if needed: protocol → server → core+CLI → web → cleanup. Each broken-build-until-next-merges. Pre-release-only acceptable.

### Effort estimate (one focused engineer, at time of design)

| Phase | Days |
|---|---|
| 1 — protocol + schema | 0.5 |
| 2 — server | 2 |
| 3 — core | 0.5 |
| 4 — CLI | 1 |
| 5 — web | 1.5 |
| 6 — cleanup + verification | 0.5 |
| **Total** | **~6 days** |

Server's broadcast registry + snapshot coordinator rewrite is the largest single risk. Multiplexing layer (server router + client `DocRouter`) is conceptually simple but a lot of small wiring.

Plus: sharing UI (invite flow, member management, doc switcher) is **separate from the above** and not estimated here. The above is just the *protocol/storage substrate*. The product on top — invite UX, board view, member list, removal confirmation, etc. — is its own scope.

---

## Text fields must be mergeable before sharing

**Status: not done. Blocking for sharing; strongly recommended before that for multi-device. Superseded for the storage question by `notes-plan.md` (Phase 1), which also decides that `text` stays a register.**

Today an item's `text` and `notes` are plain string values in the item `LoroMap` (`map.insert(KEY_NOTES, notes)` in `core/src/doc.rs`). A map value is a last-writer-wins register over the *whole string*: two concurrent edits to the same item's notes merge by discarding one of them entirely. This already applies to a single user's phone and laptop editing offline; with sharing it becomes two people losing each other's work, which is the one CRDT failure users actually notice.

### Change

| Field | Today | Target |
|---|---|---|
| `text` | string register | string register (unchanged; see `notes-plan.md`, "Why `text` stays a register") |
| `notes` | string register | mergeable `LoroText` child container |

- Character-level merges come for free from Loro; no new crate, no new protocol surface, op blobs stay opaque to the server.
- Pre-release, so rip-and-replace per the premise above: no in-doc migration, no dual-read. The `001_init.sql` rule applies in spirit to the doc layout too. Do it before any doc has more than one member, or a notes migration has to run inside every shared doc.
- `data-model.md` item table gets the `notes` type change. `edit_item_notes` in core kept its signature (takes a full string) and diffed it into the container with `LoroText::update`, so CLI and existing callers were untouched (since removed: `apply_notes_delta` is the only notes write, see `notes-plan.md`). `edit_item_text` is unchanged: a title is a short phrase rewritten whole, and a character merge of two concurrent rewrites interleaves them, so LWW is the better outcome there.
- Diff translation (`id` kept inside the map so a container handle resolves to its item, see data-model.md) already anticipates child containers under an item; text containers slot into the same path.

### What this does not commit to

The editor layers are separable and should be decided separately:

1. **Merge-correct storage** (this section). Editors stay as they are: load `text.to_string()`, write back the full string on change. The web task dialog's plain-text contenteditable and the row quick-entry editor need no changes beyond the store API.
2. **Live co-editing** (remote deltas applied under a live caret, presence cursors). Only needed once two people can edit one item at the same moment. The web editors already preserve caret by character offset across re-renders (`locateOffsetInLinkified`), so applying a remote delta and restoring the caret is feasible on the current contenteditable substrate. Presence is sharing-UI work, not storage work.
3. **Rich text.** A product decision, not a sharing prerequisite. If it ever lands, that is the point to adopt a real editor (ProseMirror/Tiptap or CodeMirror, both have Loro bindings). A `<textarea>` would not get there and does not help merges, so it is not a stepping stone; contenteditable is the right substrate to keep.

### Test emphasis

- Two clients edit different parts of the same item's notes offline, sync, both edits present.
- Two clients edit the same region; result is a character-level merge, not a dropped edit.
- CLI `edit` of notes and web dialog edits interleave on the same item without loss (the CLI ↔ web multi-device proof already exists; extend it to text).

## Known limitations (accepted for first sharing release)

- **Forward-only revocation.** Removing a member or revoking ownership stops the server from giving them new ops, but they retain their local copy of the doc's DEK and any ops they previously synced. They cannot decrypt *future* ops they don't already have, but they *can* decrypt anything they already pulled. True revocation requires DEK rotation (generate fresh DEK, distribute new wraps to every remaining member, encrypt all future ops with new key, optionally re-encrypt history). Distributed coordination problem; not solved here. Document loudly in product UI.

- **No history fence on join.** A new member receives the full doc snapshot + op history from the moment they join. No "shared only going forward" mode.

- **No per-doc role granularity beyond owner/member.** Read-only roles, time-bound access, etc. are future work.

- **Invite tokens are bearer credentials.** Anyone with URL (token + fragment) can accept up to expiry. Mitigation: out-of-band channel hygiene + revocation endpoint.

- **Recovery model.** Recovery returns wraps for every membership; a recovered account regains access to every doc it was a member of.

---

## Open questions

- **Invite codes vs X25519 public keys** — decision deferred to when sharing is built.
- **Doc deletion semantics** when doc has multiple members: unilateral owner delete, member-by-member opt-in, or "leave for everyone else, doc keeps existing for the leaver"? TBD.
- **Discovery / UI** for shared docs: flat list with primary doc privileged at top, or folder-like hierarchy? Product question, out of scope.
- **Connection-per-doc vs multiplexing.** Decided: multiplexing (one WS per device, `doc_id` on every frame). Connection-per-doc was the alternative; rejected because it scales worse with many docs.

---

## Decisions recorded during planning (so future-you knows what was considered)

- **Rip-and-replace** chosen over v1/v2 transition window because pre-release. Revisit if production data exists when this lands.
- **Multiplexing** chosen over connection-per-doc. Reason: scales better with N docs, single reconnect/backoff to manage.
- **Per-doc DEK** chosen over per-member-derived keys. Reason: per-member derivation requires sharing master secrets, which breaks E2EE.
- **Invite codes** chosen as leading-candidate sharing primitive (over X25519 public keys) for simpler UX and no new key material on accounts. Not locked in; reconsider when actually building.
- **Primary doc is privileged** in UI: non-deletable, non-leavable. The account's Home.
- **Member-removal rotation deferred** — forward-only revocation accepted as a v1 limitation.
- **Recovery-code-after-signup deferred** — recovery stays a signup-only opt-in.
