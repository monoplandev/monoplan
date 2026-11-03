// Theme preference manager. Stores "auto" | "light" | "dark" in a
// cookie so the inline pre-render script in index.html can pick it up
// before Solid mounts (no FOUC). Single-origin cookie — monoplan's
// bundle is served from the API origin. The same value also goes to
// `localStorage`; the cookie wins where both exist.
//
// The macOS app's window (`mirror.html`) has no theme of its own: the
// app owns Appearance (Settings › General, spec/mac-window-plan.md
// "Settings") and sets the whole app's appearance, which the webview's
// `prefers-color-scheme` follows. There the preference is always
// "auto", nothing is stored, and a value left by an earlier build is
// ignored.

import { isMacShell } from "./sync/mirrorHost.ts";

export type ThemePreference = "auto" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const COOKIE_NAME = "theme";
const MAX_AGE = 31536000; // 1 year

function cookieAttrs(maxAge: number): string {
  return `path=/;max-age=${maxAge};SameSite=Lax`;
}

function readCookie(): ThemePreference {
  if (isMacShell()) return "auto";
  try {
    const m = document.cookie.match(/(?:^|; )theme=([01])/);
    if (m) return m[1] === "1" ? "dark" : "light";
  } catch {}
  try {
    const v = localStorage.getItem(COOKIE_NAME);
    if (v === "1" || v === "0") return v === "1" ? "dark" : "light";
  } catch {}
  return "auto";
}

function writeCookie(pref: ThemePreference) {
  if (isMacShell()) return;
  const value = pref === "auto" ? null : pref === "dark" ? "1" : "0";
  try {
    if (value === null) {
      document.cookie = `${COOKIE_NAME}=;${cookieAttrs(0)}`;
    } else {
      document.cookie = `${COOKIE_NAME}=${value};${cookieAttrs(MAX_AGE)}`;
    }
  } catch {}
  try {
    if (value === null) localStorage.removeItem(COOKIE_NAME);
    else localStorage.setItem(COOKIE_NAME, value);
  } catch {}
}

export function resolveTheme(pref: ThemePreference): ResolvedTheme {
  if (pref === "auto") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  }
  return pref;
}

export function createTheme(opts?: {
  onApply?: (resolved: ResolvedTheme) => void;
}) {
  let current: ThemePreference = readCookie();

  function apply(pref: ThemePreference) {
    const resolved = resolveTheme(pref);
    document.documentElement.dataset.theme = resolved;
    opts?.onApply?.(resolved);
  }

  apply(current);

  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const onChange = () => {
    if (current === "auto") apply("auto");
  };
  mq.addEventListener("change", onChange);

  return {
    get: () => current,
    set(pref: ThemePreference) {
      current = pref;
      apply(pref);
      writeCookie(pref);
    },
    dispose() {
      mq.removeEventListener("change", onChange);
    },
  };
}
