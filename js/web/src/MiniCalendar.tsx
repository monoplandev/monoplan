// The sidebar's mini calendar: a month at a glance pinned at the top, for
// orientation (what day it is, where a date falls in the week). The
// shared corvu grid (`MonthGrid.tsx`) with nothing selected; a click on
// a day opens the agenda at that day (Workspace's `showAgendaDay`). The
// sidebar's blank-space context menu (`nav.tsx`) shows and hides it, a
// device preference in localStorage like `navWidth.ts`: shown by
// default, only "hidden" is stored. The item the task surface shows has
// its date marked, and the grid pages to that date's month when it
// changes. The agenda's days on screen are shaded as a band while it
// is open, and the grid shows the month of the band's first day,
// paging when that day moves to another month (so a month paged to by
// hand holds until the agenda's first day on screen changes).

import { createEffect, createSignal, on, untrack } from "solid-js";
import { localDateStamp, nowMs, parseLocalDateParts } from "./format.tsx";
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
  /** `YYYY-MM-DD` to mark: the shown item's date, or null. */
  markedDay?: string | null;
  /** The agenda's days on screen, or null when it is not open. */
  range?: { from: string; to: string } | null;
}) {
  const [month, setMonth] = createSignal(new Date(nowMs()));
  // Whether days `from`..`to` touch the month shown (keys as strings:
  // the month's `YYYY-MM` against theirs).
  const inShownMonth = (from: string, to: string) => {
    const ym = localDateStamp(untrack(month)).slice(0, 7);
    return from.slice(0, 7) <= ym && ym <= to.slice(0, 7);
  };
  // Page to a newly marked day so it is in view; clearing the mark
  // leaves the month where it is.
  createEffect(
    on(
      () => props.markedDay ?? null,
      (day) => {
        const d = day ? parseLocalDateParts(day) : null;
        if (!d) return;
        if (!inShownMonth(day!, day!)) setMonth(d);
      },
    ),
  );
  createEffect(
    on(
      () => props.range?.from ?? null,
      (from) => {
        if (!from || inShownMonth(from, from)) return;
        const d = parseLocalDateParts(from);
        if (d) setMonth(d);
      },
    ),
  );
  return (
    <div class="mini-calendar">
      <MonthGrid
        value={null}
        markedDay={props.markedDay ?? null}
        range={props.range ?? null}
        month={month()}
        onMonthChange={setMonth}
        // Pinned above the nav list: a five-week month would shift the
        // list by a row as months page by.
        fixedWeeks
        shortMonth
        navRight
        todayButton
        onPick={(d) => {
          if (d) props.onPickDay(localDateStamp(d));
        }}
      />
    </div>
  );
}
