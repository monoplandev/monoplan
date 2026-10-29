// Per-doc member directory: account id → email for every shared doc the
// session has open, so item attribution (`ItemView.createdBy`, an
// account id) can render as a person. Loaded lazily from
// `GET /api/docs/:id/members` once per doc per session, then topped up
// from the server's `Presence` frames (`spec/sync-protocol.md`
// §Presence): a member who accepts an invite after this session loaded
// the list opens the doc and is announced with their email, so their
// first items resolve without a refetch. A doc's list otherwise changes
// rarely (members removed) and a stale entry only means a removed member
// still resolves to their email, which is the right thing to show on
// items they created. Home is never loaded: attribution is not shown
// there (`spec/sharing-plan.md` §Attribution).

import { createSignal, untrack } from "solid-js";
import { api, type Member } from "../api.ts";

export interface MemberDirectory {
  /** Email of `accountId` as a member of `docId`, or undefined while the
   *  doc's list is not loaded or when the account is not in it (a
   *  member removed before this session loaded the list). Reactive: a
   *  read inside a memo re-runs when the list lands. */
  emailOf(docId: string, accountId: string): string | undefined;
  /** Number of active members of `docId`, or undefined while its list is
   *  not loaded: the count in the nav's shared-marker tooltip. Reactive. */
  countOf(docId: string): number | undefined;
  /** Fetch the doc's member list if this session has not yet. A failed
   *  fetch is forgotten, so the next `ensure` retries. */
  ensure(docId: string): void;
  /** Fetch (or re-fetch) the doc's member list. Resolves after the
   *  directory is updated; a failure leaves the previous list. */
  refresh(docId: string): Promise<void>;
  /** Add accounts the server announced as present on `docId` (account
   *  id + email). Entries are merged into whatever is known; the fetched
   *  list, when it lands, replaces them with the same facts. Entries
   *  without an email are skipped. Does not count as a load: `ensure`
   *  still fetches and `countOf` stays undefined until it does. */
  learn(docId: string, accounts: ReadonlyArray<{ accountId: string; email: string }>): void;
}

type FetchMembers = (docId: string) => Promise<Member[]>;

const defaultFetch: FetchMembers = async (docId) => (await api.listMembers(docId)).members;

export function createMemberDirectory(fetchMembers: FetchMembers = defaultFetch): MemberDirectory {
  const [byDoc, setByDoc] = createSignal<Record<string, Record<string, string>>>({});
  // Docs whose list has been fetched at least once. Separate from
  // `byDoc` so a presence seed landing before the fetch neither skips
  // the fetch nor reads as a (partial) member count.
  const [loaded, setLoaded] = createSignal<ReadonlySet<string>>(new Set());
  const inflight = new Set<string>();

  const refresh = async (docId: string): Promise<void> => {
    if (inflight.has(docId)) return;
    inflight.add(docId);
    try {
      const list = await fetchMembers(docId);
      const emails: Record<string, string> = {};
      for (const mb of list) emails[mb.account_id] = mb.email;
      setByDoc((prev) => ({ ...prev, [docId]: emails }));
      setLoaded((prev) => (prev.has(docId) ? prev : new Set(prev).add(docId)));
    } catch (e) {
      console.warn("members: load failed:", e);
    } finally {
      inflight.delete(docId);
    }
  };

  return {
    emailOf(docId, accountId) {
      return byDoc()[docId]?.[accountId];
    },
    countOf(docId) {
      if (!loaded().has(docId)) return undefined;
      return Object.keys(byDoc()[docId] ?? {}).length;
    },
    ensure(docId) {
      // Untracked on purpose: `ensure` runs inside effects, and reading
      // the signal here would make a load re-trigger the effect.
      if (!untrack(loaded).has(docId)) void refresh(docId);
    },
    refresh,
    learn(docId, accounts) {
      const known = untrack(byDoc)[docId];
      let next: Record<string, string> | undefined;
      for (const { accountId, email } of accounts) {
        if (email === "" || known?.[accountId] === email) continue;
        next ??= { ...known };
        next[accountId] = email;
      }
      if (next !== undefined) setByDoc((prev) => ({ ...prev, [docId]: next! }));
    },
  };
}
