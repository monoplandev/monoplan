# Crypto primitives

Reference inventory of every cryptographic primitive in use, what it does, and where to find it. Companion to `encryption.md` (which describes the *design* — key hierarchy, wraps, flows). This file is the *what* and *why* at the primitive level.

## 1. Argon2id — password-based KDF

**Location:** `core/src/crypto/kdf.rs` (`argon2id`).
**Purpose:** Stretch low-entropy human input (password, recovery code) into 32 bytes of master key material.
**Inputs:** password / recovery code + per-account 16-byte salt + cost params (memory, time, parallelism).
**Output:** `PasswordMaster` / `RecoveryMaster` (32 bytes).

**Why this and not bcrypt/scrypt/PBKDF2:** Memory-hard — defeats GPU/ASIC brute force. Argon2id specifically blends Argon2i (side-channel resistance) with Argon2d (GPU resistance). Won the 2015 Password Hashing Competition; current best practice for password-derived keys.

## 2. HKDF-SHA256 — subkey expansion

**Location:** `core/src/crypto/kdf.rs` (`hkdf_expand`).
**Purpose:** Split one master key into multiple purpose-specific subkeys without correlation between them.
**Inputs:** master (32 bytes) + a domain-separation `info` string (`"monoplan/kek/v1"`, `"monoplan/auth/v1"`, etc.).
**Output:** 32-byte subkey.

**Why it matters:** Critical that `KEK` and `auth_secret` are different bytes — the server gets `auth_secret`, must not be able to derive `KEK`. HKDF guarantees subkeys are computationally independent (one-way: holding any subkey reveals nothing about the master or other subkeys). The versioned `info` string lets us rotate derivation later without breaking old accounts (bump the suffix, keep the old path for migration).

## 3. XChaCha20-Poly1305 — AEAD

**Location:** `core/src/crypto/aead.rs`.
**Purpose:** Authenticated encryption — confidentiality + integrity in one primitive.

Used for two distinct purposes:

- **Home DEK wrap:** `Kek::wrap(&dek)` — encrypt the primary doc's DEK (the Home DEK, which is the account key) under the password-derived KEK at signup / password change. One wrap per `(account, password)`. Optional second wrap per `(account, recovery_code)`.
- **Doc DEK wrap:** `Dek::wrap_dek(&other)` / `Dek::unwrap_dek(&wrapped)` — encrypt any other doc's DEK under the Home DEK (same AEAD, fresh nonce; byte-for-byte `Dek::seal` over the 32 key bytes). One wrap per non-primary membership (`encryption.md` §Wrap-states).
- **Op + snapshot encryption:** `Dek::seal(&plaintext)` — encrypt every CRDT op and snapshot blob before it goes to the server. Fresh random 24-byte nonce per call. Server stores ciphertext + nonce as opaque blobs.

**Why this AEAD:** Software-fast (no AES-NI hardware needed — important for wasm and mobile). 24-byte nonces let us use random nonces safely (collision probability ~2⁻⁹⁶ even after a billion uses). Simple API surface — easy to use right.

**Nonce discipline:** Random per call. Reusing a `(key, nonce)` pair would break confidentiality on structured plaintexts and forge-ability on Poly1305 — never reuse.

## 4. SHA-256 — server-side one-way hash

**Location:** `server/src/auth/tokens.rs` (`sha256`).
**Purpose:** Store proof-of-secret without storing the secret itself.

Two uses:

- **Token storage:** Device tokens and recovery session tokens are 32 random bytes. Server stores `SHA-256(token)`, never the raw token. DB leak yields hashes, not usable credentials.
- **`auth_secret` storage:** Server stores `SHA-256(auth_secret)` as `password_hash`.

**Why SHA-256 and not Argon2 server-side:** The inputs (`auth_secret`, tokens) already have 256 bits of entropy. Argon2's memory-hardness defends against weak/guessable inputs — pointless cost when input is already a uniform-random 32-byte value. No rainbow-table risk because the input space is too large to precompute.

## 5. CSPRNG — `rand::thread_rng`

**Location:** `core/src/crypto/mod.rs` (`random_bytes`), `core/src/crypto/recovery.rs`, `server/src/auth/tokens.rs`.
**Purpose:** Generate every "must be unique" or "must be unguessable" value.

Used for:

- Per-account `master_salt` (16 bytes)
- Per-account `recovery_salt` (16 bytes, optional)
- Per-message AEAD nonces (24 bytes, fresh every wrap/seal)
- Fresh DEKs (32 bytes, generated at signup)
- Device tokens, recovery session tokens (32 bytes)
- BIP39 recovery code entropy (16 bytes → 12 words)

**Why it matters:** Every uniqueness/unpredictability property in the design hangs off this RNG being good. `thread_rng` pulls from the OS CSPRNG (`getrandom` on Linux, `SecRandomCopyBytes` on macOS) — the right source.

## 6. BIP39 — recovery code encoding

**Location:** `core/src/crypto/recovery.rs`.
**Purpose:** Encode 128 bits of entropy as 12 English words (with a 4-bit checksum) so users can write it down without transcription errors.

Same scheme Bitcoin/Ethereum wallets use for seed phrases — Monoplan cribs the format because it's well-tested and the wordlist (BIP39 English) is chosen to minimize ambiguous similar-sounding words. Length and case-tolerance are handled by `parse_recovery_code`.

**Note:** We use the words as a **string** input to Argon2id, not as a derivation seed (which is BIP39's traditional role in wallets). The 128 bits of entropy is what makes the recovery KEK strong; BIP39 is just the user-facing wrapper.

## 7. Constant-time byte comparison

**Location:** Auth route handlers (`constant_time_eq`).
**Purpose:** Compare secret-hash values without leaking byte-position information through timing.

Used for:

- Comparing `SHA-256(presented_token)` against the stored hash on every authenticated request.
- Comparing `SHA-256(auth_secret)` against `password_hash` on login.
- Comparing recovery auth proofs.

**Why:** A naive `==` short-circuits at the first mismatched byte. An attacker who can measure response time can recover a secret one byte at a time. Constant-time eq runs the full comparison every time and accumulates differences via XOR/OR — same work regardless of where (or whether) bytes differ.

## 8. Hex encoding

**Location:** `server/src/auth/tokens.rs` (`encode_token`), `cli/src/keystore.rs` (`dek_to_hex`/`dek_from_hex`).
**Purpose:** Wire/storage format for binary tokens and the on-disk DEK.

**Why hex over base64:** Lowercase hex is universally URL/file/case-safe. Tokens (32 bytes → 64 chars) and the DEK (32 bytes → 64 chars) are short enough that the size cost over base64 doesn't matter.

## How they compose

```
                                                    ┌──────────────┐
password ──Argon2id(salt,params)──▶ PasswordMaster ──HKDF─┤ KEK          │──XChaCha20-Poly1305──▶ wrap(DEK) ─▶ server (opaque)
                                                          │ AuthSecret   │──SHA-256──▶ password_hash ─▶ server
                                                          └──────────────┘

recovery code ──Argon2id(salt,params)──▶ RecoveryMaster ──HKDF─┤ recovery KEK         ├──XChaCha20-Poly1305──▶ recovery_wrap(DEK)
                                                                │ recovery AuthSecret  │──SHA-256──▶ recovery_auth_hash
                                                                └─────────────────────┘

random ──▶ DEK (32 bytes, one per doc; the primary doc's is the Home DEK)
Home DEK ──XChaCha20-Poly1305(fresh nonce)──▶ wrap(other doc's DEK) ─▶ server (opaque, on that doc's membership row)
DEK ──XChaCha20-Poly1305(fresh nonce per blob)──▶ ciphertext for each op + snapshot ─▶ server (opaque)
```

Layered roles:

- **Argon2id** — cost layer. Turn weak human input into strong key material.
- **HKDF** — separation layer. One master serves multiple roles without cross-leakage.
- **XChaCha20-Poly1305** — sealing layer. Confidentiality + integrity for everything we encrypt.
- **SHA-256** — verification layer. Server's "I held this and it was right" check, never the gate that protects secrets.
- **CSPRNG / BIP39 / constant-time eq / hex** — supporting plumbing for uniqueness, human input, comparison, and encoding.

## What's not used (and why)

- **AES-GCM** — would work, but XChaCha20-Poly1305 is faster in pure software and tolerates random nonces better at scale. We pick one and stick with it.
- **bcrypt / scrypt / PBKDF2** — superseded by Argon2id for password-based KDFs.
- **RSA / ECDSA / Ed25519 / Curve25519** — no asymmetric crypto currently. No public-key encryption, signatures, or key exchange. Auth is symmetric (`auth_secret` over TLS). If we add device-to-device handshakes or signed ops in the future, those would land here.
- **Ratchets (Double Ratchet, Signal-style)** — no per-message key rotation. Single long-lived DEK per doc. CRDT semantics don't benefit from ratcheting and the protocol complexity isn't worth it right now.
- **Argon2 server-side** — see SHA-256 reasoning above.
