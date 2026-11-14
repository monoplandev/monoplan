// The full-page wait: the app's empty panes (sidebar, main surface, idle
// side panel) standing in for the app while it boots (session restore,
// the tab gate, the doc load). Built from the real layout classes
// (`.app`, `.nav`, `.content`, `.side-panel`) so the workspace lands on
// the same lines when it replaces this.
import { useAppI18n } from "./i18n.tsx";
import { loadNavWidth } from "./navWidth.ts";

export function Splash() {
  const { m } = useAppI18n();
  return (
    <div
      class="app splash"
      role="status"
      aria-label={m().common.loading}
      style={{ "--nav-width": `${loadNavWidth()}px` }}
    >
      <div class="nav" />
      <div class="content" />
      <div class="side-panel" />
    </div>
  );
}
