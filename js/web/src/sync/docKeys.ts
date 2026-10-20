// Learning about docs after login. A device vaults the memberships it
// unwrapped at login (`dek-vault.ts`, one entry per doc id); a doc the
// account created or joined on another device since then is unknown to
// it until it asks. `GET /api/docs` returns every current membership;
// this module turns the ones the device lacks into DEKs. Every
// non-primary wrap is under the Home DEK (spec/encryption.md
// §Wrap-states), so the Home DEK alone opens all of them.

import { unwrapDekWithDek, type Dek } from "@monoplan/core/wasm";
import type { Membership } from "../api.ts";

/** The memberships in `memberships` whose doc id is neither the primary
 *  nor in `known`, unwrapped under `home`. The primary's wrap is under
 *  the password KEK, which this device does not hold after login, so it
 *  is never re-derived here (it is always in `known` anyway). Throws if
 *  a wrap fails to open: the server returned a wrap under a key this
 *  account does not hold, which is a server bug, not a recoverable
 *  state. */
export function unwrapNewMemberships(
  home: Dek,
  primaryDocId: string,
  memberships: readonly Membership[],
  known: ReadonlySet<string> | ReadonlyMap<string, unknown>,
): Map<string, Dek> {
  const out = new Map<string, Dek>();
  for (const m of memberships) {
    if (m.doc_id === primaryDocId || known.has(m.doc_id) || out.has(m.doc_id)) continue;
    out.set(m.doc_id, unwrapDekWithDek(home, m.wrapped_dek, m.wrapped_dek_nonce));
  }
  return out;
}
