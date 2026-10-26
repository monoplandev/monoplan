// Recovery-code enrolment and redemption for the web client: the key
// work around `/api/account/signup`, `/recover` and `/password/reset`
// (spec/encryption.md §Recovery), kept free of HTTP so it is testable
// without a server. Mirrors cli/src/commands/signup.rs and recover.rs.
// The code itself is handed to the UI once for display and is never
// persisted on the client.

import {
  Dek,
  deriveRecovery,
  generateRecoveryCode,
  parseRecoveryCode,
  unwrapDek,
  wrapDek,
} from "@monoplan/core/wasm";
import type { DocKey, KdfParams, RecoveryMaterial } from "../api.ts";

export interface RecoveryEnrolment {
  /** The 12-word phrase, canonical spacing. Show once, then drop. */
  code: string;
  /** What signup ships so the server can gate and store the wrap. */
  material: RecoveryMaterial;
}

/** Mint a fresh code and the material that lets it open `home` later:
 *  a random salt, the auth secret the server hashes, and the Home DEK
 *  under the recovery KEK. Argon2id runs here (hundreds of ms). */
export function enrolRecovery(home: Dek, kdf: KdfParams): RecoveryEnrolment {
  const code = generateRecoveryCode();
  const salt = randomBytes(16);
  const derived = deriveRecovery(code, salt, kdf.m_kib, kdf.t, kdf.p);
  const wrapped = wrapDek(derived.recoveryKek, home);
  return {
    code,
    material: {
      recovery_salt: salt,
      recovery_auth_secret: derived.recoveryAuthSecret,
      recovery_wrapped_dek: wrapped.ciphertext,
      recovery_wrapped_dek_nonce: wrapped.nonce,
    },
  };
}

/** Canonical form of a typed phrase (whitespace and case tolerant).
 *  Throws on an unknown word or a bad checksum. */
export function normaliseRecoveryCode(input: string): string {
  return parseRecoveryCode(input);
}

/** Open the Home DEK from a `/api/account/recover` response with the
 *  recovery KEK. The response carries exactly one key, the primary
 *  membership's recovery wrap; anything else is a server bug. */
export function openRecoveryWrap(
  recoveryKek: Uint8Array,
  primaryDocId: string,
  docKeys: readonly DocKey[],
): Dek {
  const key = docKeys.find((k) => k.doc_id === primaryDocId);
  if (!key) {
    throw new Error("recovery response has no key for the primary doc");
  }
  return unwrapDek(recoveryKek, key.wrapped_dek, key.wrapped_dek_nonce);
}

export function randomBytes(n: number): Uint8Array {
  const out = new Uint8Array(n);
  crypto.getRandomValues(out);
  return out;
}
