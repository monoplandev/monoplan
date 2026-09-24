// The planned time row shared by the task dialog's dates band
// (`WhenField`) and the row-level `when` modal (`CalendarPicker`): the
// typed start picker (`TimePicker`) reading a dim "All day" while no time
// is set, an inset ✕ that strips the time part, and, once a start exists,
// an end picker after an arrow. The end is derived: start + the stored
// `duration` (a length, not an end, so moving the start keeps it). Typing
// an end writes the difference in minutes; an end at or before the start
// on the clock means the next day; clearing it removes the duration.
//
// The host owns the time (the register's time part, or a value held
// locally until a date is picked) and the duration; the row only ever
// hands back a complete hour + minute or null.

import { Show } from "solid-js";
import {
  durationBetween,
  endTimeOf,
  formatDurationShort,
  hourCycle,
  type TimeParts,
} from "./format.tsx";
import arrowRightSvg from "./icons/arrow-right.svg?raw";
import clockSvg from "./icons/clock.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { TimePicker } from "./TimePicker.tsx";

export function WhenTimeRow(props: {
  time: () => Required<TimeParts> | null;
  onTimeChange: (t: Required<TimeParts> | null) => void;
  /** Stored duration in minutes, or null. */
  duration: () => number | null;
  onDurationChange: (minutes: number | null) => void;
}) {
  const { m, locale } = useAppI18n();

  const end = () => {
    const start = props.time();
    const d = props.duration();
    return start && d ? endTimeOf(start, d) : null;
  };
  const onEndChange = (t: Required<TimeParts> | null) => {
    const start = props.time();
    if (!t || !start) {
      props.onDurationChange(null);
      return;
    }
    props.onDurationChange(durationBetween(start, t));
  };

  return (
    <div class="task-dialog-time-row">
      {/* Clock glyph inset in the input like the date glyph; the input
          carries the accessible name. */}
      <TimePicker
        class="time-picker-input"
        icon={clockSvg}
        value={props.time}
        onChange={props.onTimeChange}
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
        <Show when={props.time()}>
          <button
            type="button"
            class="icon-button time-picker-clear"
            aria-label={m().when.clearTime}
            title={m().when.clearTime}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => props.onTimeChange(null)}
          >
            ✕
          </button>
        </Show>
      </TimePicker>
      {/* End field, shown once a start time is set. The arrow glyph is
          inset in its left edge, like the clock in the start field. */}
      <Show when={props.time()}>
        <TimePicker
          class="time-picker-input task-dialog-end-input"
          icon={arrowRightSvg}
          value={end}
          onChange={onEndChange}
          cycle={() => hourCycle(locale())}
          locale={locale}
          label={m().when.end}
          placeholder={() => m().when.end}
          after={props.time}
          optionHint={(t) => {
            const start = props.time();
            return start ? formatDurationShort(durationBetween(start, t)) : null;
          }}
        />
      </Show>
    </div>
  );
}
