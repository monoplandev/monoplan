// Inbox is a queue in arrival order, oldest first (`spec/inbox-plan.md`).
// Showing it newest first is a device preference like the list counts
// (`listCounts.ts`), so it lives in a cookie beside them, off by default.
// It reverses the display only and never writes to the doc.
import { createSignal } from "solid-js";

const INBOX_NEWEST_COOKIE = "inboxnewest";

function readInboxNewestPref(): boolean {
  try {
    return /(?:^|; )inboxnewest=1(?:;|$)/.test(document.cookie);
  } catch {
    return false;
  }
}

const [inboxNewestFirst, setInboxNewestSignal] = createSignal(readInboxNewestPref());
export { inboxNewestFirst };

export function setInboxNewestFirst(on: boolean): void {
  setInboxNewestSignal(on);
  try {
    document.cookie = on
      ? `${INBOX_NEWEST_COOKIE}=1;path=/;max-age=31536000;SameSite=Lax`
      : `${INBOX_NEWEST_COOKIE}=;path=/;max-age=0;SameSite=Lax`;
  } catch {}
}
