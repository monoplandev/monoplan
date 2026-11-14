// The workspace view ("which list / Done / Bin am I on?") as a signal
// seeded from, and written through to, the per-account prefs row.
// Kept apart from the runtime (`runtime.ts`): the view is local UI
// state, independent of how the docs behind it are synced.

import { createEffect, createSignal, on, type Accessor } from "solid-js";
import { savePrefs, type Prefs, type ViewKey } from "../prefs.ts";
import type { DocsLayer } from "./docs.ts";

export function createViewState(
  docs: DocsLayer,
  prefs: Prefs | null,
  accountId: string,
): { view: Accessor<ViewKey>; setView: (v: ViewKey) => void } {
  // Seed from the prefs row; a `kind:"list"` pointing at a
  // since-deleted list falls back to Inbox silently. `focus`/`done`/`bin`
  // are global views and always resolvable.
  // A list view names its doc (`ViewKey.docId`, absent ≡ Home); the
  // canonical form drops a Home doc id so equality, prefs and URLs
  // agree. Every write goes through `setView`, which normalises.
  const normalizeView = (v: ViewKey): ViewKey =>
    v.kind === "list" ? docs.listView(docs.viewDocId(v), v.id) : v;
  const initialView: ViewKey = (() => {
    const v = prefs?.currentView;
    if (!v) return { kind: "list", id: "inbox" };
    if (v.kind === "list") {
      const target = docs.appForView(v);
      if (v.id !== "inbox" && !target.state.listsById[v.id]) {
        return { kind: "list", id: "inbox" };
      }
      return normalizeView(v);
    }
    return v;
  })();
  const [view, setViewRaw] = createSignal<ViewKey>(initialView);
  const setView = (v: ViewKey): void => {
    setViewRaw(normalizeView(v));
  };

  // ---------- Prefs: separate store, write-through ----------
  //
  // View state lives in its own IDB store keyed per account. Decoupled
  // from `device` so a burst of typing doesn't rewrite the view, and
  // switching lists doesn't rewrite the sync frontier. Anonymous
  // sessions persist too — the device row gate is about identity, not
  // local-only UI state.
  //
  // No debounce: view changes are user-initiated (click / keyboard
  // nav) and rare on the timescale of an IDB write. Coalescing them
  // would only buy us dropped saves when the tab closes mid-window.
  createEffect(
    on(
      view,
      () => {
        void savePrefs(accountId, {
          currentView: view(),
        }).catch((e) => {
          console.error("prefs save failed:", e);
        });
      },
      { defer: true },
    ),
  );

  return { view, setView };
}
