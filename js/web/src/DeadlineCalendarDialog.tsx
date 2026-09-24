// corvu's headless `@corvu/calendar` as a picker for a deadline or a
// planned date (`kind`). `CalendarPicker` is the body (grid, optional time
// field, Remove footer); `DeadlineCalendarDialog` wraps it in a centered
// modal, fully controlled + triggerless so it can be driven from anywhere
// (the task dialog's deadline badge/menu, a list/board row's context menu).
// A Kobalte Dialog rather than a Popover there: opened from a closing menu,
// a popover fights the menu's focus-restore and instantly dismisses; a
// modal doesn't. The task dialog's date input hosts the same body in a
// Popover of its own (`WhenField.tsx`), where no menu is involved.
//
// In `when` mode the typed time picker (`TimePicker`, the task dialog's
// control) sits under the grid, reading a dim "All day" while no time is
// set; an inset ✕ strips the time part. The picker only ever commits a
// complete hour + minute or null, so every edit against an already-set
// date writes through and stays open; with no date yet the time is held
// locally and a date pick applies it as it closes
// (`spec/calendar-plan.md` "Task surface and rows").

import Calendar from "@corvu/calendar";
import { Dialog } from "@kobalte/core/dialog";
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  Show,
} from "solid-js";
import {
  hourCycle,
  isCompleteTime,
  localDateStamp,
  parseLocalDateParts,
  whenDay,
  whenFromParts,
  whenTime,
  type TimeParts,
} from "./format.tsx";
import clockSvg from "./icons/clock.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems } from "./overlay.ts";
import { TimePicker } from "./TimePicker.tsx";

export interface CalendarPickerProps {
  /** Whether the host surface is showing; the time field reseeds from the
   *  register on each open. */
  open: () => boolean;
  setOpen: (v: boolean) => void;
  /** `deadline` (default): date-only, `YYYY-MM-DD`. `when`: date plus an
   *  optional time, `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`. Picks labels and
   *  whether the time field renders. */
  kind?: "deadline" | "when";
  /** Currently-set register value to preselect / open the calendar on, or
   *  null. */
  value: () => string | null;
  /** Fired with the picked register value (never null — removal is the
   *  button below). A date pick closes the host after; in `when` mode a
   *  complete-or-empty time edit against a set date also fires, without
   *  closing. */
  onPick: (stamp: string) => void;
  /** Clear the value. When provided and a value is set, a "Remove" button
   *  shows at the bottom. */
  onRemove?: () => void;
  /** `when` mode only: render the time field under the grid (default on).
   *  Off where the host edits the time elsewhere (the task dialog's dates
   *  band); a date pick still keeps the stored time. */
  withTime?: boolean;
}

/** The picker body: month grid, the `when` time field, the Remove footer.
 *  Owns no surface; the host (modal or popover) supplies it. */
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

  // The time under the grid, reseeded from the register each time the
  // host opens so a reopen shows the stored time (or "All day"). The
  // picker hands back a complete time or null, never a partial state.
  const [time, setTime] = createSignal<Required<TimeParts> | null>(null);
  createEffect(
    on(props.open, (open) => {
      if (!open) return;
      const t = whenTime(props.value() ?? "");
      setTime(t && isCompleteTime(t) ? t : null);
    }),
  );
  // Write through only when there is a date to attach the time to;
  // otherwise it waits for the date pick below.
  const onTimeChange = (t: Required<TimeParts> | null) => {
    setTime(t);
    const cur = props.value();
    if (cur) props.onPick(whenFromParts(whenDay(cur), t));
  };

  const monthLabelFmt = createMemo(
    () => new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" }),
  );
  const weekdayFmt = createMemo(
    () => new Intl.DateTimeFormat(locale(), { weekday: "short" }),
  );

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
                      <Calendar.HeadCell class="calendar-headcell">
                        {weekdayFmt().format(weekday)}
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
      <Show when={isWhen() && props.withTime !== false}>
        <div class="deadline-dialog-time">
          <TimePicker
            class="time-picker-input"
            icon={clockSvg}
            value={time}
            onChange={onTimeChange}
            cycle={() => hourCycle(locale())}
            locale={locale}
            label={m().when.time}
            placeholder={() => m().when.allDay}
          >
            {/* Inset ✕ at the input's right edge while a time is set:
                back to all-day, the date kept. mousedown is cancelled so
                the click never blurs a focused picker under it. */}
            <Show when={time()}>
              <button
                type="button"
                class="icon-button time-picker-clear"
                aria-label={m().when.clearTime}
                title={m().when.clearTime}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => onTimeChange(null)}
              >
                ✕
              </button>
            </Show>
          </TimePicker>
        </div>
      </Show>
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

export function DeadlineCalendarDialog(
  props: CalendarPickerProps & {
    /** Send focus to the items listbox on close (the workspace-level
     *  mounts, opened from a row). Off for the pickers nested in the task
     *  surface, where Kobalte's default return-to-opener lands on the
     *  field button. */
    closeToItems?: boolean;
  },
) {
  const { m } = useAppI18n();
  const title = () =>
    (props.kind === "when" ? m().when : m().deadline).dialogTitle;
  return (
    <Dialog open={props.open()} onOpenChange={props.setOpen} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay deadline-dialog-overlay" />
        <div class="dialog-positioner deadline-dialog-positioner">
          <Dialog.Content
            class="deadline-dialog"
            onCloseAutoFocus={props.closeToItems ? closeToItems : undefined}
          >
            <Dialog.Title class="deadline-dialog-title">{title()}</Dialog.Title>
            <CalendarPicker {...props} />
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
}
