// The sidebar's mini calendar: a month at a glance pinned at the top, for
// orientation (what day it is, where a date falls in the week). The
// shared corvu grid (`MonthGrid.tsx`) with nothing selected; a click on
// a day opens the agenda at that day (Workspace's `showAgendaDay`). The
// sidebar's blank-space context menu (`nav.tsx`) shows and hides it, a
// device preference in localStorage like `navWidth.ts`: shown by
// default, only "hidden" is stored.

import { createSignal } from "solid-js";
import { localDateStamp, nowMs } from "./format.tsx";
import { MonthGrid } from "./MonthGrid.tsx";

const KEY = "monoplan.nav.miniCalendarHidden";

function loadMiniCalendarShown(): boolean {
  try {
    return localStorage.getItem(KEY) !== "1";
  } catch {
    return true;
  }
}

const [miniCalendarShown, setMiniCalendarSignal] = createSignal(loadMiniCalendarShown());
export { miniCalendarShown };

export function setMiniCalendarShown(shown: boolean): void {
  setMiniCalendarSignal(shown);
  try {
    if (shown) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, "1");
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
}

export function MiniCalendar(props: {
  /** A day clicked, as `YYYY-MM-DD`. */
  onPickDay: (day: string) => void;
}) {
  const [month, setMonth] = createSignal(new Date(nowMs()));
  return (
    <div class="mini-calendar">
      <MonthGrid
        value={null}
        month={month()}
        onMonthChange={setMonth}
        // Pinned above the nav list: a five-week month would shift the
        // list by a row as months page by.
        fixedWeeks
        shortMonth
        onPick={(d) => {
          if (d) props.onPickDay(localDateStamp(d));
        }}
      />
    </div>
  );
}
