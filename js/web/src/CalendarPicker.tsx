// The shared corvu month grid (`MonthGrid.tsx`) as a picker for a
// deadline or a planned date (`kind`). `CalendarPicker` is the body (grid, Remove
// footer), hosted in a Popover by the task surface's date and deadline
// fields (`WhenField.tsx`, `DeadlineField.tsx`). There is no standalone
// modal: a row's "Set date…" / "Set deadline…" opens the item with the
// matching popover showing, and the time is edited in the task surface's
// dates band (`WhenTimeRow`), not here.

import { createEffect, createMemo, createSignal, Show } from "solid-js";
import {
  localDateStamp,
  nowMs,
  parseLocalDateParts,
  whenDay,
  whenFromParts,
  whenTime,
} from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import { MonthGrid } from "./MonthGrid.tsx";

export interface CalendarPickerProps {
  setOpen: (v: boolean) => void;
  /** `deadline` (default): date-only, `YYYY-MM-DD`. `when`: date plus an
   *  optional time, `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`. Picks labels and
   *  whether a pick carries the stored time along. */
  kind?: "deadline" | "when";
  /** Currently-set register value to preselect / open the calendar on, or
   *  null. */
  value: () => string | null;
  /** A suggestion under the cursor, shown without committing it. */
  preview?: () => string | null;
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
  const { m } = useAppI18n();
  const isWhen = () => props.kind === "when";

  // Register stamp ⇄ Date on the boundary — the register stores a floating
  // local date; corvu works in `Date`s. Never `new Date(stamp)` (that's UTC
  // and shifts the day in negative-offset zones).
  const value = createMemo<Date | null>(() => {
    const s = props.value();
    return s ? parseLocalDateParts(whenDay(s)) : null;
  });

  const previewDay = createMemo(() => {
    const stamp = props.preview?.();
    return stamp ? whenDay(stamp) : null;
  });
  const [month, setMonth] = createSignal(value() ?? new Date(nowMs()));
  createEffect(() => {
    const stamp = previewDay();
    const day = stamp ? parseLocalDateParts(stamp) : null;
    if (day) setMonth(day);
  });

  // The stored time part a `when` pick carries over to the new day.
  const time = () => whenTime(props.value() ?? "");

  const labels = () => (isWhen() ? m().when : m().deadline);

  return (
    <>
      <MonthGrid
        value={value()}
        month={month()}
        onMonthChange={setMonth}
        disabled={props.disabled}
        previewDay={previewDay()}
        onPick={(d) => {
          if (d) {
            const day = localDateStamp(d);
            props.onPick(isWhen() ? whenFromParts(day, time()) : day);
          }
          props.setOpen(false);
        }}
      />
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
