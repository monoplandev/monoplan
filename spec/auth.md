# Auth

## HTTP endpoints

```
POST   /api/account/signup
POST   /api/account/prelogin           (returns salt(s) for an email)
POST   /api/account/login              (password path)
POST   /api/account/recover            (recovery-code path; gated by recovery_auth_secret)
POST   /api/account/logout             (authed; revokes calling device + clears cookie)
POST   /api/account/password/change    (change password, authed by device token + current_auth_secret)
POST   /api/account/password/reset     (set new password, authed by recovery_session_token)
GET    /api/devices                    (list current account's devices: name, last_seen_at, last_seen_ip, last_acked_seq)
POST   /api/devices                    (register a new device)
PATCH  /api/devices/:device_id         (rename a device)
DELETE /api/devices/:device_id
```

WebSocket upgrade: `GET /api/sync` with `Authorization: Bearer <device_token>` (CLI) or the `monoplan_device` cookie (web). Server validates on upgrade; the connection is bound to `(account_id, device_id)` for its lifetime. No per-message auth. (Frame encoding + version handshake: see `sync-protocol.md`.)

All HTTP request and response bodies are MessagePack-encoded (`Content-Type: application/msgpack`). Same encoding as the WS path; one wire format across the system.

## Token model

- Opaque random 32-byte token, hex-encoded. Stored server-side as `auth_token_hash` (SHA-256 is sufficient — the token is already high-entropy).
- Issued per-device on registration. Forever-lived. Revocable via `DELETE /api/devices/:id` (or `POST /api/account/logout` for the calling device).
- Every authed request (HTTP and WS upgrade) bumps the device's `last_seen_at` and overwrites `last_seen_ip` with the client address: leftmost `X-Forwarded-For` (Caddy sets it), else `X-Real-IP`, else the TCP peer. One IP per device, no history; it is deleted with the row on revoke. Surfaced only to the account owner via `GET /api/devices` so they can spot a device they do not recognise.
- Same token, two transports: CLI sends `Authorization: Bearer <token>`; web receives an HttpOnly cookie and never touches the token from JS.

Forever tokens in both threat models. A compromised CLI host hands the attacker the DEK from keychain anyway; a browser owned by XSS already drives the live session and can re-derive everything from local crypto state. Splitting into short access + long refresh shrinks the XSS exposure window but doesn't change what an active payload can reach. Single tier for now; revisit when there's a re-auth boundary that crypto state actually depends on.

### Web (cookie transport)

- Token-issuing endpoints (`signup`, `login`, `password/reset`, `POST /devices`) attach `Set-Cookie: monoplan_device=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=34560000` alongside the response body. Body keeps the token because the CLI consumes it; web ignores the body's token field and trusts the cookie. `Max-Age` is 400 days — the browser-honoured maximum (Chrome/Firefox/Safari clamp longer values). The token is forever-lived, so the cookie should persist as long as browsers allow; a *session* cookie (no `Max-Age`) is discarded by mobile Safari/WebKit on backgrounded-tab memory reclaim, silently de-authing the client while the server session is still valid.
- `DeviceAuth` extractor and the WS upgrade try `Authorization: Bearer` first, fall back to the `monoplan_device` cookie. CLI is unaffected.
- `POST /api/account/logout` (authed): revokes the calling device's token server-side and emits `Set-Cookie: monoplan_device=; Max-Age=0`. `DELETE /api/devices/:id` retains its existing semantics (revoke any device by id) and does not touch cookies.
- Web bundle and API must share registrable domain (`SameSite=Strict` permits same-site cross-origin, e.g. `app.monoplan.io` → `api.monoplan.io`). Self-hosted instances serve their own bundle from their own domain; we do not support one web bundle pointed at arbitrary remote APIs.
- CSRF mitigation: `SameSite=Strict`. No double-submit token currently.

## Account model

- One account per email.
- Password is the master key for E2EE. The server **never** sees it in any form from which the KEK could be derived.
- **Email verification is not part of the self-hosted flow.** Email is just an account identifier; self-hosters must be able to run without an SMTP dependency. A future SaaS deployment can layer verification on top — the core auth flow stays identical, with signup/login gated on a `verified_at` column populated by a verification round-trip.

## Password handling — never sent in plaintext

**Invariant: the raw password never leaves the client, ever.** A server that sees the password (request body, log line, heap dump, malicious admin) has the salt too and can derive the KEK → unwrap DEK → decrypt everything. Any path that lets plaintext password reach the server breaks E2EE entirely. TLS does not protect against this — TLS terminates at the server, which then has plaintext in memory.

Client-side derivation (one Argon2id pass, two HKDF splits):

```
master       = Argon2id(password, master_salt)
kek          = HKDF(master, info = "monoplan/kek/v1")
auth_secret  = HKDF(master, info = "monoplan/auth/v1")
```

- `kek` is used to wrap/unwrap the DEK. **Never leaves the client.**
- `auth_secret` is sent to the server as the login credential. HKDF is one-way, so possession of `auth_secret` reveals neither `master` nor `kek`.

Server-side:

- Stores `password_hash = SHA-256(auth_secret)` and `password_salt = master_salt`.
- On login: client posts `auth_secret`; server SHA-256s and compares. SHA-256 is enough server-side because `auth_secret` already has Argon2id cost baked in; no rainbow-table risk.
- A DB leak yields `password_hash`, not `auth_secret` → not directly usable to log in.

## Password rules

Minimum length **10 characters**, no composition rules. Client-enforced at every entry point (signup, password change, recovery reset) — the server never sees the password, so it cannot enforce this; the password is the KEK and weak input means weak crypto.

## Login is two-step

Client cannot derive `auth_secret` without the salt. Standard pattern:

1. `POST /api/account/prelogin { email }` → server returns `{ password_salt, kdf_params }`, or **404 if email unknown**. We accept that this leaks account existence: signup (duplicate-email rejection) and recovery already leak the same bit, so faking a deterministic dummy salt here would be theatre while costing every typo'd-email attempt a full client-side Argon2id pass. Defence is rate-limiting (per-IP + per-email backoff), not response shaping. Revisit only if Monoplan ever holds data where "is X a user" is itself sensitive.
2. Client computes `master`, `kek`, `auth_secret` from the password and salt.
3. `POST /api/account/login { email, auth_secret }` → server verifies, returns `{ wrapped_dek, wrapped_dek_nonce, recovery_present, device_token? }` (device_token only if registering this client as a device in the same call; otherwise client follows up with `POST /api/devices`).
4. Client unwraps DEK with `kek`.

## Device pairing

### Device 1 (signup)
1. POST `/api/account/signup` with all the encryption material.
2. Server creates account + initial device row, returns `device_token`.

### Device 2 (existing account)
1. POST `/api/account/prelogin { email }` → returns `password_salt`.
2. Client derives `master` → `kek`, `auth_secret` from password + salt.
3. POST `/api/account/login { email, auth_secret }` → returns `wrapped_dek`.
4. Client unwraps DEK with `kek`.
5. POST `/api/devices` → returns new `device_token`.
6. WS connect, `PullSnapshot` if present, then `PullOps`.

### Recovery (lost password / fresh device)
1. POST `/api/account/prelogin { email }` → `{ master_salt, recovery_salt }`.
2. Client computes `recovery_master = Argon2id(recovery_code, recovery_salt)` → `recovery_kek`, `recovery_auth_secret`.
3. POST `/api/account/recover { email, recovery_auth_secret }` → server verifies `SHA-256(recovery_auth_secret) == recovery_auth_hash`, returns `{ recovery_wrapped_dek, recovery_wrapped_dek_nonce, recovery_session_token }`. **Wrap is never released without this proof.** `recovery_session_token` is single-use and TTL 15 min — long enough to type a strong new password without rushing, short enough that an abandoned tab doesn't leave the reset window open.
4. Client unwraps DEK locally with `recovery_kek`.
5. Client picks new password, derives new `master_salt`, `master`, `kek`, `auth_secret`; re-wraps DEK.
6. POST `/api/account/password/reset { recovery_session_token, new_master_salt, new_auth_secret, new_wrapped_dek, new_wrapped_dek_nonce, device_name }` → atomically updates password material + creates device row + invalidates session token, returns `{ device_id, device_token }`.
7. WS connect, `PullSnapshot` if present, then `PullOps`.

## Rate limiting

All three credential endpoints (`/prelogin`, `/login`, `/recover`) rate-limit per-IP and per-email with exponential backoff. `/recover` gets the strictest limits — it gates the recovery wrap, so a successful brute-force is account takeover, not just a login. `/prelogin` is the most lenient (legitimate clients hit it on every login).
