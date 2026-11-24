// The month grid over corvu's headless `@corvu/calendar`, shared by the
// date pickers (`CalendarPicker.tsx`) and the sidebar's mini calendar
// (`MiniCalendar.tsx`): month header with ‹ / › paging (or up / down
// chevrons grouped on the right, `navRight`), two-letter
// weekday headings, day cells with adjacent months' spill-over days
// dimmed but live, and today's week banded. Owns no surface and no
// value semantics: the host says what is selected and what a pick does.

import Calendar from "@corvu/calendar";
import { createMemo, For, Show } from "solid-js";
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
  /** Month label on the left with both paging buttons grouped on the
   *  right as up / down chevrons (previous / next), rather than ‹ / ›
   *  either side of the label. */
  navRight?: boolean;
  /** With `navRight`: a "Today" button (Radix reset icon) beside the month label, shown
   *  while another month is paged in, that pages back to this month. */
  todayButton?: boolean;
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
  const isThisMonth = (d: Date) => {
    const now = new Date(nowMs());
    return d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth();
  };
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
          <div class="calendar-header" data-nav-right={props.navRight ? "" : undefined}>
            <Show
              when={props.navRight}
              fallback={
                <>
                  <Calendar.Nav
                    action="prev-month"
                    class="calendar-nav"
                    aria-label={m().calendar.prevMonth}
                  >
                    ‹
                  </Calendar.Nav>
                  <Calendar.Label class="calendar-label">
                    {monthLabel(cal.month)}
                  </Calendar.Label>
                  <Calendar.Nav
                    action="next-month"
                    class="calendar-nav"
                    aria-label={m().calendar.nextMonth}
                  >
                    ›
                  </Calendar.Nav>
                </>
              }
            >
              <div class="calendar-label-group">
                <Calendar.Label class="calendar-label">
                  {monthLabel(cal.month)}
                </Calendar.Label>
                <Show when={props.todayButton && !isThisMonth(cal.month)}>
                  <button
                    type="button"
                    class="calendar-today"
                    aria-label={m().calendar.today}
                    title={m().calendar.today}
                    onClick={() => props.onMonthChange(new Date(nowMs()))}
                  >
                    <ResetIcon />
                  </button>
                </Show>
              </div>
              <div class="calendar-nav-group">
                <Calendar.Nav
                  action="prev-month"
                  class="calendar-nav"
                  aria-label={m().calendar.prevMonth}
                >
                  <Chevron up />
                </Calendar.Nav>
                <Calendar.Nav
                  action="next-month"
                  class="calendar-nav"
                  aria-label={m().calendar.nextMonth}
                >
                  <Chevron />
                </Calendar.Nav>
              </div>
            </Show>
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

/** A small stroked chevron, pointing down unless `up`. */
function Chevron(props: { up?: boolean }) {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <path
        d={props.up ? "M2 6.5 5 3.5 8 6.5" : "M2 3.5 5 6.5 8 3.5"}
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  );
}

/** Radix Icons' `reset` (MIT): a return arrow. */
function ResetIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 15 15" fill="none" aria-hidden="true">
      <path
        d="M4.85355 2.14645C5.04882 2.34171 5.04882 2.65829 4.85355 2.85355L3.70711 4H9C11.4853 4 13.5 6.01472 13.5 8.5C13.5 10.9853 11.4853 13 9 13H5C4.72386 13 4.5 12.7761 4.5 12.5C4.5 12.2239 4.72386 12 5 12H9C10.933 12 12.5 10.433 12.5 8.5C12.5 6.567 10.933 5 9 5H3.70711L4.85355 6.14645C5.04882 6.34171 5.04882 6.65829 4.85355 6.85355C4.65829 7.04882 4.34171 7.04882 4.14645 6.85355L2.14645 4.85355C1.95118 4.65829 1.95118 4.34171 2.14645 4.14645L4.14645 2.14645C4.34171 1.95118 4.65829 1.95118 4.85355 2.14645Z"
        fill="currentColor"
        fill-rule="evenodd"
        clip-rule="evenodd"
      />
    </svg>
  );
}
