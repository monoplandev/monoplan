import { describe, expect, test } from "bun:test";

import { Dek, wrapDekWithDek } from "@monoplan/core/wasm";
import { unwrapNewMemberships } from "../src/sync/docKeys.ts";
import type { Membership } from "../src/api.ts";

const member = (home: Dek, docId: string, dek: Dek): Membership => {
  const w = wrapDekWithDek(home, dek);
  return {
    doc_id: docId,
    role: "owner",
    wrapped_dek: w.ciphertext,
    wrapped_dek_nonce: w.nonce,
  };
};

describe("unwrapNewMemberships", () => {
  test("unwraps only the docs the device does not already hold", () => {
    const home = Dek.generate();
    const known = Dek.generate();
    const fresh = Dek.generate();
    const memberships: Membership[] = [
      // The primary's wrap is under the KEK; it must be skipped unread.
      { doc_id: "home", role: "owner", wrapped_dek: new Uint8Array(3), wrapped_dek_nonce: new Uint8Array(3) },
      member(home, "known", known),
      member(home, "fresh", fresh),
    ];
    const out = unwrapNewMemberships(home, "home", memberships, new Map([["home", home], ["known", known]]));
    expect([...out.keys()]).toEqual(["fresh"]);
    expect(out.get("fresh")!.toHex()).toBe(fresh.toHex());
  });

  test("accepts a Set of known ids and dedups repeated memberships", () => {
    const home = Dek.generate();
    const fresh = Dek.generate();
    const m = member(home, "fresh", fresh);
    const out = unwrapNewMemberships(home, "home", [m, m], new Set(["home"]));
    expect(out.size).toBe(1);
  });

  test("a wrap under a different key throws rather than yielding a bad DEK", () => {
    const home = Dek.generate();
    const other = Dek.generate();
    const m = member(other, "fresh", Dek.generate());
    expect(() => unwrapNewMemberships(home, "home", [m], new Set(["home"]))).toThrow();
  });
});
