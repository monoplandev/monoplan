// Per-doc member directory: account id → email for every shared doc the
// session has open, so item attribution (`ItemView.createdBy`, an
// account id) can render as a person. Loaded lazily from
// `GET /api/docs/:id/members` once per doc per session; a doc's list
// changes rarely (invites accepted, members removed) and a stale entry
// only means a removed member still resolves to their email, which is
// the right thing to show on items they created. Home is never loaded:
// attribution is not shown there (`spec/sharing-plan.md` §Attribution).

import { createSignal } from "solid-js";
import { api, type Member } from "../api.ts";

export interface MemberDirectory {
  /** Email of `accountId` as a member of `docId`, or undefined while the
   *  doc's list is not loaded or when the account is not in it (a
   *  member removed before this session loaded the list). Reactive: a
   *  read inside a memo re-runs when the list lands. */
  emailOf(docId: string, accountId: string): string | undefined;
  /** Fetch the doc's member list if this session has not yet. A failed
   *  fetch is forgotten, so the next `ensure` retries. */
  ensure(docId: string): void;
  /** Fetch (or re-fetch) the doc's member list. Resolves after the
   *  directory is updated; a failure leaves the previous list. */
  refresh(docId: string): Promise<void>;
}

type FetchMembers = (docId: string) => Promise<Member[]>;

const defaultFetch: FetchMembers = async (docId) => (await api.listMembers(docId)).members;

export function createMemberDirectory(fetchMembers: FetchMembers = defaultFetch): MemberDirectory {
  const [byDoc, setByDoc] = createSignal<Record<string, Record<string, string>>>({});
  const inflight = new Set<string>();

  const refresh = async (docId: string): Promise<void> => {
    if (inflight.has(docId)) return;
    inflight.add(docId);
    try {
      const list = await fetchMembers(docId);
      const emails: Record<string, string> = {};
      for (const mb of list) emails[mb.account_id] = mb.email;
      setByDoc((prev) => ({ ...prev, [docId]: emails }));
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
    ensure(docId) {
      // Untracked on purpose: `ensure` runs inside effects, and reading
      // the signal here would make a load re-trigger the effect.
      if (docId in byDoc() === false) void refresh(docId);
    },
    refresh,
  };
}
