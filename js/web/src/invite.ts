// Sharing links on the receiving side (`spec/sharing-plan.md` §Invites).
//
// A link is `<app>/#invite_<secret>`. The secret must never reach the
// server, the address bar's history, or the router, so it is captured
// at the very start of the page load (`captureInviteFromLocation` in
// `index.tsx`), parked in sessionStorage and stripped from the URL
// before anything renders. Parking it survives the sign-in or sign-up
// the invitee may need first (the app remounts on a session swap) and
// dies with the tab. Once an authenticated session is up,
// `InviteDialog` picks the pending secret up, fetches the wrap behind
// its id, opens it with the secret and joins.

import { InviteSecret, wrapDekWithDek, type Dek } from "@monoplan/core/wasm";
import { api, ApiError, type InvitePayload } from "./api.ts";
import type { Session } from "./Login.tsx";
import { parseInviteHash } from "./url.ts";

const PENDING_KEY = "monoplan.pendingInvite";

/** The slice of `Storage` this module uses; injectable for tests. */
export interface KeyStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** The slice of `location` + `history` this module uses. */
export interface Address {
  hash: string;
  pathname: string;
  search: string;
  replace(url: string): void;
}

function sessionStore(): KeyStore | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

function pageAddress(): Address {
  return {
    hash: location.hash,
    pathname: location.pathname,
    search: location.search,
    replace: (url) => history.replaceState(null, "", url),
  };
}

/** If the page was opened on a sharing link, park its secret and strip
 *  the fragment. Returns true when a link was captured. Must run before
 *  the workspace mounts: the router's first pass would otherwise rewrite
 *  the unknown hash to the restored view and the secret would be lost. */
export function captureInviteFromLocation(
  address: Address = pageAddress(),
  store: KeyStore | null = sessionStore(),
): boolean {
  const secret = parseInviteHash(address.hash);
  if (secret === null) return false;
  try {
    store?.setItem(PENDING_KEY, secret);
  } catch {
    // Private mode / blocked storage: the dialog below still gets one
    // shot this page load through the in-memory fallback.
    memoryPending = secret;
  }
  address.replace(address.pathname + address.search);
  return true;
}

let memoryPending: string | null = null;

/** The parked secret, if any. */
export function pendingInvite(store: KeyStore | null = sessionStore()): string | null {
  try {
    return store?.getItem(PENDING_KEY) ?? memoryPending;
  } catch {
    return memoryPending;
  }
}

export function clearPendingInvite(store: KeyStore | null = sessionStore()): void {
  memoryPending = null;
  try {
    store?.removeItem(PENDING_KEY);
  } catch {
    // nothing to clear
  }
}

export type InviteLookup = { secret: InviteSecret; payload: InvitePayload };

/** Resolve a secret to the wrap behind it. Throws `ApiError` 404 for an
 *  unknown or revoked link and 410 for an expired or used one. */
export async function lookupInvite(secretHex: string): Promise<InviteLookup> {
  const secret = InviteSecret.fromHex(secretHex);
  const payload = await api.fetchInvite(secret.id());
  return { secret, payload };
}

/** Open the wrap with the secret, re-wrap the doc's DEK under the
 *  session's Home DEK and join. An "already a member" answer is a
 *  success here: the caller holds the doc either way. Returns the doc id
 *  and the DEK the caller now needs to boot it. */
export async function acceptInvite(
  session: Session,
  lookup: InviteLookup,
): Promise<{ docId: string; dek: Dek; alreadyMember: boolean }> {
  const { secret, payload } = lookup;
  const dek = secret.unwrap(payload.wrapped_dek, payload.wrapped_dek_nonce);
  const wrapped = wrapDekWithDek(session.dek, dek);
  try {
    const { doc_id } = await api.acceptInvite(secret.id(), {
      wrapped_dek: wrapped.ciphertext,
      wrapped_dek_nonce: wrapped.nonce,
    });
    return { docId: doc_id, dek, alreadyMember: false };
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) {
      return { docId: payload.doc_id, dek, alreadyMember: true };
    }
    throw e;
  }
}
