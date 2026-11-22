// Whether the web Upcoming view shows every day or only the days
// something is on (`fillEmptyDays` in `dayGroups.ts`). Device-local UI
// state, so localStorage, like `navWidth.ts`. On by default: only the
// "off" choice is stored.

const KEY = "monoplan.agenda.allDays";

export function loadAgendaAllDays(): boolean {
  try {
    return localStorage.getItem(KEY) !== "0";
  } catch {
    return true;
  }
}

export function saveAgendaAllDays(on: boolean): void {
  try {
    if (on) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, "0");
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
}
