// Which workspace the combined Inbox was last filtered to
// (`spec/inbox-plan.md` "One Inbox"): the doc id, or nothing for every
// workspace. Device-local UI state, so localStorage, like
// `agendaMode.ts`. The sidebar's Inbox row and the 4 key open the
// Inbox the way it was left. Only a filter is stored.

const KEY = "monoplan.inbox.scope";

/** The doc id the Inbox was last filtered to, or null for all. */
export function loadInboxScope(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function saveInboxScope(docId: string | null): void {
  try {
    if (docId === null) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, docId);
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
}
