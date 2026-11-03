// The macOS app's device settings as the window's page sees them
// (spec/mac-window-plan.md "Settings"; `WindowSettings` on the Swift
// side). The app owns these; the page reads them and never persists
// its own. Two deliveries: a document-start user script sets
// `window.__monoplanSettings` before any module runs, so the
// readers (`whenToken.ts`, `format.tsx`, `i18n.tsx`) see the app's
// values on first paint; and
// `window.__monoplanHost.settings(...)` (`mirrorHost.ts`) pushes a
// change live, which `applyHostSettings` fans out to the readers.
// Outside the app the global is absent, nothing here fires, and the
// readers fall back to their cookies.

export interface HostSettings {
  /** `whenToken.ts`: recognise a date typed into an item. */
  typedDates?: boolean;
  /** `i18n.tsx`: the app's own language. */
  language?: "es" | "en";
  /** `format.tsx`: the system's hour cycle. */
  hourCycle?: "12h" | "24h";
}

export const SETTINGS_GLOBAL = "__monoplanSettings";

function readGlobal(): HostSettings | null {
  if (typeof window === "undefined") return null;
  const v = (window as unknown as Record<string, unknown>)[SETTINGS_GLOBAL];
  return v !== null && typeof v === "object" ? (v as HostSettings) : null;
}

let current: HostSettings | null = readGlobal();
const listeners = new Set<(settings: HostSettings) => void>();

/** Whether the app provides settings at all (the window's page). */
export function hasHostSettings(): boolean {
  return current !== null;
}

/** The app's value, or undefined outside the app (or for a key this
 *  app build does not send). */
export function hostSetting<K extends keyof HostSettings>(key: K): HostSettings[K] | undefined {
  return current?.[key];
}

/** A live change from the app: merged over what it sent before, then
 *  every reader is told. */
export function applyHostSettings(settings: HostSettings): void {
  current = { ...current, ...settings };
  for (const cb of listeners) cb(current);
}

/** Hear every live change; returns the unsubscribe. */
export function onHostSettings(cb: (settings: HostSettings) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}
