import { describe, expect, test } from "bun:test";

import { Dek, deriveRecovery } from "@monoplan/core/wasm";
import {
  enrolRecovery,
  normaliseRecoveryCode,
  openRecoveryWrap,
} from "../src/sync/recovery.ts";

// Test-weak Argon2id so the suite stays quick; the real params are
// exercised in Rust.
const KDF = { m_kib: 8, t: 1, p: 1 };

describe("recovery enrolment", () => {
  test("the enrolled code reopens the Home DEK via the recover response", () => {
    const home = Dek.generate();
    const { code, material } = enrolRecovery(home, KDF);
    expect(code.split(" ")).toHaveLength(12);

    // What the server hands back after the auth proof: the primary
    // membership's recovery wrap, as stored from signup.
    const response = [
      {
        doc_id: "home",
        wrapped_dek: material.recovery_wrapped_dek,
        wrapped_dek_nonce: material.recovery_wrapped_dek_nonce,
      },
    ];
    const typed = `  ${code.toUpperCase().replace(/ /g, "\n")} `;
    const derived = deriveRecovery(
      normaliseRecoveryCode(typed),
      material.recovery_salt,
      KDF.m_kib,
      KDF.t,
      KDF.p,
    );
    expect(Array.from(derived.recoveryAuthSecret)).toEqual(
      Array.from(material.recovery_auth_secret),
    );
    const opened = openRecoveryWrap(derived.recoveryKek, "home", response);
    expect(opened.toHex()).toBe(home.toHex());
  });

  test("a wrong code neither proves nor opens", () => {
    const home = Dek.generate();
    const { material } = enrolRecovery(home, KDF);
    const other = enrolRecovery(Dek.generate(), KDF);
    const derived = deriveRecovery(
      other.code,
      material.recovery_salt,
      KDF.m_kib,
      KDF.t,
      KDF.p,
    );
    expect(Array.from(derived.recoveryAuthSecret)).not.toEqual(
      Array.from(material.recovery_auth_secret),
    );
    expect(() =>
      openRecoveryWrap(derived.recoveryKek, "home", [
        {
          doc_id: "home",
          wrapped_dek: material.recovery_wrapped_dek,
          wrapped_dek_nonce: material.recovery_wrapped_dek_nonce,
        },
      ]),
    ).toThrow();
  });

  test("rejects a phrase that is not BIP39", () => {
    expect(() => normaliseRecoveryCode("not a real phrase at all")).toThrow();
  });

  test("refuses a response without the primary doc's key", () => {
    const home = Dek.generate();
    const { code, material } = enrolRecovery(home, KDF);
    const derived = deriveRecovery(code, material.recovery_salt, KDF.m_kib, KDF.t, KDF.p);
    expect(() => openRecoveryWrap(derived.recoveryKek, "home", [])).toThrow(
      /primary doc/,
    );
  });
});
