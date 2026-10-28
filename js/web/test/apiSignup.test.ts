import { afterEach, describe, expect, test } from "bun:test";

import { api } from "../src/api.ts";

// Regression for the anonymous -> account path (spec/auth.md §Anonymous
// → account): `api.signup` must put `primary_doc_id` on the wire when the
// caller asks to adopt the local doc, and leave it out otherwise. The
// field was accepted but silently dropped once, which made every web
// signup from an anonymous session fail with "server did not adopt the
// local doc id at signup" (the server minted a fresh id, as it should
// when the field is absent).

const realFetch = globalThis.fetch;

/** Stub fetch to capture the JSON body of the one call signup makes and
 *  answer with a minimal signup response echoing the requested id. */
function captureSignup(): { body: () => Record<string, unknown> } {
  let captured: Record<string, unknown> | null = null;
  globalThis.fetch = (async (_input: RequestInfo | URL, init?: RequestInit) => {
    captured = JSON.parse(init?.body as string) as Record<string, unknown>;
    const primary = (captured.primary_doc_id as string | undefined) ?? "server-minted";
    return new Response(
      JSON.stringify({
        account_id: "acc",
        device_id: "dev",
        device_token: "tok",
        primary_doc_id: primary,
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  }) as typeof fetch;
  return {
    body: () => {
      if (!captured) throw new Error("signup made no request");
      return captured;
    },
  };
}

const baseArgs = () => ({
  email: "a@example.com",
  master_salt: new Uint8Array(16),
  kdf_params: { m_kib: 8, t: 1, p: 1 },
  auth_secret: new Uint8Array(32),
  wrapped_dek: new Uint8Array(48),
  wrapped_dek_nonce: new Uint8Array(24),
  recovery: {
    recovery_salt: new Uint8Array(16),
    recovery_auth_secret: new Uint8Array(32),
    recovery_wrapped_dek: new Uint8Array(48),
    recovery_wrapped_dek_nonce: new Uint8Array(24),
  },
  device_name: "test",
});

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe("api.signup", () => {
  test("sends the adopted doc id as primary_doc_id", async () => {
    const stub = captureSignup();
    const id = "0199e0a0-0000-7000-8000-000000000001";
    const resp = await api.signup({ ...baseArgs(), primary_doc_id: id });
    expect(stub.body().primary_doc_id).toBe(id);
    expect(resp.primary_doc_id).toBe(id);
  });

  test("omits primary_doc_id when there is nothing to adopt", async () => {
    const stub = captureSignup();
    await api.signup(baseArgs());
    expect("primary_doc_id" in stub.body()).toBe(false);
  });
});
