// The full-page wait: the app's empty panes (sidebar, main surface, idle
// side panel) standing in for the app while it boots (session restore,
// the tab gate, the doc load). Built from the real layout classes
// (`.app`, `.nav`, `.content`, `.side-panel`) so the workspace lands on
// the same lines when it replaces this.
import { Show } from "solid-js";
import { useAppI18n } from "./i18n.tsx";
import { loadNavWidth } from "./navWidth.ts";
import { isMacShell } from "./sync/mirrorHost.ts";

export function Splash() {
  const { m } = useAppI18n();
  return (
    <div
      class="app splash"
      role="status"
      aria-label={m().common.loading}
      style={{ "--nav-width": `${loadNavWidth()}px` }}
    >
      <div class="nav">
        <Show when={isMacShell()}>
          <div class="nav-titlebar" />
        </Show>
      </div>
      <div class="content" />
      <div class="side-panel" />
    </div>
  );
}
