// Whether the sidebar counts each list's open items: a device
// preference like typed dates (`whenSuggest.ts`), so it lives in a cookie
// beside it, off by default. Inbox, Starred and the bin are counted
// either way. In the macOS app's window the app owns it
// (`hostSettings.ts`): read from there, followed live, never written.
import { createSignal } from "solid-js";
import { hasHostSettings, hostSetting, onHostSettings } from "./hostSettings.ts";

const LIST_COUNTS_COOKIE = "listcounts";

function readListCountsPref(): boolean {
  const host = hostSetting("listCounts");
  if (host !== undefined) return host;
  try {
    return /(?:^|; )listcounts=1(?:;|$)/.test(document.cookie);
  } catch {
    return false;
  }
}

const [listCountsPref, setListCountsSignal] = createSignal(readListCountsPref());
export { listCountsPref };
onHostSettings((s) => {
  if (s.listCounts !== undefined) setListCountsSignal(s.listCounts);
});

export function setListCountsPref(on: boolean): void {
  setListCountsSignal(on);
  if (hasHostSettings()) return;
  try {
    document.cookie = on
      ? `${LIST_COUNTS_COOKIE}=1;path=/;max-age=31536000;SameSite=Lax`
      : `${LIST_COUNTS_COOKIE}=;path=/;max-age=0;SameSite=Lax`;
  } catch {}
}
