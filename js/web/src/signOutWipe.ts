// The browser-storage half of a user sign-out: everything this app
// keeps outside IndexedDB that describes the account or what it was
// looking at. IndexedDB itself goes through `wipeMonoplanDb`.
//
// Kept on purpose: `theme`, `locale` and `hourcycle` (cookie plus
// localStorage mirror). They describe the browser, not the account,
// and the signed-out screen paints with them.

import { clearPendingInvite } from "./invite.ts";
import { setListCountsPref } from "./listCounts.ts";

/** Every localStorage key of ours is prefixed `monoplan:` or
 *  `monoplan.`; the three kept preferences are not. */
const OWN_PREFIX = "monoplan";

export function clearBrowserState(): void {
  try {
    const doomed: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key !== null && key.startsWith(OWN_PREFIX)) doomed.push(key);
    }
    for (const key of doomed) localStorage.removeItem(key);
  } catch {
    // Private mode / blocked storage: nothing was stored either.
  }
  clearPendingInvite();
  setListCountsPref(false);
}
