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
// In `when` mode a Kobalte `TimeField` sits under the grid. Blank means
// all-day; a "Clear time" button beside it blanks both segments at once.
// The field fires on every segment edit, including partial states, so it
// is held locally and written through only when complete (both segments)
// or empty (neither): a date pick applies the last such state and closes;
// a time edit against an already-set date writes through and stays open
// (`spec/calendar-plan.md` "Task surface and rows").

import Calendar from "@corvu/calendar";
import { Dialog } from "@kobalte/core/dialog";
import { TimeField } from "@kobalte/core/time-field";
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
  isEmptyTime,
  localDateStamp,
  parseLocalDateParts,
  whenDay,
  whenFromParts,
  whenTime,
  type TimeParts,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems } from "./overlay.ts";

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

  // Time field state, reseeded from the register each time the host
  // opens so a reopen shows the stored time (or blank for all-day).
  const [time, setTime] = createSignal<TimeParts>({});
  createEffect(
    on(props.open, (open) => {
      if (open) setTime(whenTime(props.value() ?? "") ?? {});
    }),
  );
  const settledTime = (): TimeParts | null =>
    isCompleteTime(time()) ? time() : null;

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
            props.onPick(isWhen() ? whenFromParts(day, settledTime()) : day);
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
          <TimeField
            class="time-field"
            value={time()}
            hourCycle={hourCycle(locale())}
            granularity="minute"
            onChange={(v) => {
              const next: TimeParts = { hour: v?.hour, minute: v?.minute };
              setTime(next);
              // Write through only from a settled state, and only
              // when there is a date to attach it to.
              const cur = props.value();
              if (!cur) return;
              if (isCompleteTime(next)) {
                props.onPick(whenFromParts(whenDay(cur), next));
              } else if (isEmptyTime(next)) {
                props.onPick(whenDay(cur));
              }
            }}
          >
            <TimeField.Label class="time-field-label">
              {m().when.time}
            </TimeField.Label>
            <TimeField.Input class="time-field-input">
              {(segment) => (
                <TimeField.Segment
                  class="time-field-segment"
                  segment={segment()}
                />
              )}
            </TimeField.Input>
          </TimeField>
          <Show when={!isEmptyTime(time())}>
            <button
              type="button"
              class="deadline-dialog-remove"
              onClick={() => {
                setTime({});
                const cur = props.value();
                if (cur) props.onPick(whenDay(cur));
              }}
            >
              {m().when.clearTime}
            </button>
          </Show>
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
