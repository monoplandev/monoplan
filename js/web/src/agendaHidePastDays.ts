// Whether the web Upcoming view leaves out the days gone by ("Hide past
// days" in its filter menu), so the agenda starts at Overdue / Today.
// Device-local UI state, so localStorage, like `agendaHideEmptyDays.ts`.
// Off by default: only "on" is stored.

const KEY = "monoplan.agenda.hidePastDays";

export function loadHidePastDays(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export function saveHidePastDays(hide: boolean): void {
  try {
    if (hide) localStorage.setItem(KEY, "1");
    else localStorage.removeItem(KEY);
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
}
