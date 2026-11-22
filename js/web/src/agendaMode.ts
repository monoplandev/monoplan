// Which agenda view the calendar opens in: the agenda list (days down
// the page) or the agenda columns (a column per day). Device-local UI
// state, so localStorage, like `agendaHideEmptyDays.ts`. The list is the
// default: only "columns" is stored.

export type AgendaMode = "list" | "columns";

const KEY = "monoplan.agenda.mode";

export function loadAgendaMode(): AgendaMode {
  try {
    return localStorage.getItem(KEY) === "columns" ? "columns" : "list";
  } catch {
    return "list";
  }
}

export function saveAgendaMode(mode: AgendaMode): void {
  try {
    if (mode === "columns") localStorage.setItem(KEY, "columns");
    else localStorage.removeItem(KEY);
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
}
