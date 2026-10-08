// corvu's headless `@corvu/calendar` as a picker for a deadline or a
// planned date (`kind`). `CalendarPicker` is the body (grid, Remove
// footer), hosted in a Popover by the task surface's date and deadline
// fields (`WhenField.tsx`, `DeadlineField.tsx`). There is no standalone
// modal: a row's "Set date…" / "Set deadline…" opens the item with the
// matching popover showing, and the time is edited in the task surface's
// dates band (`WhenTimeRow`), not here.

import Calendar from "@corvu/calendar";
import { createMemo, For, Show } from "solid-js";
import {
  localDateStamp,
  parseLocalDateParts,
  whenDay,
  whenFromParts,
  whenTime,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";

export interface CalendarPickerProps {
  setOpen: (v: boolean) => void;
  /** `deadline` (default): date-only, `YYYY-MM-DD`. `when`: date plus an
   *  optional time, `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`. Picks labels and
   *  whether a pick carries the stored time along. */
  kind?: "deadline" | "when";
  /** Currently-set register value to preselect / open the calendar on, or
   *  null. */
  value: () => string | null;
  /** Fired with the picked register value (never null — removal is the
   *  button below); the host closes after. In `when` mode the pick keeps
   *  the stored time part. */
  onPick: (stamp: string) => void;
  /** Clear the value. When provided and a value is set, a "Remove" button
   *  shows at the bottom. */
  onRemove?: () => void;
  /** Days the grid shows but will not pick (dimmed, inert). The end-date
   *  picker uses it to keep the end inside the register's range. */
  disabled?: (day: Date) => boolean;
}

/** The picker body: month grid and the Remove footer. Owns no surface;
 *  the host (modal or popover) supplies it. */
export function CalendarPicker(props: CalendarPickerProps) {
  const { m, locale } = useAppI18n();
  const isWhen = () => props.kind === "when";

  // Register stamp ⇄ Date on the boundary — the register stores a floating
  // local date; corvu works in `Date`s. Never `new Date(stamp)` (that's UTC
  // and shifts the day in negative-offset zones).
  const value = createMemo<Date | null>(() => {
    const s = props.value();
    return s ? parseLocalDateParts(whenDay(s)) : null;
  });

  // The stored time part a `when` pick carries over to the new day.
  const time = () => whenTime(props.value() ?? "");

  const monthLabelFmt = createMemo(
    () => new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" }),
  );
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

  const labels = () => (isWhen() ? m().when : m().deadline);

  return (
    <>
      <Calendar
        mode="single"
        value={value()}
        initialMonth={value() ?? undefined}
        // Adjacent months' spill-over days are real dates; corvu's
        // default renders them but makes them inert, which reads as
        // broken. Keep them pickable, just dimmed (see data-outside).
        disableOutsideDays={false}
        disabled={props.disabled}
        onValueChange={(d) => {
          if (d) {
            const day = localDateStamp(d);
            props.onPick(isWhen() ? whenFromParts(day, time()) : day);
          }
          props.setOpen(false);
        }}
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
                {monthLabelFmt().format(cal.month)}
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
                    <tr>
                      <For each={week}>
                        {(day) => (
                          <Calendar.Cell class="calendar-cell">
                            <Calendar.CellTrigger
                              day={day}
                              class="calendar-cell-trigger"
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
      <Show when={props.onRemove && props.value()}>
        <div class="deadline-dialog-footer">
          <button
            type="button"
            class="deadline-dialog-remove"
            onClick={() => {
              props.onRemove?.();
              props.setOpen(false);
            }}
          >
            {labels().remove}
          </button>
        </div>
      </Show>
    </>
  );
}
