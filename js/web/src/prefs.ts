// Per-account UI preferences for the web client.
//
// Distinct from `DeviceConfig` (sync identity + frontier in the
// `device` store) — these are purely "where was I" / "how do I like
// the UI" state that shouldn't be rewritten every time the sync
// engine ticks, and conversely shouldn't drag the frontier write
// every time the user changes view.
//
// One row per account in the `prefs` store of the shared `monoplan-web`
// IndexedDB database (schema declared in `@monoplan/core/storage/web-db`).
// Single-write-replace: each `savePrefs` overwrites the whole row.
// Callers compose a full `Prefs` object; we don't merge on the server
// side because there isn't one — this is local-only state.

import { STORE_PREFS, openMonoplanDb } from "@monoplan/core/storage/web-db";

/** Last view the user was on. Mirrors the in-memory shape used by
 *  `App.tsx`; persisted verbatim. */
export type ViewKey =
  | {
      kind: "list";
      id: string;
      /** The doc the list lives in. Absent ≡ the account's Home doc, so
       *  rows written before docs existed, and every URL for a Home
       *  list, keep their shape; the runtime's `setView` normalises a
       *  Home doc id away (`spec/sharing-plan.md` Phase 5). */
      docId?: string;
    }
  | { kind: "focus" }
  | { kind: "upcoming" }
  | { kind: "done" }
  | { kind: "bin" };

/** Future-proof container. Add new fields as optional so older rows
 *  written before the field existed load as `undefined` without a
 *  schema bump. */
export interface Prefs {
  currentView?: ViewKey;
}

interface PrefsRow {
  account_id: string;
  prefs: Prefs;
}

/** Read the per-account prefs row, or null if none exists. */
export async function loadPrefs(accountId: string): Promise<Prefs | null> {
  const db = await openMonoplanDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_PREFS, "readonly");
    const req = tx.objectStore(STORE_PREFS).get(accountId);
    req.onsuccess = () => {
      const row = req.result as PrefsRow | undefined;
      resolve(row?.prefs ?? null);
    };
    req.onerror = () => reject(req.error);
  });
}

/** Replace the per-account prefs row. */
export async function savePrefs(
  accountId: string,
  prefs: Prefs,
): Promise<void> {
  const db = await openMonoplanDb();
  const row: PrefsRow = { account_id: accountId, prefs };
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE_PREFS, "readwrite");
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error ?? new Error("idb tx aborted"));
    tx.objectStore(STORE_PREFS).put(row);
  });
}
