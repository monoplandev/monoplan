# Encryption

## Keys

- **DEK** (data encryption key) — 32-byte random, generated client-side **at doc creation** (the primary doc's at signup). Encrypts every op + snapshot blob of *that doc*. AEAD: XChaCha20-Poly1305. **The DEK is a doc key, not an account key**: different docs have independent DEKs, and an account holds one wrapped copy per doc it is a member of (`doc_members`, see below). The account row carries password material only.
- **Master** — `Argon2id(password, master_salt)`. One expensive call per session.
- **KEK** (key encryption key) — `HKDF(master, "monoplan/kek/v1")`. Wraps/unwraps the DEK. Client-only.
- **Auth secret** — `HKDF(master, "monoplan/auth/v1")`. Sent to server as login credential. Server stores `SHA-256(auth_secret)`. HKDF is one-way — possessing `auth_secret` reveals neither `master` nor `kek`. See `auth.md` for the full password-handling invariant.
- **Recovery code** — 12 words from the English BIP39 wordlist (2048 words). 128 bits of entropy + 4-bit checksum. Generated client-side at signup.
- **Recovery master** — `Argon2id(recovery_code, recovery_salt)`.
- **Recovery KEK** — `HKDF(recovery_master, "monoplan/recovery_kek/v1")`. Wraps/unwraps the DEK on the recovery path. Client-only.
- **Recovery auth secret** — `HKDF(recovery_master, "monoplan/recovery_auth/v1")`. Proves possession of the recovery code to the server. Server stores `SHA-256(recovery_auth_secret)`. The recovery wrap is **only** released after this proof — same threat model as the password path: don't hand attackers harvestable wrap material in exchange for an email address.

## Wrap-states stored on server

Wraps live on the **membership**, one row per `(doc, account)` in `doc_members` (`storage.md`). Every account has at least its `owner` row for its primary doc, inserted at signup.

| Field (on `doc_members`) | When present |
|---|---|
| `wrapped_dek` | always (password-KEK wrap of this doc's DEK) |
| `recovery_wrapped_dek` | iff the member opted into a recovery code |
| `escrowed_dek` | future optional field (Vault-held key wrap) |

Changing the password re-wraps **every** active membership in one transaction; the server refuses (400) a request whose `new_doc_keys` set does not equal the account's memberships, because a membership left under a KEK nobody holds any more is unrecoverable.

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
3. Client posts `/api/account/login { email, auth_secret }` → server returns `primary_doc_id` and `memberships: [{ doc_id, role, wrapped_dek, wrapped_dek_nonce }]`, one per active membership (length 1 today; clients never special-case that).
4. Client unwraps every membership's DEK with `kek` and stores them keyed by doc id; the primary doc's is the one it boots.

### Recovery

1. Client posts `/api/account/prelogin { email }` → server returns `{ master_salt, recovery_salt }` (recovery_salt only present if account opted in).
2. User enters recovery code; client computes `recovery_master = Argon2id(recovery_code, recovery_salt)` → `recovery_kek`, `recovery_auth_secret`.
3. Client posts `/api/account/recover { email, recovery_auth_secret }` → server verifies hash, returns `{ primary_doc_id, doc_keys: [{ doc_id, wrapped_dek, wrapped_dek_nonce }], recovery_session_token }` — the recovery-KEK wrap of every membership that has one. **The wraps are only released after the auth proof — never just on email.** Endpoint is rate-limited.
4. Client unwraps every doc key locally with `recovery_kek`.
5. User sets new password. Client generates new `master_salt`, derives new `master` → new `kek`, new `auth_secret`. Re-wraps every recovered DEK with new `kek`.
6. Client posts `/api/account/password/reset { recovery_session_token, new_master_salt, new_auth_secret, new_doc_keys, device_name }`. In one transaction the server validates + consumes the token, updates password material, rewrites each membership's `wrapped_dek`, and creates the device row; returns `{ device_id, device_token, primary_doc_id }`. `new_doc_keys` must equal the set returned in step 3, else 400 and the session is **not** consumed (retry within the TTL). A membership with no recovery wrap cannot be re-wrapped — the client never had its DEK — so the server closes it (`removed_at`) rather than leave a wrap nobody can open; today that set is always empty, because signup writes both wraps together.

### Password change (logged in, has DEK in memory)

1. Client generates new `master_salt`, derives new `master` → new `kek`, new `auth_secret`. Re-wraps **every** DEK it holds with new `kek`.
2. Client posts `/api/account/password/change` (authenticated by current device token) with `{ current_auth_secret, new_master_salt, new_auth_secret, new_doc_keys: [{ doc_id, wrapped_dek, wrapped_dek_nonce }] }`. The `current_auth_secret` is required so the server can re-verify the user *now* (defense against a hijacked logged-in session changing the password). `new_doc_keys` must cover exactly the account's active memberships (400 otherwise, nothing written) — a client whose local key set is stale (a membership added since its last login) must log in again first.

## Op encryption

Each op blob is independently encrypted with the DEK + a fresh 24-byte random nonce. Nonce stored alongside ciphertext. No additional authenticated data currently.

## Local key storage

- CLI: `secrets.toml` holds one DEK per doc under `[docs."<doc_id>"]` (`cli.md`); OS keychain is future work. Recovery code never persisted by client.
- Web: the primary doc's DEK, AES-GCM-wrapped under a non-extractable per-origin key in IndexedDB (`js/core/src/storage/dek-vault.ts`). Other memberships are not yet held — the web client opens one doc.

## KDF parameters

Default Argon2id: **`m = 64 MiB, t = 3, p = 1`**. Used for both `master` (password) and `recovery_master` (recovery code).

- **Why 64 MiB.** Below ~32 MiB, commodity GPUs make weak passwords cheap to crack offline against the server-stored wrap. 64 MiB is the convergence point for E2EE consumer apps (Bitwarden, 1Password, ProtonMail all sit in this neighbourhood). Argon2 is the *only* defence for users with weak passwords once the DB leaks — don't shave memory to save 50ms.
- **Why not 256 MiB.** WASM heap pressure. Mobile Safari is mean about big WASM allocations; cheap Android browsers will OOM-kill the worker. 64 MiB sails through everywhere we care about.
- **Performance budget.** ~600–1000ms in WASM on a mid-range phone, ~150–300ms native on laptop. The benchmark bar is the slowest realistic client (WASM in mobile Safari on a ~2019 phone) — anything that breaks 2s there is a UX wound and gets reverted.

**Per-user, server-stored, upgradable.** `kdf_params` lives alongside `master_salt` on the account row and is returned by `/prelogin`. New accounts use the server's current default; existing accounts keep their original params until next password change. This means raising the floor later doesn't break older accounts.

**Password change upgrades params** to the current server default. Re-derivation happens on a path the user already pays for; no extra prompt.
