# Encryption

## Keys

- **DEK** (data encryption key) — 32-byte random, generated client-side **at doc creation** (the primary doc's at signup). Encrypts every op + snapshot blob of *that doc*. AEAD: XChaCha20-Poly1305. Different docs have independent DEKs, and an account holds one wrapped copy per doc it is a member of (`doc_members`, see below). The account row carries password material only.
- **Home DEK** — the primary (Home) doc's DEK, which doubles as the **account key** (decided 2026-10-19). It is the only DEK the password KEK and recovery KEK ever wrap. Every other doc's DEK is wrapped under the Home DEK (XChaCha20-Poly1305, fresh nonce), so holding the Home DEK is holding every doc the account can open. See "Trade-offs" below.
- **Master** — `Argon2id(password, master_salt)`. One expensive call per session.
- **KEK** (key encryption key) — `HKDF(master, "monoplan/kek/v1")`. Wraps/unwraps the DEK. Client-only.
- **Auth secret** — `HKDF(master, "monoplan/auth/v1")`. Sent to server as login credential. Server stores `SHA-256(auth_secret)`. HKDF is one-way — possessing `auth_secret` reveals neither `master` nor `kek`. See `auth.md` for the full password-handling invariant.
- **Recovery code** — 12 words from the English BIP39 wordlist (2048 words). 128 bits of entropy + 4-bit checksum. Generated client-side at signup.
- **Recovery master** — `Argon2id(recovery_code, recovery_salt)`.
- **Recovery KEK** — `HKDF(recovery_master, "monoplan/recovery_kek/v1")`. Wraps/unwraps the DEK on the recovery path. Client-only.
- **Recovery auth secret** — `HKDF(recovery_master, "monoplan/recovery_auth/v1")`. Proves possession of the recovery code to the server. Server stores `SHA-256(recovery_auth_secret)`. The recovery wrap is **only** released after this proof — same threat model as the password path: don't hand attackers harvestable wrap material in exchange for an email address.

## Wrap-states stored on server

Wraps live on the **membership**, one row per `(doc, account)` in `doc_members` (`storage.md`). Every account has at least its `owner` row for its primary doc, inserted at signup. The columns are the same on every row; **what `wrapped_dek` is wrapped under depends on whether the row is the account's primary membership** (`doc_id = accounts.primary_doc_id`).

| Field (on `doc_members`) | Primary membership | Every other membership |
|---|---|---|
| `wrapped_dek` / `wrapped_dek_nonce` | always: the Home DEK under the **password KEK** | always: this doc's DEK under the **Home DEK** |
| `recovery_wrapped_dek` / `_nonce` | iff the member opted into a recovery code: the Home DEK under the recovery KEK | always NULL |
| `escrowed_dek` | future optional field (Vault-held wrap of the Home DEK) | n/a |

Consequences:

- Changing the password re-wraps **only the Home DEK**. `new_doc_keys` on `/password/change` and `/password/reset` must name exactly the primary doc id (400 otherwise, nothing written). Non-primary wraps are under the Home DEK, which the password change does not touch.
- Recovery releases only the primary membership's recovery wrap; once the client holds the Home DEK it opens every other membership's wrap itself.
- A non-primary membership never needs a recovery wrap, so `POST /api/docs` (and, later, accepting an invite) takes just `{ wrapped_dek, wrapped_dek_nonce }`: the doc's DEK under the caller's Home DEK.
- The server does not verify which key a wrap is under (it cannot); the rule is a client contract, and the primary-doc id is what tells a client which key to try.

## Recovery tiers

- **Password only** — only `wrapped_dek`. Lose password → lose data. Power-user mode.
- **Password + recovery code** — both wraps. Lose password → redeem recovery code → unwrap DEK → set new password.
- **Password + server escrow** — adds Vault-held escrow. Future work.

User picks tier at signup; can upgrade later. Default UX presents recovery code as recommended.

## Flows

### Signup

1. User picks password and (optionally) generates recovery code (client-side).
2. Client generates `master_salt` (16 bytes random) and (if recovery code) `recovery_salt`.
3. Client computes `master = Argon2id(password, master_salt)` → `kek`, `auth_secret` via HKDF.
4. (If recovery code) Client computes `recovery_master = Argon2id(recovery_code, recovery_salt)` → `recovery_kek`, `recovery_auth_secret` via HKDF.
5. Client generates DEK (32 bytes random).
6. Client wraps DEK with `kek` and (if present) `recovery_kek`.
7. Client posts `/api/account/signup` with `email`, `auth_secret`, `master_salt`, `wrapped_dek`, `wrapped_dek_nonce`, optional `{ recovery_salt, recovery_auth_secret, recovery_wrapped_dek, recovery_wrapped_dek_nonce }`. Both wraps are of the **primary doc's** DEK; the server mints the doc id.
8. Server stores `password_hash = SHA-256(auth_secret)`, `recovery_auth_hash = SHA-256(recovery_auth_secret)` (if present) and the salts on `accounts`, and the wraps on the account's `owner` membership of the new primary doc (`doc_members`). Response carries `primary_doc_id`.

### Login (two-step)

1. Client posts `/api/account/prelogin { email }` → server returns `master_salt` (and `recovery_salt` if present, mostly for symmetry).
2. Client computes `master` → `kek`, `auth_secret`.
3. Client posts `/api/account/login { email, auth_secret }` → server returns `primary_doc_id` and `memberships: [{ doc_id, role, wrapped_dek, wrapped_dek_nonce }]`, one per active membership (clients never special-case the length).
4. Client unwraps the membership whose `doc_id == primary_doc_id` with `kek` → the Home DEK. It then unwraps every other membership's `wrapped_dek` with the Home DEK, and stores the DEKs keyed by doc id.

### Doc creation (logged in)

1. Client generates the new doc's DEK (32 bytes random) and wraps it under the Home DEK (fresh nonce).
2. Client posts `/api/docs { wrapped_dek, wrapped_dek_nonce }` (device-token auth) → server mints the doc id and inserts an `owner` membership carrying the wrap; returns `{ doc_id }`.
3. The doc's history starts with the first `PushOps` after `OpenDoc`. Nothing else is written: password change, reset and recovery never touch this row.

### Recovery

1. Client posts `/api/account/prelogin { email }` → server returns `{ master_salt, recovery_salt }` (recovery_salt only present if account opted in).
2. User enters recovery code; client computes `recovery_master = Argon2id(recovery_code, recovery_salt)` → `recovery_kek`, `recovery_auth_secret`.
3. Client posts `/api/account/recover { email, recovery_auth_secret }` → server verifies hash, returns `{ primary_doc_id, doc_keys: [{ doc_id, wrapped_dek, wrapped_dek_nonce }], recovery_session_token }` — `doc_keys` holds exactly one entry, the primary membership's recovery-KEK wrap of the Home DEK. **The wrap is only released after the auth proof — never just on email.** Endpoint is rate-limited.
4. Client unwraps the Home DEK locally with `recovery_kek`.
5. User sets new password. Client generates new `master_salt`, derives new `master` → new `kek`, new `auth_secret`. Re-wraps the Home DEK with new `kek`.
6. Client posts `/api/account/password/reset { recovery_session_token, new_master_salt, new_auth_secret, new_doc_keys, device_name }`. In one transaction the server validates + consumes the token, updates password material, rewrites the primary membership's `wrapped_dek`, and creates the device row; returns `{ device_id, device_token, primary_doc_id }`. `new_doc_keys` must name exactly the primary doc, else 400 and the session is **not** consumed (retry within the TTL). No other membership is touched: their wraps are under the Home DEK, which the client has just recovered.
7. Client fetches `GET /api/docs` with the new device token and unwraps every non-primary membership with the Home DEK, exactly as after login.

### Password change (logged in, has the Home DEK in memory)

1. Client generates new `master_salt`, derives new `master` → new `kek`, new `auth_secret`. Re-wraps the **Home DEK** with new `kek`.
2. Client posts `/api/account/password/change` (authenticated by current device token) with `{ current_auth_secret, new_master_salt, new_auth_secret, new_doc_keys: [{ doc_id: <primary>, wrapped_dek, wrapped_dek_nonce }] }`. The `current_auth_secret` is required so the server can re-verify the user *now* (defense against a hijacked logged-in session changing the password). `new_doc_keys` must name exactly the primary doc (400 otherwise, nothing written). Other docs are unaffected, so a client whose local key set is stale (a doc created on another device since its last login) can still change the password.

## Trade-offs of the Home DEK as account key

Recorded with the 2026-10-19 decision; none of these is mitigated today.

- **Blast radius.** Compromise of the Home DEK (a leaked `secrets.toml`, a dumped browser vault) exposes every doc the account is a member of, including docs shared with it later. Under the previous model each doc's wrap was independent, but every wrap sat under the same password KEK, so the practical exposure of a device compromise was already the whole account; what changes is that one 32-byte secret is now sufficient on its own.
- **No rotation.** Rotating the Home DEK would mean re-encrypting the primary doc's history *and* re-wrapping every other membership under the new Home DEK. No DEK rotation exists today for any doc, so this adds no new gap, but it raises the cost of ever adding one.
- **The primary doc is load-bearing.** It remains non-leavable and non-deletable (`sharing-plan.md`); losing the primary membership would orphan every other wrap. Ownership transfer of the primary doc stays off the table for the same reason.
- **One password/recovery wrap pair.** The benefit: password change, reset and recovery each touch exactly one row, a doc created on device A is openable on device B after a password change on device B with no re-wrap, and a recovery code enrolled at signup covers docs created years later.

## Op encryption

Each op blob is independently encrypted with the DEK + a fresh 24-byte random nonce. Nonce stored alongside ciphertext. No additional authenticated data currently.

## Local key storage

- CLI: `secrets.toml` holds one DEK per doc under `[docs."<doc_id>"]` (`cli.md`), unwrapped at login / recovery (Home via the KEK, the rest via the Home DEK); OS keychain is future work. Recovery code never persisted by client.
- Web: every membership's DEK, each AES-GCM-wrapped under one non-extractable per-origin key in IndexedDB, one entry per doc id (`js/core/src/storage/dek-vault.ts`). Login unwraps every membership and vaults them all, so a reloaded session holds every doc without a network round trip. Vaulting the Home DEK alone and re-deriving the rest from `GET /api/docs` was considered and rejected (2026-10-20) because it makes non-primary docs unavailable offline on a cold reload.

## KDF parameters

Default Argon2id: **`m = 64 MiB, t = 3, p = 1`**. Used for both `master` (password) and `recovery_master` (recovery code).

- **Why 64 MiB.** Below ~32 MiB, commodity GPUs make weak passwords cheap to crack offline against the server-stored wrap. 64 MiB is the convergence point for E2EE consumer apps (Bitwarden, 1Password, ProtonMail all sit in this neighbourhood). Argon2 is the *only* defence for users with weak passwords once the DB leaks — don't shave memory to save 50ms.
- **Why not 256 MiB.** WASM heap pressure. Mobile Safari is mean about big WASM allocations; cheap Android browsers will OOM-kill the worker. 64 MiB sails through everywhere we care about.
- **Performance budget.** ~600–1000ms in WASM on a mid-range phone, ~150–300ms native on laptop. The benchmark bar is the slowest realistic client (WASM in mobile Safari on a ~2019 phone) — anything that breaks 2s there is a UX wound and gets reverted.

**Per-user, server-stored, upgradable.** `kdf_params` lives alongside `master_salt` on the account row and is returned by `/prelogin`. New accounts use the server's current default; existing accounts keep their original params until next password change. This means raising the floor later doesn't break older accounts.

**Password change upgrades params** to the current server default. Re-derivation happens on a path the user already pays for; no extra prompt.
