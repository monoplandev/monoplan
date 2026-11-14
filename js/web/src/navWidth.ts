// Width of the desktop sidebar, dragged from its right edge
// (`.nav-resizer` in Workspace.tsx).
//
// Device-local UI state, so localStorage rather than the per-account
// `prefs` row in IndexedDB: the right width depends on this screen, not
// on the account, and a synchronous read means the first paint already
// has it (same reasoning as `emoji/recents.ts`).

const KEY = "monoplan.nav.width";

export const NAV_WIDTH_MIN = 200;
export const NAV_WIDTH_MAX = 320;
export const NAV_WIDTH_DEFAULT = 260;

export function clampNavWidth(px: number): number {
  if (!Number.isFinite(px)) return NAV_WIDTH_DEFAULT;
  return Math.round(Math.min(NAV_WIDTH_MAX, Math.max(NAV_WIDTH_MIN, px)));
}

export function loadNavWidth(): number {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return NAV_WIDTH_DEFAULT;
    return clampNavWidth(Number(raw));
  } catch {
    return NAV_WIDTH_DEFAULT;
  }
}

/** Persist `px`; the default is stored as no row at all, so a later
 *  change to the default reaches devices that never resized. */
export function saveNavWidth(px: number): void {
  try {
    if (px === NAV_WIDTH_DEFAULT) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, String(clampNavWidth(px)));
  } catch {
    // Private-mode / quota failures: the width just won't stick.
  }
}
