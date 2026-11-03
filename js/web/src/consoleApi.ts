// `monoplan.sync()` in the devtools console, browser and app window
// alike: the per-workspace sync facts (status, seq, epoch, fingerprint,
// counts, doc id) that used to be the Settings dialog's Advanced
// section. A console table reads better than a dialog for comparing
// two devices, and nothing else needed the section
// (spec/mac-window-plan.md "Settings"). Direct engine reads, no
// reactivity, so it works from the console with no owner. A debug
// build of the app is inspectable (`isInspectable`).

import type { DocsLayer } from "./sync/docs.ts";

export interface SyncFactsRow {
  workspace: string;
  docId: string;
  /** "synced" when nothing is unsent and the link is up. */
  status: "synced" | "pending" | "offline";
  seq: number;
  epoch: number | null;
  fingerprint: string;
  items: number;
  lists: number;
}

export function syncFactsRows(docs: DocsLayer, online: boolean): SyncFactsRow[] {
  return docs.docs().map((entry) => {
    const app = entry.app;
    const pending = app.engine.hasUnsyncedOps();
    const epoch = app.engine.serverEpoch();
    return {
      workspace: entry.home ? "Personal" : (app.state.settings.docName ?? "(untitled)"),
      docId: app.docId,
      status: !online ? "offline" : pending ? "pending" : "synced",
      seq: Number(app.engine.lastContiguousSeq()),
      epoch: epoch === undefined ? null : Number(epoch),
      fingerprint: hex(app.engine.fingerprint()),
      items: Object.keys(app.state.itemsById).length,
      lists: app.state.listsOrder.length,
    };
  });
}

function hex(buf: Uint8Array): string {
  let s = "";
  for (let i = 0; i < buf.length; i++) s += buf[i].toString(16).padStart(2, "0");
  return s;
}

/** Install the global; once per page, from `Workspace`. */
export function installConsoleApi(docs: DocsLayer, online: () => boolean): void {
  (window as unknown as Record<string, unknown>).monoplan = {
    sync(): SyncFactsRow[] {
      const rows = syncFactsRows(docs, online());
      console.table(rows);
      return rows;
    },
  };
}
