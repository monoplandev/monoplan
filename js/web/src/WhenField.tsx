// The task dialog's planned-date control: a text input in the dates band
// that reads the set `when` as a day ("Wed 23 Sept"; "Date" as its
// placeholder when unset) and opens a popover: Today / Tomorrow quick
// actions on top (they keep any time part and close), then the shared
// calendar picker (`CalendarPicker`; Remove). The input is read-only for now: typed dates are a later step,
// so the popover is the only writer. Anchored on the input rather than a
// Popover.Trigger button so it can become editable without changing shape.
// The time row (`WhenTimeRow`: start picker, ✕, end picker) sits above
// the input and shows only once a date is set: without one the section
// is just the date input, and there is no day for a time to belong to.
// The date input has an inset ✕ while a `when` is set, which removes the
// whole value, time included, as the popover's Remove does. The popover
// carries no time picker of its own. Its open state is the caller's, so a
// row context menu's "Set date…" can open the item straight onto it.
//
// Once a date is set, an end-date input sits after the date input, under
// the end time, with the same arrow glyph: the day the span ends on,
// start day + `spanEndOffset`. While that is the start day itself (no
// span) it reads dim, like a placeholder: the end is implied, not
// stored. It is the second way to write the stored `duration` (the end
// time above is the first). Beside a timed start, picking a day keeps the
// end's clock time and writes the length to it; beside an all-day date
// the length counts whole days (Tue–Thu is three days), and picking the
// start day drops it. Its own popover is a plain calendar, no quick
// actions or Remove: the end is never absent, only implied. Days the
// register cannot reach are dimmed and inert: before the start day; with
// a timed start, the start day itself while the end is not past the start
// on the clock (that end would be a day later, not earlier); and past the
// one-week ceiling.

import { Popover } from "@kobalte/core/popover";
import { createMemo, createSignal, Show } from "solid-js";
import { CalendarPicker } from "./DeadlineCalendarDialog.tsx";
import {
  addDaysToStamp,
  allDayDurationFor,
  calendarDayDiff,
  durationBetween,
  durationForTimedStart,
  endTimeOf,
  isValidEndDayOffset,
  MAX_ALL_DAY_OFFSET,
  nowMs,
  spanEndOffset,
  parseLocalDateParts,
  todayStamp,
  whenDay,
  whenFromParts,
  whenTime,
} from "./format.tsx";
import arrowRightSvg from "./icons/arrow-right.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { WhenTimeRow } from "./WhenTimeRow.tsx";

export function WhenField(props: {
  when: () => string | null;
  /** Stored duration in minutes, or null. */
  duration: () => number | null;
  muted: () => boolean;
  onChange: (value: string | null) => void;
  onDurationChange: (minutes: number | null) => void;
  open: () => boolean;
  setOpen: (v: boolean) => void;
}) {
  const { m, locale } = useAppI18n();
  let inputRef: HTMLInputElement | undefined;
  let endInputRef: HTMLInputElement | undefined;
  const [endOpen, setEndOpen] = createSignal(false);

  const pickDay = (day: string) => {
    props.onChange(whenFromParts(day, whenTime(props.when() ?? "")));
    props.setOpen(false);
  };

  // The picker hands back a complete hour + minute, or null for all-day,
  // so every change writes straight through. The row only renders against
  // a set date; today is a fallback for the type, not a path taken.
  const time = () => whenTime(props.when() ?? "");
  const onTimeChange = (t: { hour: number; minute: number } | null) => {
    const w = props.when();
    const day = w ? whenDay(w) : todayStamp(nowMs());
    const wasAllDay = !time();
    const d = props.duration();
    props.onChange(whenFromParts(day, t));
    // All-day → timed with a whole-day span: keep its last day rather
    // than letting the days-as-minutes run a day past it.
    if (t && wasAllDay) {
      const reshaped = durationForTimedStart(d);
      if (reshaped !== d) props.onDurationChange(reshaped);
    }
  };

  // The inputs read the day only ("Wed 23 Sept", the year once it isn't
  // this one) rather than the badge's relative labels: a field should say
  // what is stored, and the time has its own row above. Judged against
  // the shared `nowMs()` tick so the year appears at the turn of the year
  // without a reload.
  const formatDay = (stamp: string) => {
    const date = parseLocalDateParts(stamp);
    if (!date) return stamp;
    const thisYear =
      date.getFullYear() ===
      parseLocalDateParts(todayStamp(nowMs()))?.getFullYear();
    return new Intl.DateTimeFormat(locale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
      ...(thisYear ? {} : { year: "numeric" }),
    }).format(date);
  };
  const label = createMemo(() => {
    const w = props.when();
    return w ? formatDay(whenDay(w)) : "";
  });

  // Days past the start day the span ends on; 0 is the start day itself,
  // the implied end. Null without a date, when the field is not shown.
  const endOffset = createMemo(() => {
    const w = props.when();
    return w ? spanEndOffset(w, props.duration()) : null;
  });
  const endDay = createMemo(() => {
    const w = props.when();
    const o = endOffset();
    return w && o !== null ? addDaysToStamp(whenDay(w), o) : null;
  });
  const endLabel = createMemo(() => {
    const e = endDay();
    return e ? formatDay(e) : "";
  });
  // A day in the end picker, as days past the start day.
  const offsetOf = (day: Date) => {
    const w = props.when();
    const startDate = w ? parseLocalDateParts(whenDay(w)) : null;
    return startDate ? calendarDayDiff(day, startDate) : 0;
  };
  const endDayDisabled = (day: Date) => {
    const w = props.when();
    if (!w) return true;
    const start = time();
    const offset = offsetOf(day);
    if (!start) return offset < 0 || offset > MAX_ALL_DAY_OFFSET;
    const d = props.duration();
    if (!d) return true;
    return !isValidEndDayOffset(start, endTimeOf(start, d), offset);
  };
  // Picking an end day: with a timed start, keep the end's clock time and
  // write the length up to it (the picker already fenced off every day
  // that could not hold one, so the wrap in `durationBetween` never fires
  // here); all-day, write whole days, or drop the length for the start
  // day itself.
  const pickEndDay = (stamp: string) => {
    const day = parseLocalDateParts(stamp);
    if (!props.when() || !day) return;
    const offset = offsetOf(day);
    const start = time();
    if (!start) {
      props.onDurationChange(allDayDurationFor(offset));
      return;
    }
    const d = props.duration();
    if (!d) return;
    props.onDurationChange(durationBetween(start, endTimeOf(start, d), offset));
  };

  const openOnKey = (open: () => void) => (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
      e.preventDefault();
      open();
    }
  };

  return (
    <>
      {/* Time row, only once there is a date for the time to sit on. */}
      <Show when={props.when()}>
        <WhenTimeRow
          time={time}
          onTimeChange={onTimeChange}
          duration={props.duration}
          onDurationChange={props.onDurationChange}
        />
      </Show>
      {/* Start date, and once one is set the end's day beside it: the row
          splits into the time row's two columns so the end date sits
          under the end time. */}
      <div
        classList={{
          "task-dialog-dates-row": true,
          "task-dialog-dates-row-split": endDay() !== null,
        }}
      >
        <Popover
          open={props.open()}
          onOpenChange={props.setOpen}
          placement="bottom-start"
          gutter={6}
          // With no room below or above (a phone, a long title), slide
          // over the input rather than run off the screen: the popover is
          // portaled to <body>, which never scrolls, so an overflowing
          // Remove row could not be reached.
          overlap
        >
          <Popover.Anchor as="span" class="task-dialog-when-anchor">
            {/* Calendar glyph sits inside the input's left edge; the input
            pads past it. Decorative: the input carries the label. */}
            <span
              class="task-dialog-when-icon"
              aria-hidden="true"
              innerHTML={calendarSvg}
            />
            <input
              ref={inputRef}
              type="text"
              class="task-dialog-when-input"
              readOnly
              inputMode="none"
              value={label()}
              placeholder={m().when.placeholder}
              aria-label={m().when.label}
              aria-haspopup="dialog"
              aria-expanded={props.open()}
              data-muted={props.muted() ? "" : undefined}
              onClick={() => props.setOpen(true)}
              onKeyDown={openOnKey(() => props.setOpen(true))}
            />
            {/* Inset ✕ at the input's right edge. mousedown is cancelled
                so the click never moves focus off the input. */}
            <Show when={props.when()}>
              <button
                type="button"
                class="icon-button task-dialog-when-clear"
                aria-label={m().when.remove}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => {
                  props.onChange(null);
                  props.setOpen(false);
                }}
              >
                ✕
              </button>
            </Show>
          </Popover.Anchor>
          <Popover.Portal>
            <Popover.Content
              class="deadline-dialog when-popover"
              // A click on the input while open would otherwise dismiss on
              // pointerdown and reopen on click; keep it open instead.
              onInteractOutside={(e) => {
                if (
                  inputRef &&
                  e.target instanceof Node &&
                  inputRef.contains(e.target)
                ) {
                  e.preventDefault();
                }
              }}
              onCloseAutoFocus={(e) => {
                e.preventDefault();
                inputRef?.focus();
              }}
            >
              <div class="when-popover-quick">
                <button
                  type="button"
                  class="when-popover-quick-action"
                  onClick={() => pickDay(todayStamp(nowMs()))}
                >
                  {m().when.today}
                </button>
                <button
                  type="button"
                  class="when-popover-quick-action"
                  onClick={() =>
                    pickDay(addDaysToStamp(todayStamp(nowMs()), 1))
                  }
                >
                  {m().when.tomorrow}
                </button>
              </div>
              <CalendarPicker
                kind="when"
                setOpen={props.setOpen}
                value={props.when}
                onPick={props.onChange}
                onRemove={() => props.onChange(null)}
              />
            </Popover.Content>
          </Popover.Portal>
        </Popover>
        {/* End date; dim while it is only the start day again. */}
        <Show when={endDay()}>
          <Popover
            open={endOpen()}
            onOpenChange={setEndOpen}
            placement="bottom-start"
            gutter={6}
            overlap
          >
            <Popover.Anchor as="span" class="task-dialog-when-anchor">
              <span
                class="task-dialog-when-icon"
                aria-hidden="true"
                innerHTML={arrowRightSvg}
              />
              <input
                ref={endInputRef}
                type="text"
                class="task-dialog-when-input task-dialog-end-date-input"
                readOnly
                inputMode="none"
                value={endLabel()}
                aria-label={m().when.endDate}
                aria-haspopup="dialog"
                aria-expanded={endOpen()}
                data-muted={props.muted() ? "" : undefined}
                data-implied={endOffset() === 0 ? "" : undefined}
                onClick={() => setEndOpen(true)}
                onKeyDown={openOnKey(() => setEndOpen(true))}
              />
            </Popover.Anchor>
            <Popover.Portal>
              <Popover.Content
                class="deadline-dialog when-popover"
                onInteractOutside={(e) => {
                  if (
                    endInputRef &&
                    e.target instanceof Node &&
                    endInputRef.contains(e.target)
                  ) {
                    e.preventDefault();
                  }
                }}
                onCloseAutoFocus={(e) => {
                  e.preventDefault();
                  endInputRef?.focus();
                }}
              >
                <CalendarPicker
                  setOpen={setEndOpen}
                  value={endDay}
                  onPick={pickEndDay}
                  disabled={endDayDisabled}
                />
              </Popover.Content>
            </Popover.Portal>
          </Popover>
        </Show>
      </div>
    </>
  );
}
