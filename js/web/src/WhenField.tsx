// The task dialog's planned-date control: a text input in the dates band
// that reads the set `when` as a day ("Wed 23 Sept"; "Date" as its
// placeholder when unset) and opens a popover: Today / Tomorrow quick
// actions on top (they keep any time part and close), then the shared
// calendar picker (`CalendarPicker`, without its time field; Remove). The input is read-only for now: typed dates are a later step,
// so the popover is the only writer. Anchored on the input rather than a
// Popover.Trigger button so it can become editable without changing shape.
// The typed time picker (`TimePicker`) sits on a row above the input and
// shows only once a date is set: without one the section is just the
// date input, and there is no day for a time to belong to. Against a set
// date it reads a dim "All day" placeholder while the `when` has no time
// part. A single ✕ after the end field strips the time part (back to
// all-day); the date input has an inset ✕ while a `when` is set, which
// removes the whole value, time included, as the popover's Remove does.
// The popover carries no time field of its own.
// After the arrow, a second picker reads the end time: the start plus the
// stored `duration` (a length, not an end, so moving the start keeps it).
// Typing an end writes the difference in minutes; an end at or before the
// start on the clock means the next day. It renders only once a start
// time exists, since without one there is nothing to be after.

import { Popover } from "@kobalte/core/popover";
import { createMemo, Show } from "solid-js";
import { CalendarPicker } from "./DeadlineCalendarDialog.tsx";
import {
  addDaysToStamp,
  durationBetween,
  endTimeOf,
  formatDurationShort,
  hourCycle,
  isCompleteTime,
  nowMs,
  parseLocalDateParts,
  todayStamp,
  whenDay,
  whenFromParts,
  whenTime,
} from "./format.tsx";
import arrowRightSvg from "./icons/arrow-right.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import clockSvg from "./icons/clock.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { TimePicker } from "./TimePicker.tsx";

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

  const pickDay = (day: string) => {
    props.onChange(whenFromParts(day, whenTime(props.when() ?? "")));
    props.setOpen(false);
  };

  // The picker hands back a complete hour + minute, or null for all-day,
  // so every change writes straight through. The row only renders against
  // a set date; today is a fallback for the type, not a path taken.
  const time = () => {
    const t = whenTime(props.when() ?? "");
    return t && isCompleteTime(t) ? t : null;
  };
  const onTimeChange = (t: { hour: number; minute: number } | null) => {
    const w = props.when();
    const day = w ? whenDay(w) : todayStamp(nowMs());
    props.onChange(whenFromParts(day, t));
  };

  // The end field is derived: start + duration. Committing an end stores
  // the length back; clearing it removes the duration.
  const end = () => {
    const start = time();
    const d = props.duration();
    return start && d ? endTimeOf(start, d) : null;
  };
  const onEndChange = (t: { hour: number; minute: number } | null) => {
    const start = time();
    if (!t || !start) {
      props.onDurationChange(null);
      return;
    }
    props.onDurationChange(durationBetween(start, t));
  };

  // The input reads the day only ("Wed 23 Sept", the year once it isn't
  // this one) rather than the badge's relative labels: a field should say
  // what is stored, and the time has its own row above. Judged against
  // the shared `nowMs()` tick so the year appears at the turn of the year
  // without a reload.
  const label = createMemo(() => {
    const w = props.when();
    if (!w) return "";
    const date = parseLocalDateParts(whenDay(w));
    if (!date) return w;
    const thisYear =
      date.getFullYear() ===
      parseLocalDateParts(todayStamp(nowMs()))?.getFullYear();
    const day = new Intl.DateTimeFormat(locale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
      ...(thisYear ? {} : { year: "numeric" }),
    }).format(date);
    return day;
  });

  return (
    <>
      {/* Time row, only once there is a date for the time to sit on. */}
      <Show when={props.when()}>
        <div class="task-dialog-time-row">
          {/* Clock glyph inset in the input like the date glyph below; the
              input carries the accessible name. */}
          <TimePicker
            class="task-dialog-time-input"
            icon={clockSvg}
            value={time}
            onChange={onTimeChange}
            cycle={() => hourCycle(locale())}
            locale={locale}
            label={m().when.time}
            placeholder={() => m().when.allDay}
          >
            {/* Inset ✕ at the start field's right edge, shown on hover like
                the date input's: strips the time part, making the item
                all-day (the core keeps the duration, so re-adding a time
                restores the end). mousedown is cancelled so the click never
                blurs a focused picker under it. */}
            <Show when={time()}>
              <button
                type="button"
                class="icon-button task-dialog-time-clear"
                aria-label={m().when.clearTime}
                title={m().when.clearTime}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => onTimeChange(null)}
              >
                ✕
              </button>
            </Show>
          </TimePicker>
          {/* End field, shown once a start time is set. The arrow glyph is
              inset in its left edge, like the clock in the start field. */}
          <Show when={time()}>
            <TimePicker
              class="task-dialog-time-input task-dialog-end-input"
              icon={arrowRightSvg}
              value={end}
              onChange={onEndChange}
              cycle={() => hourCycle(locale())}
              locale={locale}
              label={m().when.end}
              placeholder={() => m().when.end}
              after={time}
              optionHint={(t) => {
                const start = time();
                return start ? formatDurationShort(durationBetween(start, t)) : null;
              }}
            />
          </Show>
        </div>
      </Show>
      <div class="task-dialog-dates-row">
        <Popover
          open={props.open()}
          onOpenChange={props.setOpen}
          placement="bottom-start"
          gutter={6}
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
              onKeyDown={(e) => {
                if (
                  e.key === "Enter" ||
                  e.key === " " ||
                  e.key === "ArrowDown"
                ) {
                  e.preventDefault();
                  props.setOpen(true);
                }
              }}
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
                open={props.open}
                setOpen={props.setOpen}
                value={props.when}
                onPick={props.onChange}
                onRemove={() => props.onChange(null)}
                withTime={false}
              />
            </Popover.Content>
          </Popover.Portal>
        </Popover>
      </div>
    </>
  );
}
