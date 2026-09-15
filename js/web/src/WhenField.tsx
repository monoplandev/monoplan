// The task dialog's planned-date control: a text input in the dates band
// that reads the set `when` as a day ("Wed 23 Sept"; "Date" as its
// placeholder when unset) and opens a popover: Today / Tomorrow quick
// actions on top (they keep any time part and close), then the shared
// calendar picker (`CalendarPicker`, without its time field; Remove). The input is read-only for now: typed dates are a later step,
// so the popover is the only writer. Anchored on the input rather than a
// Popover.Trigger button so it can become editable without changing shape.
// An "All day" switch sits at the band's right edge: on strips the time
// part, off attaches a default 09:00. Timed values also show a segmented
// time field on a row above the input and switch, writing through on each
// complete edit; the popover carries no time field of its own.

import { Popover } from "@kobalte/core/popover";
import { Switch } from "@kobalte/core/switch";
import { TimeField } from "@kobalte/core/time-field";
import { createEffect, createMemo, createSignal, on, Show } from "solid-js";
import { CalendarPicker } from "./DeadlineCalendarDialog.tsx";
import {
  addDaysToStamp,
  hourCycle,
  isCompleteTime,
  nowMs,
  parseLocalDateParts,
  todayStamp,
  whenDay,
  whenFromParts,
  whenTime,
  type TimeParts,
} from "./format.tsx";
import calendarSvg from "./icons/calendar.svg?raw";
import clockSvg from "./icons/clock.svg?raw";
import { useAppI18n } from "./i18n.tsx";

/** Time attached when "All day" is switched off; the popover's time field
 *  adjusts it from there. */
const DEFAULT_TIME = { hour: 9, minute: 0 };

export function WhenField(props: {
  when: () => string | null;
  muted: () => boolean;
  onChange: (value: string | null) => void;
  open: () => boolean;
  setOpen: (v: boolean) => void;
}) {
  const { m, locale } = useAppI18n();
  let inputRef: HTMLInputElement | undefined;

  const pickDay = (day: string) => {
    props.onChange(whenFromParts(day, whenTime(props.when() ?? "")));
    props.setOpen(false);
  };

  const allDay = () => {
    const w = props.when();
    return !w || whenTime(w) === null;
  };

  // Time field state, reseeded whenever the register's time changes (a
  // switch flip, a sync from another device). Partial states stay local:
  // only a complete hour + minute writes through, so the stored time is
  // never torn mid-edit and blanking the segments doesn't flip to all-day
  // under the user (that is the switch's job).
  const [time, setTime] = createSignal<TimeParts>({});
  createEffect(
    on(
      () => whenTime(props.when() ?? ""),
      (t) => setTime(t ?? {}),
    ),
  );
  const onTimeChange = (v: { hour?: number; minute?: number } | null) => {
    const next: TimeParts = { hour: v?.hour, minute: v?.minute };
    setTime(next);
    const w = props.when();
    if (w && isCompleteTime(next)) {
      props.onChange(whenFromParts(whenDay(w), next));
    }
  };
  const setAllDay = (on: boolean) => {
    const w = props.when();
    if (!w) return;
    props.onChange(on ? whenDay(w) : whenFromParts(whenDay(w), DEFAULT_TIME));
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
      <Show when={!allDay()}>
        <div class="task-dialog-time-row">
          <TimeField
            class="time-field"
            value={time()}
            hourCycle={hourCycle(locale())}
            granularity="minute"
            onChange={onTimeChange}
          >
            {/* Clock glyph in the date glyph's slot; the text stays as
                the accessible name only. */}
            <TimeField.Label class="time-field-label task-dialog-time-icon">
              <span aria-hidden="true" innerHTML={clockSvg} />
              <span class="sr-only">{m().when.time}</span>
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
        <Switch
          class="done-switch task-dialog-allday-switch"
          checked={allDay()}
          disabled={!props.when()}
          onChange={setAllDay}
        >
          <Switch.Label class="done-switch-label">
            {m().when.allDay}
          </Switch.Label>
          <Switch.Input class="done-switch-input" />
          <Switch.Control class="done-switch-control">
            <Switch.Thumb class="done-switch-thumb" />
          </Switch.Control>
        </Switch>
      </div>
    </>
  );
}
