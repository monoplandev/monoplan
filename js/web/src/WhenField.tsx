// The task dialog's planned-date control: a text input in the dates band
// that reads the set `when` as a day ("Wed 23 Sept"; "Date" as its
// placeholder when unset) and opens a popover: Today / Tomorrow quick
// actions on top (they keep any time part and close), then the shared
// calendar picker (`CalendarPicker`, without its time field; Remove). The input is read-only for now: typed dates are a later step,
// so the popover is the only writer. Anchored on the input rather than a
// Popover.Trigger button so it can become editable without changing shape.
// The typed time picker (`TimePicker`) sits on a row above the input and
// is always shown: a dim placeholder when the `when` has no time part
// ("All day" against a set date, "Time" while there is no date for it to
// be all of), and a ✕ that strips one. The date input carries the same
// inset ✕ while a `when` is set; it removes the whole value, time
// included, as the popover's Remove does. A time picked while no date is
// set lands on today. The popover carries no time field of its own.
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
  // so every change writes straight through. Without a date yet, a time
  // is attached to today.
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
          placeholder={() => (props.when() ? m().when.allDay : m().when.time)}
          clearLabel={m().when.clearTime}
        />
        {/* Arrow to the end field; both only once a start time is set. */}
        <Show when={time()}>
          <span
            class="task-dialog-time-arrow"
            aria-hidden="true"
            innerHTML={arrowRightSvg}
          />
          <TimePicker
            class="task-dialog-time-input task-dialog-end-input"
            value={end}
            onChange={onEndChange}
            cycle={() => hourCycle(locale())}
            locale={locale}
            label={m().when.end}
            placeholder={() => m().when.end}
            clearLabel={m().when.clearEnd}
          />
        </Show>
      </div>
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
            {/* Same inset ✕ as the time field. mousedown is cancelled so
                the click never moves focus off the input. */}
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
