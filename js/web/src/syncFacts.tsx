import { createSignal, onCleanup, type Accessor } from "solid-js";
import { useAppI18n } from "./i18n.tsx";

export type SyncStatus = "offline" | "disconnected" | "syncing" | "synced";

/** Fold link state and local pending work into the four-way status shown
 *  by the dot + label. `pending` is whatever scope the caller cares about
 *  (the footer popover: every open doc). */
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
