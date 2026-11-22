// Whether the web Upcoming view hides the days nothing is on ("Hide
// empty days" in its filter menu) or gives every day a section
// (`fillEmptyDays` in `dayGroups.ts`). Device-local UI state, so
// localStorage, like `navWidth.ts`. Off by default: only "on" is stored.

const KEY = "monoplan.agenda.hideEmptyDays";

export function loadHideEmptyDays(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export function saveHideEmptyDays(hide: boolean): void {
  try {
    if (hide) localStorage.setItem(KEY, "1");
    else localStorage.removeItem(KEY);
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
}
