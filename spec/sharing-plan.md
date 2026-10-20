# Sharing — design + implementation plan

**Status:** Substrate, multi-doc clients and **invites** built; member removal, ownership transfer and live revocation (`DocClosed`) are not. Invites (sharing links) landed 2026-10-21: see §Invites below.

This document preserves the multi-doc / sharing design discussed during early planning. Pre-release, we made a deliberate scoping decision: lay the doc-keyed substrate now (so the eventual migration is incremental) but defer everything that needs a second doc to exist — membership tables, per-doc DEKs, multi-doc clients, sharing UX — until there is real conviction that shared lists are part of the product.

Substrate in place as of 2026-10-13: the `docs` entity and `accounts.primary_doc_id`; ops, snapshots, seq counter, snapshot coordinator and broadcast registry keyed by doc; `device_doc_frontiers` for per-(device, doc) acks; and the wire protocol carrying `doc_id` on every payload frame behind an `OpenDoc` / `DocOpened` binding step (`sync-protocol.md` §Doc binding).

Key model done as of 2026-10-17, revised 2026-10-19: **the DEK is a doc key, and the Home DEK is the account key.** `doc_members` holds one wrap per membership and `accounts` holds password material only. The primary membership's wrap is under the password KEK (plus the recovery KEK iff enrolled); every other membership's wrap is under the Home DEK. Login returns a `memberships` array (the client opens the primary with the KEK and the rest with the Home DEK); recovery releases only the primary's recovery wrap; password change / reset take `new_doc_keys` naming exactly the primary doc (set-checked, one transaction, no other row touched). `POST /api/docs` takes just a Home-DEK wrap. `can_open` is the `doc_members` lookup and the compaction horizon spans the devices of every active member. The CLI stores one DEK per doc in `secrets.toml` (every membership unwrapped at login / recovery); the web client unwraps every membership at login, vaults one entry per doc id, and boots a doc + engine per membership on one multiplexed socket (2026-10-20); the UI still renders the primary only. Trade-offs are recorded in `encryption.md` §Trade-offs.

Multi-doc transport done as of 2026-10-19 (headless; no UI): a connection multiplexes **any number** of open docs (`WSSession.open` is a map; per-doc subscription and lease slot; the session loop polls every subscription); `POST /api/docs` / `GET /api/docs` create and list an account's own docs; the `Hello` handshake moved out of the engine to the connection owner (core `hello_frame` / `parse_hello_response` / `server_frame_doc_id`); the js `SyncBridge` is the router (N engines, one socket, frames routed by `doc_id`). Proven by `server/tests/sync.rs` and the two-doc `js/core/test/sync-e2e.test.ts`. What is *not* built as of 2026-10-21: member removal / leave / ownership transfer, live revocation (`DocClosed`), a CLI router (the CLI still runs one engine on its primary doc), and every client surface (the web client boots every membership headlessly as of 2026-10-20 but renders only the Home doc; see "Decisions").

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

**Forward hook in place (core side done 2026-10-20):** the Focus lens
(`spec/focus.md`) encodes references as `FocusRef = "<doc_id>:<item_id>"`, with
the bare `"<item_id>"` form meaning the local doc. Core keeps foreign refs,
surfaces them as entries and adds them on request; the host resolves and prunes
them. A shared item is pulled into a member's Focus by writing the cross-doc form
into their Home doc — no migration, no schema change to the focus container.

### Per-doc DEK (built 2026-10-17; Home DEK as account key 2026-10-19)

DEK is generated **at doc creation** (32 bytes random). Encrypts every op + snapshot blob for that doc. Different docs have independent, unrelated DEKs. The DEK wrap is per-membership, on `doc_members`, not on `accounts`. The account row holds password material only. The primary doc's DEK (the Home DEK) is the one key the password and recovery KEKs wrap; every other membership's wrap is under the Home DEK, so password change / reset / recovery touch one row and never need to know about other docs. See `encryption.md` for the flows and trade-offs.

Server is blind to which DEK encrypted any given blob; `doc_id` on the wire is solely a routing key.

### Invites (built 2026-10-21)

A sharing link is the whole invite. It carries one 32-byte random **invite secret** in the URL fragment:

```
<app>/#invite_<secret>          secret = [0-9a-f]{64}
```

The client derives two HKDF-SHA256 subkeys from the secret (`core/src/crypto/invite.rs`, `InviteSecret`):

| Subkey | `info` | Who sees it | Purpose |
|---|---|---|---|
| invite id | `monoplan/invite_id/v1` | server (as `doc_invites.id`), every invite call | lookup key; one-way from the secret |
| invite KEK | `monoplan/invite_kek/v1` | client only | the doc's DEK is wrapped under it (XChaCha20-Poly1305, fresh nonce) |

So the server stores an id it cannot invert and a wrap it cannot open; the link holder derives both. One secret in the URL instead of a path token plus a fragment key keeps the whole credential out of access logs and gives the server nothing to correlate. The fragment never leaves the browser (`urls.md`).

**Issuing (owner only).** The web `SharingDialog` (heading menu of a non-Home workspace) or `monoplan share link <doc>` generates the secret, wraps the doc's DEK under the invite KEK and posts `{ invite_id, wrapped_dek, wrapped_dek_nonce, expires_in_secs? }` to `POST /api/docs/:doc_id/invites`. The server checks for an active `owner` row (404 otherwise, so a non-member learns nothing), refuses the account's **primary doc with 403** before any lookup (the Home DEK is the account key: sharing Home would hand over every doc the account can open, `encryption.md` §Trade-offs), clamps the lifetime to [60 s, 30 days] with a 7-day default, and inserts `doc_invites`. The link is shown once; the server cannot rebuild it. `GET /api/docs/:doc_id/invites` lists what was issued (owner only) and `DELETE /api/docs/:doc_id/invites/:invite_id` revokes by deleting the row.

**Accepting.** The invitee opens the link. The web client captures the secret at the very start of the page load (`js/web/src/invite.ts`, called from `index.tsx`), parks it in `sessionStorage` and strips the fragment before the router runs, so it is never mirrored into history. `GET /api/invites/:invite_id` is unauthenticated (holding the id is the credential) and returns `{ doc_id, wrapped_dek, wrapped_dek_nonce, expires_at, inviter_hint }`, the hint being the inviter's masked email (`d…@example.com`). The `InviteDialog` shows who invited them; an anonymous session is sent through the normal sign-in / sign-up dialog first and the parked secret survives the session swap. On Join the client opens the wrap with the invite KEK, re-wraps the DEK under its **Home DEK** (`wrapDekWithDek`, the same wrap `POST /api/docs` takes, so password change / reset / recovery never need to know about it) and posts it to `POST /api/invites/:invite_id/accept`. In one transaction the server validates the invite, inserts (or, for a previously removed member, reactivates) the `member` row with that wrap and stamps `consumed_at` / `consumed_by_account_id`, so two holders of the same link cannot both join: the second gets 410. The response names the doc; the client boots it like any non-Home doc first seen on a device (`runtime.joinDoc`: empty `Doc`, the server's snapshot fills it on `OpenDoc`), vaults the key and navigates to its Inbox. The CLI equivalent is `monoplan share accept <link>`, which stores the DEK in `secrets.toml`.

**Status codes.** Unknown or revoked id: 404. Expired or already used: 410 (`gone`; the message says which). The caller is already an active member (the owner opening their own link): 409, and the invite is **not** consumed, so it still reaches the person it was meant for. Members of a doc can read `GET /api/docs/:doc_id/members` (`{ account_id, email, role, added_at }`); non-members get 404.

**Why not X25519 public keys.** The alternative considered was per-account public keys with the inviter encrypting the DEK to the invitee directly. Rejected for now: it adds key material and a rotation story to every account and a key directory the server would have to serve honestly, for a UX that is only one-shot if the inviter already knows the invitee's account. The link needs none of that and works for someone with no account yet. Revisit if sharing between existing accounts becomes the common case.

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
DELETE /api/docs/:doc_id                                  -- delete a doc; owner only, never the primary (403), refused while another member is active (409) — built 2026-10-20

POST   /api/docs/:doc_id/invites                          -- issue a link (owner only; 403 on the primary doc) — built 2026-10-21
GET    /api/docs/:doc_id/invites                          -- list issued links (owner only) — built 2026-10-21
DELETE /api/docs/:doc_id/invites/:invite_id               -- revoke — built 2026-10-21
GET    /api/invites/:invite_id                            -- fetch invite payload (unauth; 404 / 410) — built 2026-10-21
POST   /api/invites/:invite_id/accept                     -- authed; body: { wrapped_dek, wrapped_dek_nonce } (a Home-DEK wrap); 409 if already a member — built 2026-10-21
GET    /api/docs/:doc_id/members                          -- list active members (any member) — built 2026-10-21

POST   /api/docs/:doc_id/transfer-ownership               -- owner only
DELETE /api/docs/:doc_id/members/:account_id              -- remove or leave
```

### Planned schema additions

`doc_members` and `doc_invites` are both built (authoritative copies in `storage.md`).

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

-- Built 2026-10-21 (authoritative copy in storage.md):
CREATE TABLE doc_invites (
  id                        BLOB PRIMARY KEY,   -- HKDF(secret, "monoplan/invite_id/v1")
  doc_id                    BLOB NOT NULL REFERENCES docs(id),
  issued_by_account_id      BLOB NOT NULL REFERENCES accounts(id),
  wrapped_dek               BLOB NOT NULL,      -- doc DEK under the invite KEK
  wrapped_dek_nonce         BLOB NOT NULL,
  created_at                INTEGER NOT NULL,
  expires_at                INTEGER NOT NULL,
  consumed_at               INTEGER,            -- NULL = open; single use
  consumed_by_account_id    BLOB REFERENCES accounts(id)
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
- Skim CLI and web local storage to confirm migration paths drop-and-recreate cleanly.
- Item `notes` are already a mergeable `LoroText` child container (`notes-plan.md` Phase 1, done 2026-09-08; `text` stays a register on purpose), so concurrent edits merge character-wise and no doc-layout change is needed before a doc is shared. Rich / structured text is `notes-plan.md` Phases 3-4 and is not a sharing prerequisite.

### Phase 1 — Protocol types + server schema (foundation)

Files: protocol crate, `server/migrations/001_init.sql`.

- ~~Add `doc_id` to every payload-carrying frame.~~ Done 2026-10-13 (with `OpenDoc` / `DocOpened` / `DocRejected`), no version bump needed pre-release.
- ~~Edit `migrations/001_init.sql` in place: add `doc_members`; move the DEK wraps off `accounts`.~~ Done 2026-10-17. ~~`doc_invites`.~~ Done 2026-10-21.

Compile breaks downstream — fix in Phase 2.

### Phase 2 — Server

**2a — Queries.** ~~`is_member`, membership-wide `doc_horizon`.~~ Done 2026-10-17 (`auth::queries::is_member`, `list_memberships`; `sync::queries::doc_horizon`).

**2b — Session + WS routing.** ~~`can_open` → membership check; `WSSession.open` → map; broadcast arm polls every open subscription.~~ Done 2026-10-19.

**2c — Snapshot coordinator.** Already keyed by doc; nothing to do.

**2d — HTTP endpoints.** ~~Signup / login / recover / reset / change in memberships shape.~~ Done 2026-10-17. ~~`POST /api/docs`, `GET /api/docs`.~~ Done 2026-10-19 (`server/src/http/doc_routes.rs`). ~~Invites, member list.~~ Done 2026-10-21 (`server/src/http/invite_routes.rs`, `server/src/auth/invites.rs`; `server/tests/auth.rs` covers issue → anonymous fetch → accept → listed membership, single use, expiry, owner-only, 403 on Home, 409 for an existing member, revoke). Still to add: member removal / leave, ownership transfer.

**2e — Tests.** ~~Fixture builders; "two docs on one account".~~ Done 2026-10-17 / 2026-10-19 (`server/tests/sync.rs`: two docs interleaved on one connection, broadcast isolation across three connections, per-doc lease release; `server/tests/auth.rs`: created doc covered by list / login / password change / recovery, recovery wrap required on enrolled accounts).

### Phase 3 — Core (minimal)

- `core/src/sync.rs`: ~~engine signatures gain `doc_id` (stamped on outbound frames).~~ Done. ~~The `Hello` handshake moves out of the engine~~ Done 2026-10-19: the engine starts at `OpenDoc`; the host runs `Hello` once per socket with `hello_frame` / `parse_hello_response` and routes inbound frames with `server_frame_doc_id`. `EngineOptions` (client identity) now belongs to the connection, not the engine constructor.
- `core/src/crypto/`: no changes. DEK is still 32 bytes; wrap/unwrap unchanged.
- HTTP body construction (if any in core): memberships shape.

### Phase 4 — CLI

- **Local storage** (`cli/migrations/001_init.sql`, `cli/src/db.rs`): replace `doc_snapshot` single-row table with a `docs` table keyed by `doc_id` (snapshot blob, last_acked_seq, DEK).
- ~~**Secrets / config** (`cli/src/config.rs`): `Secrets.dek_hex` → `Secrets.docs`.~~ Done 2026-10-17 (`BTreeMap<String, DocSecrets>` keyed by doc uuid).
- **Sync integration** (`cli/src/sync.rs`): introduce a `DocRouter` that owns the WS and a `HashMap<DocId, SyncEngine>`. Inbound demuxed by `doc_id`; outbound stamped before send. The handshake step is already the session's (2026-10-19); the router is the remaining piece, plus a doc selector on commands. `monoplan share` (link / pending / revoke / accept / members, 2026-10-21) manages links and stores a joined doc's DEK in `secrets.toml`, but the CLI still syncs the Home doc only.
- ~~**Auth flows**: login unwraps every membership into `Secrets.docs`; recovery / password change loop over memberships.~~ Done 2026-10-17.
- **Tests**: update integration tests to seed new schema.

### Phase 5 — Web

- **IndexedDB schema** (`js/core/src/storage/web-db.ts`): bump IDB version. `onupgradeneeded` drops old stores, creates new with composite keyPaths: `ops[account_id, doc_id, oplog_seq]`, `snapshot_meta[account_id, doc_id]`. Wrap-key store: one entry per `doc_id` (non-extractable WebCrypto AES-GCM wrap).
- ~~**SyncBridge → DocRouter** (`js/core/src/sync-bridge.ts`).~~ Done 2026-10-19: the bridge takes `engines: SyncEngine[]`, runs the handshake, routes by `doc_id`, reports unroutable frames and handshake failures via `onError`. The web client passes one engine today.
- **App boot** (`js/web/src/App.tsx`): on login, unwrap each membership DEK (Home via KEK, rest via Home DEK) and boot a `Doc` + engine for every one (see "Decisions": clients load every membership). As of 2026-10-19 `Login.tsx` unwraps every membership into `Session.docKeys` and `api.ts` has `createDoc` / `listDocs`, but the session still vaults and boots only the primary doc. Still to do here:
  - ~~**Per-doc vault.**~~ Done 2026-10-20: the vault record holds one AES-GCM entry per doc id under its single non-extractable wrapping key (`sealSession` / `openRecord` in `dek-vault.ts`, tested in `js/core/test/dek-vault.test.ts`); `DekVault.putDocKey` adds a doc that appears after login. Chosen over vaulting the Home DEK alone and re-deriving from `GET /api/docs`, so every membership opens offline on a cold reload. A reload now restores the full `Session.docKeys`.
  - ~~**`docId` on `ItemView`.**~~ Done 2026-10-20: the web store stamps every projected `ItemView` with its engine's doc id (snapshot and `itemAdded` paths; drafts take the current app's). Never on the wire. The cross-doc lenses merge by it and route mutations back to the owning `DocApp` by it.
  - ~~**Core keeps foreign Focus refs.**~~ Done 2026-10-20 (`spec/focus.md` §FocusRef grammar): the Home doc's core keeps `<doc_id>:<item_id>` refs through every sweep, surfaces them via `focus_entries()` / `focusEntriesJson`, writes one on `add_foreign_to_focus`, and leaves judging them dead to the host, which prunes by item id. No "resident docs" declaration was needed: a client that has not loaded the other doc simply never prunes.
  - ~~**Boot every membership.**~~ Done 2026-10-20: `BootGate` (`App.tsx`) replays one `IdbStorage` + `Doc` per entry in `Session.docKeys` (Home first; a non-Home doc first seen on a device starts empty and takes the server snapshot), and `runtime.ts` builds one `DocRuntime` (engine, synced store, op-log persistence rhythm) per doc and hands every engine to one `createSyncBridge`, which routes server frames to the owning runtime's pulse. Boot also calls `GET /api/docs` (raced against the vaulted docs' replay, 5 s cap) and unwraps, vaults and boots any membership the vault lacks (`sync/docKeys.ts`), so a doc created on another device appears on the next boot; memberships the server no longer lists are not dropped (that is `DocClosed` work).
  - ~~**Personal layer, create-doc, per-doc nav, merged lenses.**~~ Done 2026-10-20 (`js/web/src/sync/docs.ts`, `createDocsLayer`): the doc map (reactive, Home first), the `itemId → docId` reverse index (resolved by probing each doc's store), `routed: ItemActions` (every item-keyed `DocApp` mutation, grouped by doc; a call spanning docs commits once per doc) and the merged reads. `Workspace` renders a list view from its own doc's `DocApp` (`ViewKey.docId`, absent ≡ Home; `runtime.setView` normalises) and the lenses from the layer. `runtime.createDoc(name)`: fresh DEK wrapped under the Home DEK → `POST /api/docs` → `DekVault.putDocKey` → `Doc.create` (seeded like Home, same peer) → a `DocRuntime` under the runtime's owner → `SyncBridge.addEngine` (opens the doc on the live socket) → `setDocName`. The nav renders one collapsible `DocSection` per doc (own heading, Inbox, lists `Dnd`, new-list form; collapse state per doc in localStorage; rename via the heading's context menu) and, for authed sessions, a "New workspace" item in the footer app menu that opens a name dialog (`NewDocDialog`; no nav row). Search fans out over every doc's index and merge-ranks; list results carry their doc. Covered by `js/web/test/docs.test.ts` (routing, reverse index, foreign Focus refs + prune, Done / calendar prefs, per-doc undo) and the `addEngine` e2e in `js/core/test/sync-e2e.test.ts`; not yet exercised in a browser as of 2026-10-20.
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
| 2 — server | 1.5 (memberships, keys, multi-open, `POST /api/docs`, invites, member list done; removal / transfer, `DocClosed` remain) |
| 3 — core | 0.5 (done) |
| 4 — CLI | 0.5 (key storage, handshake, `share` done; `DocRouter` + doc selector remain) |
| 5 — web | 1.5 (done: router, per-doc vault, boot-every-membership, `listDocs` refresh, personal layer, create-doc UI, per-doc nav sections, merged lenses, sharing dialog + invite accept) |
| 6 — cleanup + verification | 0.5 |
| **Total** | **~6 days** |

Server's broadcast registry + snapshot coordinator rewrite is the largest single risk. Multiplexing layer (server router + client `DocRouter`) is conceptually simple but a lot of small wiring.

Plus: the rest of the sharing UI (member removal, presence, conflict UX) is **separate from the above** and not estimated here; the invite flow and member list shipped 2026-10-21. The above is just the *protocol/storage substrate*. The product on top — invite UX, board view, member list, removal confirmation, etc. — is its own scope.

---

## Known limitations (accepted for first sharing release)

- **Forward-only revocation.** Removing a member or revoking ownership stops the server from giving them new ops, but they retain their local copy of the doc's DEK and any ops they previously synced. They cannot decrypt *future* ops they don't already have, but they *can* decrypt anything they already pulled. True revocation requires DEK rotation (generate fresh DEK, distribute new wraps to every remaining member, encrypt all future ops with new key, optionally re-encrypt history). Distributed coordination problem; not solved here. Document loudly in product UI.

- **No history fence on join.** A new member receives the full doc snapshot + op history from the moment they join. No "shared only going forward" mode.

- **No per-doc role granularity beyond owner/member.** Read-only roles, time-bound access, etc. are future work.

- **Invite links are bearer credentials.** Anyone holding the URL can join until it expires (7 days by default, 30 at most) or is used (single use) or revoked. Mitigation: out-of-band channel hygiene, the short default lifetime, single use, and the revoke endpoint. The web client strips the fragment before the router or history can see it.

- **Recovery model.** Recovery releases the Home DEK, from which every other membership's wrap opens; a recovered account regains access to every doc it was a member of. Conversely, anyone holding an account's Home DEK holds every doc it can open (`encryption.md` §Trade-offs).

---

## Open questions

- **Doc deletion semantics** when doc has multiple members: unilateral owner delete, member-by-member opt-in, or "leave for everyone else, doc keeps existing for the leaver"? TBD. Until decided, `DELETE /api/docs/:doc_id` (built 2026-10-20) requires the caller to be the sole active member and answers 409 otherwise; the primary doc is refused with 403 before any lookup (`accounts.primary_doc_id` on the server, `DocEntry.home` / `Session.primaryDocId` on the client). The web client offers Delete only on non-Home headings (the "..." menu, confirmed), then drops the engine from the bridge (`SyncBridge.removeEngine`), the doc from the layer (`DocsLayer.removeDoc`, which also drops its foreign Focus pins), the key from the session and vault (`DekVault.removeDocKey`) and the local op log (`IdbStorage.wipe`). Other devices drop a vaulted doc the server no longer lists at their next boot (`refreshDocKeys` in `App.tsx`), since its `OpenDoc` would be refused and a refusal ends the session; a device with the doc open *right now* is not told — that is the `DocClosed` frame.
- **Discovery / UI** for shared docs: flat list with primary doc privileged at top, or folder-like hierarchy? Product question, out of scope.
- **Connection-per-doc vs multiplexing.** Decided: multiplexing (one WS per device, `doc_id` on every frame). Connection-per-doc was the alternative; rejected because it scales worse with many docs.

---

## Decisions recorded during planning (so future-you knows what was considered)

- **Rip-and-replace** chosen over v1/v2 transition window because pre-release. Revisit if production data exists when this lands.
- **Multiplexing** chosen over connection-per-doc. Reason: scales better with N docs, single reconnect/backoff to manage.
- **Per-doc DEK** chosen over per-member-derived keys. Reason: per-member derivation requires sharing master secrets, which breaks E2EE.
- **Home DEK as account key** (decided 2026-10-19) over one password/recovery wrap pair per membership. Reason: password change, reset and recovery become single-row operations that never need the client's full key set, a doc created on one device is immediately openable on another after any later password change, and a recovery code enrolled at signup covers every future doc. Accepted costs (recorded in `encryption.md` §Trade-offs): Home DEK compromise exposes every doc; rotating the Home DEK would mean re-wrapping every other membership (no rotation exists today); the primary doc stays non-leavable and non-deletable.
- **Sharing links** (decided and built 2026-10-21) over X25519 public keys: simpler UX, works for invitees without an account, no new key material on accounts. Refinement over the original sketch: one fragment secret with two HKDF subkeys (server lookup id, wrap key) instead of a path token plus a fragment key, so the credential never appears in a URL path or access log and the server stores only a one-way id. Single use and owner-only by default; never for the primary doc.
- **Primary doc is privileged** in UI: non-deletable, non-leavable. The account's Home.
- **Member-removal rotation deferred** — forward-only revocation accepted as a v1 limitation.
- **Recovery-code-after-signup deferred** — recovery stays a signup-only opt-in.
- **Web: one `DocApp` per doc plus a personal layer, not a merged store** (decided 2026-10-20). Everything under `DocApp` is per doc and cannot merge (core `Doc`, UndoManager, search index, notes delta bridge, event dispatcher, `IdbStorage`, engine), and docs come and go at runtime once sharing exists. So the web keeps `createSyncedApp` per doc (`runtime.ts` already builds one per booted doc and returns `docApps`) and adds a thin layer above: the doc map, an `itemId → docId` reverse index (item ids are uuid-v7, globally unique), mutations routed by `ItemView.docId`, and merged read views for the cross-doc lenses. Undo stays per doc; search fans out and merge-ranks.
- **Cross-doc lens requirements** (stated 2026-10-20): the calendar merges every doc's items with per-doc on/off toggles; Focus is personal only, lives in the Home doc's `focus` container, and is populated from any doc via foreign refs (core side done, see `focus.md`); the Done view is personal by default and can be amalgamated across docs. Personal cross-doc preferences (calendar toggles, Done amalgamation) belong in the Home doc's `settings` map, which syncs across the account's devices and is never shared.
- **Every doc is seeded like Home** (decided 2026-10-20): `Doc.create` gives a non-Home doc its own Inbox and Events lists; there is no Home-only Inbox.
- **"Personal Done" is the Home doc's closed items** (decided 2026-10-20 when the merge was built). The `done_by` alternative (items this account completed in any doc) needs a client-written account id on the lifecycle transition; revisit before any doc has two members if "what I finished" turns out to matter more than "my own list". The amalgamate switch (`done_amalgamate` on the Home settings map) shows every doc's. The **Bin always merges** every doc: a binned row must stay reachable to restore or delete, and "personal" carries no meaning there; "Empty bin" empties every doc's.
- **Doc names live in the doc** (`settings.doc_name`, decided 2026-10-20): content, encrypted, seen by every member; the server stays dumb. Home is never named — clients label it "Personal". URLs: a user list's id is unique across docs so `#list_<id>` carries no doc; another doc's Inbox is `#inbox_<doc_id>` (`urls.md`).
- **Foreign Focus refs are pruned on observation** (web, 2026-10-20): the layer drops a `<doc>:<item>` ref once it *sees* the item closed or binned in its doc (an effect, plus inline on the routed close / bin paths), never when the item is merely missing (the doc may not have synced). The prune is an ordinary Home commit, so undoing the close in the other doc does not restore the pin — the same as local auto-remove-on-Done. A foreign row ticked in Focus gets no linger (only Home's store records a Focus slot).
- **Undo interleaves docs chronologically** (web, 2026-10-20): every entry on a `DocApp`'s undo / redo stack carries a process-wide counter stamp (recorded when the action lands, or when it was undone); `docs.undo()` undoes the doc whose top undo entry is newest, `redo()` the doc whose top redo entry is newest, so across docs Cmd+Z walks one history. Undo itself stays per doc: a routed batch spanning docs is undone one doc at a time, and a new action in one doc clears only that doc's redo stack.
- **Cross-doc move is not offered** (web, 2026-10-20, open question above still stands): the move palette and a nav drop act in one doc — the doc of the rows being moved (first row's doc for a mixed lens selection); rows of other docs stay put. The task surface's list picker only ever offers its item's doc.
- **Open: cross-doc move.** A move between docs is a create in the target and a bin / delete in the source, two commits across two undo managers with no atomicity. Whether the item id is preserved is undecided; if it is, URL resolution must prefer the live copy over a binned one.
- **Clients load every membership at boot** (decided 2026-10-19). A client opens a live `Doc` + engine for each doc the account is a member of, not just the one in view. Reasons: item URLs carry no doc id and are resolved by searching every loaded doc (`urls.md`), and cross-doc Focus refs need the foreign doc resident. A client-side policy to open a subset lazily is a possible later refinement; the server and router must not assume the full set is open on every connection.
