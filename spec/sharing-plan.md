# Sharing — design + implementation plan

**Status:** Planned, not implemented. v1 ships single-user only (one primary doc per account).

This document preserves the multi-doc / sharing design discussed during early planning. Pre-release, we made a deliberate scoping decision: lay the doc-keyed substrate now (so the eventual migration is incremental) but defer everything that needs a second doc to exist — membership tables, per-doc DEKs, multi-doc clients, sharing UX — until there is real conviction that shared lists are part of the product.

Substrate in place as of 2026-10-13: the `docs` entity and `accounts.primary_doc_id`; ops, snapshots, seq counter, snapshot coordinator and broadcast registry keyed by doc; `device_doc_frontiers` for per-(device, doc) acks; and the wire protocol carrying `doc_id` on every payload frame behind an `OpenDoc` / `DocOpened` binding step (`sync-protocol.md` §Doc binding).

Key model done as of 2026-10-17, revised 2026-10-19: **the DEK is a doc key, and the Home DEK is the account key.** `doc_members` holds one wrap per membership and `accounts` holds password material only. The primary membership's wrap is under the password KEK (plus the recovery KEK iff enrolled); every other membership's wrap is under the Home DEK. Login returns a `memberships` array (the client opens the primary with the KEK and the rest with the Home DEK); recovery releases only the primary's recovery wrap; password change / reset take `new_doc_keys` naming exactly the primary doc (set-checked, one transaction, no other row touched). `POST /api/docs` takes just a Home-DEK wrap. `can_open` is the `doc_members` lookup and the compaction horizon spans the devices of every active member. The CLI stores one DEK per doc in `secrets.toml` (every membership unwrapped at login / recovery); the web client unwraps every membership at login but vaults and boots the primary only. Trade-offs are recorded in `encryption.md` §Trade-offs.

Multi-doc transport done as of 2026-10-19 (headless; no UI): a connection multiplexes **any number** of open docs (`WSSession.open` is a map; per-doc subscription and lease slot; the session loop polls every subscription); `POST /api/docs` / `GET /api/docs` create and list an account's own docs; the `Hello` handshake moved out of the engine to the connection owner (core `hello_frame` / `parse_hello_response` / `server_frame_doc_id`); the js `SyncBridge` is the router (N engines, one socket, frames routed by `doc_id`). Proven by `server/tests/sync.rs` and the two-doc `js/core/test/sync-e2e.test.ts`. What is *not* built: invites and any second *account* on a doc, member management, live revocation (`DocClosed`), a CLI router (the CLI still runs one engine on its primary doc), and every client surface (the web client still boots one doc; loading every membership at boot is the decided default, see "Decisions").

Read this when:
- Considering enabling sharing as a product direction.
- Designing any change that *would* affect sharing (e.g. snapshot orchestration, key management) — to avoid painting ourselves into a corner.

---

## Why this isn't shipping in v1

The product thesis (see top of `AGENTS.md`) is "single-human-user." Sharing contradicts that thesis. A shared-list-for-couples product (Apple Reminders shared lists, but E2EE) is a genuine market gap, but it's a different product — needing kanban/board UI, real-time presence, conflict UX, mobile apps, invite flows. The protocol/encryption changes documented below are maybe 10% of that work. Doing them now without conviction is spec churn that compounds into a slower-to-ship product. The decision: stay solo, revisit if and when (a) the maintainer or a user actually wants the shared list, or (b) the rest of a shared-tasks product feels worth building.

What the v1 bare-minimum gives us:
- `docs` is a first-class entity with its own id and lifetime.
- `accounts.primary_doc_id` names the doc-of-Home explicitly in code paths.
- The wire and the server data plane are already doc-addressed, so sharing adds membership and keys without touching frame shapes or re-keying rows.
- Future migration to multi-doc storage is incremental, not a flag-day.

---

## Target design

### Membership model

A doc is a first-class entity (`docs` table). Access is granted by inserting a row into a planned `doc_members` table:

```sql
doc_members(
  doc_id, account_id,
  role,                              -- 'owner' | 'member'
  wrapped_dek, wrapped_dek_nonce,    -- primary membership: Home DEK under the member's KEK; any other: the doc's DEK under the member's Home DEK
  recovery_wrapped_dek, …,           -- primary membership only, iff member opted into recovery
  added_at, removed_at
)
```

Every account has exactly one membership where `role='owner'` and `doc_id = accounts.primary_doc_id`. Shared docs add additional memberships per inviter and invitee. The wrap is **per-membership**, so each member's own key protects their own copy of the doc's DEK — no symmetric key derivation, no per-member secret leaves the inviter's machine. Since 2026-10-19 that key is the member's **Home DEK**, not their password KEK: an invitee who accepts wraps the shared doc's DEK under their Home DEK, exactly as `POST /api/docs` does for a doc they create. Their password KEK and recovery KEK only ever wrap the Home DEK (`encryption.md` §Wrap-states).

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

### Per-doc DEK (built 2026-10-17; Home DEK as account key 2026-10-19)

DEK is generated **at doc creation** (32 bytes random). Encrypts every op + snapshot blob for that doc. Different docs have independent, unrelated DEKs. The DEK wrap is per-membership, on `doc_members`, not on `accounts`. The account row holds password material only. The primary doc's DEK (the Home DEK) is the one key the password and recovery KEKs wrap; every other membership's wrap is under the Home DEK, so password change / reset / recovery touch one row and never need to know about other docs. See `encryption.md` for the flows and trade-offs.

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
   - If not logged in: sign up or log in. Client now holds the invitee's Home DEK.
   - Re-wraps DEK with the **Home DEK** (not the password KEK — the password and recovery KEKs only ever wrap the Home DEK, so an invite accepted today still opens after any later password change or recovery with no re-wrap).
   - `POST /api/invites/:token/accept { wrapped_dek, wrapped_dek_nonce }` → server inserts `doc_members` row, consumes invite, returns `{ doc_id }`.
   - Client opens (or reuses) WS connection, begins subscribing to the new doc.

Server never sees DEK plaintext, never sees unlock key, never sees either party's KEK or Home DEK.

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
POST   /api/invites/:token/accept                         -- authed; body: { wrapped_dek, wrapped_dek_nonce } (a Home-DEK wrap)

POST   /api/docs/:doc_id/transfer-ownership               -- owner only
DELETE /api/docs/:doc_id/members/:account_id              -- remove or leave
```

### Planned schema additions

`doc_members` is built (see `storage.md`); `doc_invites` is not.

```sql
-- Built 2026-10-17 (authoritative copy in storage.md):
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

-- Already done (2026-10-13), no change needed here:
--   ops (doc_id, seq), snapshots.doc_id, doc_sequences,
--   device_doc_frontiers (device_id, doc_id, last_acked_seq).

-- accounts.wrapped_dek + recovery_wrapped_dek + nonces are removed
-- (moved to doc_members). Done 2026-10-17.
```

### Wire protocol changes

**Done (2026-10-13), without a version bump.** Every payload frame carries `doc_id` and a doc is bound to the connection explicitly: `OpenDoc { doc_id }` → `DocOpened { doc_id, epoch }` (or `DocRejected`). The authoritative frame table is `sync-protocol.md`; the earlier sketch here (with `assigned_seqs`, no `from_seq` / `up_to_vv`, no `complete` on `Snapshot`) predates push ids and delta snapshots and is superseded.

Explicit open was chosen over implicit first-frame subscription because the doc's `epoch` needs a per-doc home once a connection spans docs, and because it gives one access-check point and a natural place for leave / revoke later.

WS connection stays `(account, device)`-bound at upgrade. Per-doc seq is dense, gap-free; per-doc snapshot coordinator state; broadcast registry keyed by doc — all in place.

Remaining for sharing:

- ~~`can_open` becomes a `doc_members(doc_id, account_id)` lookup.~~ Done 2026-10-17.
- ~~A connection holds one open doc; multiplexing lifts it to a map keyed by `doc_id`.~~ Done 2026-10-19 (`WSSession.open: HashMap<Uuid, OpenDoc>`, the broadcast arm is a `poll_fn` over every open subscription, leases released per doc on disconnect).
- ~~Horizon spans the devices of every active member.~~ Done 2026-10-17 (`queries::doc_horizon` lost its `account_id` parameter).
- Removal / leave severs the open doc on live connections (a `DocClosed { doc_id, reason }` frame is the obvious shape; not yet defined). Membership is checked at `OpenDoc` only today.

### Auth flows (memberships shape) — built 2026-10-17, narrowed 2026-10-19

Login returns `memberships: [{ doc_id, role, wrapped_dek, wrapped_dek_nonce }]`; the client opens the entry whose `doc_id` is `primary_doc_id` with the password KEK (the Home DEK) and every other entry with the Home DEK. Recover returns `doc_keys: [{ doc_id, wrapped_dek, wrapped_dek_nonce }]` holding exactly the primary's recovery wrap; password change and reset take `new_doc_keys` in the same shape, set-checked against exactly the primary doc id. The server test suite creates a second doc through `POST /api/docs` (and seeds a `member`-role row directly) to prove no path special-cases the memberships length.

Signup: server creates `docs` row, `accounts` row, `doc_members` row, `devices` row in one tx (see `spec/storage.md` for insertion order).

Recovery: one recovery code, one unwrap (the Home DEK), then N Home-DEK unwraps from `GET /api/docs`. Password change: the client re-wraps the Home DEK only; a doc created on another device since this device's last login is unaffected.

---

## Implementation plan (when this is actually built)

Premise at the time of writing: rip-and-replace (no v1 transition window) — pre-release, no production data to preserve. If the codebase has shipped to real users by the time sharing is built, this plan needs a transition-window section added (server speaks both v1 and v2 simultaneously, drop v1 after one release).

### Pre-flight

- Confirm no production deployments need data preserved.
- Tag current `main` so a known-good pre-sharing commit is easy to reference.
- Skim CLI and web local storage to confirm migration paths drop-and-recreate cleanly.
- Convert item `notes` from a plain-string register to a `LoroText` container (see "Text fields must be mergeable before sharing" below; `text` stays a register, per `notes-plan.md`). This is a doc-layout change and is far cheaper before any doc is shared.

### Phase 1 — Protocol types + server schema (foundation)

Files: protocol crate, `server/migrations/001_init.sql`.

- ~~Add `doc_id` to every payload-carrying frame.~~ Done 2026-10-13 (with `OpenDoc` / `DocOpened` / `DocRejected`), no version bump needed pre-release.
- ~~Edit `migrations/001_init.sql` in place: add `doc_members`; move the DEK wraps off `accounts`.~~ Done 2026-10-17. `doc_invites` remains.

Compile breaks downstream — fix in Phase 2.

### Phase 2 — Server

**2a — Queries.** ~~`is_member`, membership-wide `doc_horizon`.~~ Done 2026-10-17 (`auth::queries::is_member`, `list_memberships`; `sync::queries::doc_horizon`).

**2b — Session + WS routing.** ~~`can_open` → membership check; `WSSession.open` → map; broadcast arm polls every open subscription.~~ Done 2026-10-19.

**2c — Snapshot coordinator.** Already keyed by doc; nothing to do.

**2d — HTTP endpoints.** ~~Signup / login / recover / reset / change in memberships shape.~~ Done 2026-10-17. ~~`POST /api/docs`, `GET /api/docs`.~~ Done 2026-10-19 (`server/src/http/doc_routes.rs`). Still to add: invites, members.

**2e — Tests.** ~~Fixture builders; "two docs on one account".~~ Done 2026-10-17 / 2026-10-19 (`server/tests/sync.rs`: two docs interleaved on one connection, broadcast isolation across three connections, per-doc lease release; `server/tests/auth.rs`: created doc covered by list / login / password change / recovery, recovery wrap required on enrolled accounts).

### Phase 3 — Core (minimal)

- `core/src/sync.rs`: ~~engine signatures gain `doc_id` (stamped on outbound frames).~~ Done. ~~The `Hello` handshake moves out of the engine~~ Done 2026-10-19: the engine starts at `OpenDoc`; the host runs `Hello` once per socket with `hello_frame` / `parse_hello_response` and routes inbound frames with `server_frame_doc_id`. `EngineOptions` (client identity) now belongs to the connection, not the engine constructor.
- `core/src/crypto/`: no changes. DEK is still 32 bytes; wrap/unwrap unchanged.
- HTTP body construction (if any in core): memberships shape.

### Phase 4 — CLI

- **Local storage** (`cli/migrations/001_init.sql`, `cli/src/db.rs`): replace `doc_snapshot` single-row table with a `docs` table keyed by `doc_id` (snapshot blob, last_acked_seq, DEK).
- ~~**Secrets / config** (`cli/src/config.rs`): `Secrets.dek_hex` → `Secrets.docs`.~~ Done 2026-10-17 (`BTreeMap<String, DocSecrets>` keyed by doc uuid).
- **Sync integration** (`cli/src/sync.rs`): introduce a `DocRouter` that owns the WS and a `HashMap<DocId, SyncEngine>`. Inbound demuxed by `doc_id`; outbound stamped before send. The handshake step is already the session's (2026-10-19); the router is the remaining piece, plus a doc selector on commands.
- ~~**Auth flows**: login unwraps every membership into `Secrets.docs`; recovery / password change loop over memberships.~~ Done 2026-10-17.
- **Tests**: update integration tests to seed new schema.

### Phase 5 — Web

- **IndexedDB schema** (`js/core/src/storage/web-db.ts`): bump IDB version. `onupgradeneeded` drops old stores, creates new with composite keyPaths: `ops[account_id, doc_id, oplog_seq]`, `snapshot_meta[account_id, doc_id]`. Wrap-key store: one entry per `doc_id` (non-extractable WebCrypto AES-GCM wrap).
- ~~**SyncBridge → DocRouter** (`js/core/src/sync-bridge.ts`).~~ Done 2026-10-19: the bridge takes `engines: SyncEngine[]`, runs the handshake, routes by `doc_id`, reports unroutable frames and handshake failures via `onError`. The web client passes one engine today.
- **App boot** (`js/web/src/App.tsx`): on login, unwrap each membership DEK (Home via KEK, rest via Home DEK) and boot a `Doc` + engine for every one (see "Decisions": clients load every membership). As of 2026-10-19 `Login.tsx` unwraps every membership into `Session.docKeys` and `api.ts` has `createDoc` / `listDocs`, but the session still vaults and boots only the primary doc. Still to do here:
  - **Per-doc vault.** `dek-vault.ts` stores one DEK. Two options: store one entry per doc id, or store the Home DEK alone and re-derive the rest from `GET /api/docs` on boot (every non-primary wrap is a Home-DEK wrap, so the vault never *needs* more than the Home DEK; the trade-off is a network round trip before the other docs open offline). Decide when the doc switcher is built.
  - **Boot every membership.** `runtime.ts` builds one engine; it should build one per entry in `docKeys` (plus whatever the vault / list endpoint yields on reload) and hand them all to `createSyncBridge`.
- ~~**Auth flows**: memberships-shaped bodies on relevant endpoints.~~ Done for login (the web has no recovery / password-change UI yet).
- **Verification**: signup → add items → reload → still works. Two devices on same account → real-time sync.

### Phase 6 — Cleanup & docs

- Delete orphaned per-account symbols left from v1.
- Update `spec/testing.md` if integration patterns changed.
- Update `spec/cli.md` if commands or config changed visibly.
- Confirm `bun run build:wasm` clean.

### Test-coverage emphasis (write before sharing UI ships)

- **Server: two-doc multiplexing.** Seed second `doc_members` row directly via DB. Open one WS, push to doc A, push to doc B, verify both get correct per-doc seqs and a third connection sees broadcasts only for its subscribed doc.
- **Core: per-engine gap handling independence.** Two engines on same connection; a stalled pull on engine A's doc must not block engine B's progress. Not covered (see `testing.md` §Known coverage gaps for why and how to add it).
- **HTTP: memberships array round-trip.** Login returns 1-element array; same code paths handle N. Never special-case length 1.

### Recommended PR cadence

Two-PR split:

1. **PR 1 — Server + protocol + schema.** Phases 1, 2. Server internally consistent, tests pass; no client can talk to it yet.
2. **PR 2 — Clients.** Phases 3, 4, 5 + cleanup. CLI and web brought up together (they share protocol crate).

Or split into 5 small PRs if needed: protocol → server → core+CLI → web → cleanup. Each broken-build-until-next-merges. Pre-release-only acceptable.

### Effort estimate (one focused engineer, at time of design)

| Phase | Days |
|---|---|
| 1 — protocol + schema | 0.5 (done except `doc_invites`) |
| 2 — server | 1.5 (memberships, keys, multi-open, `POST /api/docs` done; invites, member endpoints, `DocClosed` remain) |
| 3 — core | 0.5 (done) |
| 4 — CLI | 0.5 (key storage + handshake done; `DocRouter` + doc selector remain) |
| 5 — web | 1.5 (router done, login unwraps every membership; per-doc vault, boot every membership, doc switcher remain) |
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

- **Recovery model.** Recovery releases the Home DEK, from which every other membership's wrap opens; a recovered account regains access to every doc it was a member of. Conversely, anyone holding an account's Home DEK holds every doc it can open (`encryption.md` §Trade-offs).

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
- **Home DEK as account key** (decided 2026-10-19) over one password/recovery wrap pair per membership. Reason: password change, reset and recovery become single-row operations that never need the client's full key set, a doc created on one device is immediately openable on another after any later password change, and a recovery code enrolled at signup covers every future doc. Accepted costs (recorded in `encryption.md` §Trade-offs): Home DEK compromise exposes every doc; rotating the Home DEK would mean re-wrapping every other membership (no rotation exists today); the primary doc stays non-leavable and non-deletable.
- **Invite codes** chosen as leading-candidate sharing primitive (over X25519 public keys) for simpler UX and no new key material on accounts. Not locked in; reconsider when actually building.
- **Primary doc is privileged** in UI: non-deletable, non-leavable. The account's Home.
- **Member-removal rotation deferred** — forward-only revocation accepted as a v1 limitation.
- **Recovery-code-after-signup deferred** — recovery stays a signup-only opt-in.
- **Clients load every membership at boot** (decided 2026-10-19). A client opens a live `Doc` + engine for each doc the account is a member of, not just the one in view. Reasons: item URLs carry no doc id and are resolved by searching every loaded doc (`urls.md`), and cross-doc Focus refs need the foreign doc resident. A client-side policy to open a subset lazily is a possible later refinement; the server and router must not assume the full set is open on every connection.
