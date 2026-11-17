// Width of the desktop side panel, dragged from its left edge
// (`.panel-resizer` in Workspace.tsx). Device-local, in localStorage,
// for the reasons in `navWidth.ts`.
//
// This is the width asked for. The column itself (`--panel-col` in
// styles.css) gives some of it back on a narrow window so the main
// surface keeps a usable width.

const KEY = "monoplan.panel.width";

export const PANEL_WIDTH_MIN = 260;
export const PANEL_WIDTH_MAX = 520;
export const PANEL_WIDTH_DEFAULT = 300;

export function clampPanelWidth(px: number): number {
  if (!Number.isFinite(px)) return PANEL_WIDTH_DEFAULT;
  return Math.round(Math.min(PANEL_WIDTH_MAX, Math.max(PANEL_WIDTH_MIN, px)));
}

export function loadPanelWidth(): number {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return PANEL_WIDTH_DEFAULT;
    return clampPanelWidth(Number(raw));
  } catch {
    return PANEL_WIDTH_DEFAULT;
  }
}

/** Persist `px`; the default is stored as no row at all, so a later
 *  change to the default reaches devices that never resized. */
export function savePanelWidth(px: number): void {
  try {
    if (px === PANEL_WIDTH_DEFAULT) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, String(clampPanelWidth(px)));
  } catch {
    // Private-mode / quota failures: the width just won't stick.
  }
}
