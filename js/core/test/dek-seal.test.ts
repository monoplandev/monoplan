// The wasm `Dek.seal/open` symmetry that `IdbStorage` relies on for
// encrypt-at-rest of the op log. This file validates the underlying
// crypto primitive in Bun, independent of any browser storage.

import { describe, expect, test } from "bun:test";

import {
  Dek,
  deriveLogin,
  unwrapDek,
  unwrapDekWithDek,
  wrapDek,
  wrapDekWithDek,
} from "../wasm/monoplan_core_web.js";

describe("Dek.seal / Dek.open round-trip", () => {
  test("seal then open recovers the plaintext", () => {
    const dek = Dek.generate();
    const plaintext = new TextEncoder().encode("hello idb");
    const blob = dek.seal(plaintext);
    const back = dek.open(blob);
    expect(new TextDecoder().decode(back)).toBe("hello idb");
  });

  test("opening with the wrong DEK throws", () => {
    const a = Dek.generate();
    const b = Dek.generate();
    const blob = a.seal(new Uint8Array([1, 2, 3]));
    expect(() => b.open(blob)).toThrow();
  });

  test("DEK clone produces a key that opens the original's blobs", () => {
    const a = Dek.generate();
    const b = a.clone();
    const blob = a.seal(new Uint8Array([7, 7, 7]));
    expect(Array.from(b.open(blob))).toEqual([7, 7, 7]);
  });
});

describe("wrapDek / unwrapDek round-trip", () => {
  test("wrap with kek then unwrap with same kek returns the same DEK", () => {
    const salt = new Uint8Array(16).fill(1);
    const derived = deriveLogin("correct horse battery staple", salt, 8, 1, 1);
    const dek = Dek.generate();
    const orig = dek.toHex();
    const wrapped = wrapDek(derived.kek, dek);
    const recovered = unwrapDek(derived.kek, wrapped.ciphertext, wrapped.nonce);
    expect(recovered.toHex()).toBe(orig);
  });
});

describe("wrapDekWithDek / unwrapDekWithDek (Home DEK as account key)", () => {
  test("the Home DEK wraps another doc's DEK and opens it again", () => {
    const home = Dek.generate();
    const other = Dek.generate();
    const wrapped = wrapDekWithDek(home, other);
    expect(wrapped.nonce.length).toBe(24);
    const back = unwrapDekWithDek(home, wrapped.ciphertext, wrapped.nonce);
    expect(back.toHex()).toBe(other.toHex());
  });

  test("a different DEK does not open the wrap", () => {
    const home = Dek.generate();
    const other = Dek.generate();
    const wrapped = wrapDekWithDek(home, other);
    const stranger = Dek.generate();
    expect(() =>
      unwrapDekWithDek(stranger, wrapped.ciphertext, wrapped.nonce),
    ).toThrow();
  });

  test("the primary's KEK wrap is not a Home-DEK wrap (mixing them up throws)", () => {
    const salt = new Uint8Array(16).fill(2);
    const derived = deriveLogin("correct horse battery staple", salt, 8, 1, 1);
    const home = Dek.generate();
    const kekWrap = wrapDek(derived.kek, home);
    expect(() =>
      unwrapDekWithDek(home, kekWrap.ciphertext, kekWrap.nonce),
    ).toThrow();
    // And the right path still works: KEK → Home → other.
    const recoveredHome = unwrapDek(derived.kek, kekWrap.ciphertext, kekWrap.nonce);
    const other = Dek.generate();
    const w = wrapDekWithDek(recoveredHome, other);
    expect(unwrapDekWithDek(home, w.ciphertext, w.nonce).toHex()).toBe(other.toHex());
  });
});
