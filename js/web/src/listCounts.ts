// Whether the sidebar counts each list's open items: a device
// preference like the time format (`format.tsx`), so it lives in a
// cookie beside it, off by default. Inbox, Starred and the bin are counted
// either way.
import { createSignal } from "solid-js";

const LIST_COUNTS_COOKIE = "listcounts";

function readListCountsPref(): boolean {
  try {
    return /(?:^|; )listcounts=1(?:;|$)/.test(document.cookie);
  } catch {
    return false;
  }
}

const [listCountsPref, setListCountsSignal] = createSignal(readListCountsPref());
export { listCountsPref };

export function setListCountsPref(on: boolean): void {
  setListCountsSignal(on);
  try {
    document.cookie = on
      ? `${LIST_COUNTS_COOKIE}=1;path=/;max-age=31536000;SameSite=Lax`
      : `${LIST_COUNTS_COOKIE}=;path=/;max-age=0;SameSite=Lax`;
  } catch {}
}
