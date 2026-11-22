// The sidebar's mini calendar: a month at a glance above the footer, for
// orientation (what day it is, where a date falls in the week). The
// shared corvu grid (`MonthGrid.tsx`) with nothing selected; a click on
// a day opens the agenda at that day (Workspace's `showAgendaDay`). The
// footer's calendar button shows and hides it, a device preference in
// localStorage like `navWidth.ts`: shown by default, only "hidden" is
// stored.

import { Tooltip } from "@kobalte/core/tooltip";
import { createSignal } from "solid-js";
import { localDateStamp, nowMs } from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import calendarSvg from "./icons/calendar.svg?raw";
import { MonthGrid } from "./MonthGrid.tsx";

const KEY = "monoplan.nav.miniCalendarHidden";

export function loadMiniCalendarShown(): boolean {
  try {
    return localStorage.getItem(KEY) !== "1";
  } catch {
    return true;
  }
}

export function saveMiniCalendarShown(shown: boolean): void {
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
        // Pinned above the footer: a five-week month would move the
        // header and grow the nav list by a row as months page by.
        fixedWeeks
        shortMonth
        onPick={(d) => {
          if (d) props.onPickDay(localDateStamp(d));
        }}
      />
    </div>
  );
}

/** The footer button that shows / hides the mini calendar. */
export function MiniCalendarToggle(props: {
  shown: boolean;
  onToggle: () => void;
}) {
  const { m } = useAppI18n();
  const label = () =>
    props.shown ? m().deadline.hideCalendar : m().deadline.showCalendar;
  return (
    <Tooltip openDelay={200} closeDelay={0} placement="top">
      <Tooltip.Trigger
        as="button"
        type="button"
        class="icon-button nav-calendar-button"
        tabIndex={-1}
        aria-label={label()}
        aria-pressed={props.shown}
        onClick={() => props.onToggle()}
        innerHTML={calendarSvg}
      />
      <Tooltip.Portal>
        <Tooltip.Content class="tooltip-content">
          {label()}
          <Tooltip.Arrow />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip>
  );
}
