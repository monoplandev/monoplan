import { createSignal, onCleanup, type Accessor } from "solid-js";
import { useAppI18n } from "./i18n.tsx";
import type { DocApp } from "./sync/store.ts";

export type SyncStatus = "offline" | "disconnected" | "syncing" | "synced";

/** Per-doc sync facts read off one `DocApp`'s engine, shared by the
 *  footer connection popover (Home) and each workspace's Advanced dialog.
 *  Every accessor keys on `app.version()`, which bumps on each dispatched
 *  event, so consumers re-run exactly when the numbers can change. The
 *  reads themselves are cheap — the engine forwards into the doc.
 *
 *  `online` is the WebSocket link, which is per connection (one for all
 *  open docs), so it comes from the caller rather than the doc. */
export function createDocSyncFacts(app: DocApp, online: Accessor<boolean>) {
  const pending = (): boolean => {
    app.version();
    return app.engine.hasUnsyncedOps();
  };
  const seqLabel = (): string => {
    app.version();
    return String(app.engine.lastContiguousSeq());
  };
  // Epoch arrives on HelloAck, which dispatches no event of its own,
  // so also key on the link state: it flips to online right after.
  const epochLabel = (): string | null => {
    app.version();
    online();
    const epoch = app.engine.serverEpoch();
    return epoch === undefined ? null : String(epoch);
  };
  const fingerprintHex = (): string => {
    app.version();
    const buf = app.engine.fingerprint();
    // Full 64-char hex; the row CSS-truncates with text-overflow so the
    // visible width tracks the container, while copy-paste yields the
    // entire hash.
    let s = "";
    for (let i = 0; i < buf.length; i++) {
      s += buf[i].toString(16).padStart(2, "0");
    }
    return s;
  };
  const itemsCount = (): number => Object.keys(app.state.itemsById).length;
  const listsCount = (): number => app.state.listsOrder.length;

  const { status, statusLabel } = createSyncStatus(pending, online);

  return {
    pending,
    seqLabel,
    epochLabel,
    fingerprintHex,
    itemsCount,
    listsCount,
    status,
    statusLabel,
  };
}

/** Fold link state and local pending work into the four-way status shown
 *  by the dot + label. `pending` is whatever scope the caller cares about:
 *  one doc (the Advanced dialog) or every open doc (the footer popover). */
export function createSyncStatus(pending: Accessor<boolean>, online: Accessor<boolean>) {
  const { m } = useAppI18n();

  // Browser network reachability, distinct from `online` (the WS link).
  // Lets us split "no network at all" (Offline) from "network is up but
  // we're not connected to the server" (Disconnected).
  const [navOnline, setNavOnline] = createSignal(navigator.onLine);
  const onNet = () => setNavOnline(navigator.onLine);
  window.addEventListener("online", onNet);
  window.addEventListener("offline", onNet);
  onCleanup(() => {
    window.removeEventListener("online", onNet);
    window.removeEventListener("offline", onNet);
  });

  const status = (): SyncStatus => {
    if (!navOnline()) return "offline";
    if (!online()) return "disconnected";
    return pending() ? "syncing" : "synced";
  };
  const statusLabel = (): string => {
    switch (status()) {
      case "offline":
        return m().nav.offline;
      case "disconnected":
        return m().nav.disconnected;
      case "syncing":
        return m().nav.syncing;
      default:
        return m().nav.synced;
    }
  };
  return { status, statusLabel };
}
