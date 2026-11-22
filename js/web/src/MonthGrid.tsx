// The month grid over corvu's headless `@corvu/calendar`, shared by the
// date pickers (`CalendarPicker.tsx`) and the sidebar's mini calendar
// (`MiniCalendar.tsx`): month header with ‹ / › paging, two-letter
// weekday headings, day cells with adjacent months' spill-over days
// dimmed but live, and today's week banded. Owns no surface and no
// value semantics: the host says what is selected and what a pick does.

import Calendar from "@corvu/calendar";
import { createMemo, For } from "solid-js";
import { localDateStamp, nowMs, todayStamp } from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";

export function MonthGrid(props: {
  /** The selected day, or null for none. */
  value: Date | null;
  /** The month shown (any day in it). */
  month: Date;
  onMonthChange: (month: Date) => void;
  /** A day clicked; null when it was the selected day (corvu's
   *  deselect), which the pickers treat as "close, change nothing". */
  onPick: (day: Date | null) => void;
  /** Days drawn but inert (dimmed). */
  disabled?: (day: Date) => boolean;
  /** `YYYY-MM-DD` of a day to mark softly (a suggestion under the
   *  cursor), or null. */
  previewDay?: string | null;
  /** Always draw six weeks (the extra filled from the next month), so
   *  the grid keeps one height as the months page by. */
  fixedWeeks?: boolean;
  /** Month in the header as three letters ("Nov 2026") rather than in
   *  full, for a narrow host. */
  shortMonth?: boolean;
}) {
  const { m, locale } = useAppI18n();
  const today = createMemo(() => todayStamp(nowMs()));

  const monthLabelFmt = createMemo(
    () => new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" }),
  );
  // Three letters by grapheme off the short form: some locales' short
  // month is four ("sept") or carries a period ("nov.").
  const shortMonthFmt = createMemo(
    () => new Intl.DateTimeFormat(locale(), { month: "short", year: "numeric" }),
  );
  const monthLabel = (d: Date) => {
    if (!props.shortMonth) return monthLabelFmt().format(d);
    return shortMonthFmt()
      .formatToParts(d)
      .map((p) =>
        p.type === "month"
          ? Array.from(p.value.replace(/\./g, "")).slice(0, 3).join("")
          : p.value,
      )
      .join("");
  };
  // Two-letter day headings (Mo, Tu). Intl only offers three letters
  // ("short") or one ("narrow"), so trim the short form by grapheme; the
  // full name rides along as the cell's `abbr` for assistive tech.
  const weekdayFmt = createMemo(
    () => new Intl.DateTimeFormat(locale(), { weekday: "short" }),
  );
  const weekdayLongFmt = createMemo(
    () => new Intl.DateTimeFormat(locale(), { weekday: "long" }),
  );
  const weekdayHeading = (d: Date) =>
    Array.from(weekdayFmt().format(d)).slice(0, 2).join("");

  return (
    <Calendar
      mode="single"
      value={props.value}
      month={props.month}
      onMonthChange={props.onMonthChange}
      // Adjacent months' spill-over days are real dates; corvu's
      // default renders them but makes them inert, which reads as
      // broken. Keep them pickable, just dimmed (see data-outside).
      disableOutsideDays={false}
      fixedWeeks={props.fixedWeeks ?? false}
      disabled={props.disabled}
      onValueChange={(d) => props.onPick(d)}
    >
      {(cal) => (
        <>
          <div class="calendar-header">
            <Calendar.Nav
              action="prev-month"
              class="calendar-nav"
              aria-label={m().deadline.prevMonth}
            >
              ‹
            </Calendar.Nav>
            <Calendar.Label class="calendar-label">
              {monthLabel(cal.month)}
            </Calendar.Label>
            <Calendar.Nav
              action="next-month"
              class="calendar-nav"
              aria-label={m().deadline.nextMonth}
            >
              ›
            </Calendar.Nav>
          </div>
          <Calendar.Table class="calendar-table">
            <thead>
              <tr>
                <For each={cal.weekdays}>
                  {(weekday) => (
                    <Calendar.HeadCell
                      class="calendar-headcell"
                      abbr={weekdayLongFmt().format(weekday)}
                    >
                      {weekdayHeading(weekday)}
                    </Calendar.HeadCell>
                  )}
                </For>
              </tr>
            </thead>
            <tbody>
              <For each={cal.weeks}>
                {(week) => (
                  <tr
                    data-current-week={
                      week.some((day) => localDateStamp(day) === today())
                        ? ""
                        : undefined
                    }
                  >
                    <For each={week}>
                      {(day) => (
                        <Calendar.Cell class="calendar-cell">
                          <Calendar.CellTrigger
                            day={day}
                            class="calendar-cell-trigger"
                            data-preview={
                              localDateStamp(day) === props.previewDay ? "" : undefined
                            }
                            data-outside={
                              day.getMonth() !== cal.month.getMonth()
                                ? ""
                                : undefined
                            }
                          >
                            {day.getDate()}
                          </Calendar.CellTrigger>
                        </Calendar.Cell>
                      )}
                    </For>
                  </tr>
                )}
              </For>
            </tbody>
          </Calendar.Table>
        </>
      )}
    </Calendar>
  );
}
