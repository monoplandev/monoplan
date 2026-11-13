// The task dialog's planned-date control: a text input in the dates band
// that reads the set `when` as a day ("Wed 23 Sept"; "Date" as its
// placeholder when unset) and opens a popover: Today / Tomorrow quick
// actions on top (they keep any time part and close), then the shared
// calendar picker (`CalendarPicker`; Remove). The input is typed into as
// well (`dateTyping.tsx`): once what is typed names a date
// (`dateSuggest.ts`, over the title tokens' parser) the popover's body
// shows a listbox above the calendar of the first few dates it could
// mean. Today / Tomorrow are the defaults when no date is set, and the
// highlighted suggestion previews its day in the calendar. Typed times offer clock-time choices; date-only rows keep any
// existing time part, as a calendar pick does. Enter on
// emptied text removes the date.
// The time row (`WhenTimeRow`: start picker, ✕, end picker) sits below
// the input and shows only once a date is set: without one the section
// is just the date input, and there is no day for a time to belong to.
// The date input has an inset ✕ while a `when` is set, which removes the
// whole value, time included, as the popover's Remove does. The popover
// carries no time picker of its own. Its open state is the caller's, so a
// row context menu's "Set date…" can open the item straight onto it.
//
// Once a date is set, an end-date input sits after the date input, over
// the end time, with the same arrow glyph: the day the span ends on,
// start day + `spanEndOffset`. While that is the start day itself (no
// span) it reads dim, like a placeholder: the end is implied, not
// stored. It is the second way to write the stored `duration` (the end
// time below is the first). Beside a timed start, picking a day keeps the
// end's clock time and writes the length to it; beside an all-day date
// the length counts whole days (Tue–Thu is three days), and picking the
// start day drops it. Its own popover is a plain calendar, no quick
// actions or Remove: the end is never absent, only implied. It is typed
// into like the start input, with phrases read from the start day once
// that is ahead of today and unreachable days left out of the list.
// Enter on emptied text drops the end date: the span comes back onto the
// start day, keeping the end's clock time where that is after the start
// and falling back to the default slot where it is not. Days the
// register cannot reach are dimmed and inert: before the start day; with
// a timed start, the start day itself while the end is not past the start
// on the clock (that end would be a day later, not earlier); and past the
// one-year ceiling.

import { Popover } from "@kobalte/core/popover";
import { createMemo, createSignal, Show } from "solid-js";
import { CalendarPicker } from "./CalendarPicker.tsx";
import { createDateTyping, DateSuggestList } from "./dateTyping.tsx";
import {
  addDaysToStamp,
  allDayDurationFor,
  calendarDayDiff,
  durationBetween,
  durationForTimedStart,
  endTimeOf,
  formatWhenTime,
  isValidEndDayOffset,
  MAX_ALL_DAY_OFFSET,
  nowMs,
  sameDayDuration,
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
  let contentRef: HTMLElement | undefined;
  let endContentRef: HTMLElement | undefined;
  const [endOpen, setEndOpen] = createSignal(false);

  const pickDay = (day: string) => {
    props.onChange(whenFromParts(day, whenTime(props.when() ?? "")));
    props.setOpen(false);
  };

  // The typed side of the start input (`dateTyping.tsx`): its list stands
  // above the calendar while the text names a date.
  const typing = createDateTyping({
    open: props.open,
    setOpen: props.setOpen,
    input: () => inputRef,
    content: () => contentRef,
    reference: () => new Date(nowMs()),
    locale,
    includeTime: true,
    defaults: () => props.when() ? [] : [todayStamp(nowMs()), addDaysToStamp(todayStamp(nowMs()), 1)],
    onPick: (value) => {
      const t = whenTime(value);
      if (!t) {
        pickDay(value);
        return;
      }
      const wasAllDay = !whenTime(props.when() ?? "");
      const d = props.duration();
      props.onChange(value);
      if (wasAllDay) {
        const reshaped = durationForTimedStart(d);
        if (reshaped !== d) props.onDurationChange(reshaped);
      }
    },
    onClear: () => {
      if (props.when()) props.onChange(null);
    },
  });
  const todayOrTomorrow = (value: string) => {
    const day = whenDay(value);
    const today = todayStamp(nowMs());
    if (day === today) return m().when.today;
    return day === addDaysToStamp(today, 1) ? m().when.tomorrow : null;
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

  // The inputs read the day only ("Wed 23 Sept"; in another year the
  // year takes the weekday's place, "23 Sept 2027") rather than the badge's relative labels: a field should say
  // what is stored, and the time has its own row below. Judged against
  // the shared `nowMs()` tick so the year appears at the turn of the year
  // without a reload.
  const formatDay = (stamp: string) => {
    const date = parseLocalDateParts(stamp);
    if (!date) return stamp;
    const thisYear =
      date.getFullYear() ===
      parseLocalDateParts(todayStamp(nowMs()))?.getFullYear();
    return new Intl.DateTimeFormat(locale(), {
      day: "numeric",
      month: "short",
      ...(thisYear ? { weekday: "short" } : { year: "numeric" }),
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

  // The end input is typed into the same way. Phrases are read from the
  // start day once that is ahead of today, so "friday" is the Friday the
  // span could end on rather than one before it begins; days the register
  // cannot reach (as the calendar dims them) are left out of the list.
  const endTyping = createDateTyping({
    open: endOpen,
    setOpen: setEndOpen,
    input: () => endInputRef,
    content: () => endContentRef,
    reference: () => {
      const now = new Date(nowMs());
      const w = props.when();
      const start = w ? parseLocalDateParts(whenDay(w)) : null;
      return start && start > now ? start : now;
    },
    locale,
    allowed: (stamp) => {
      const day = parseLocalDateParts(stamp);
      return day !== null && !endDayDisabled(day);
    },
    onPick: pickEndDay,
    // Emptied and entered: drop the end date, back onto the start day.
    onClear: () => {
      const d = props.duration();
      const next = sameDayDuration(time(), d);
      if (next !== d) props.onDurationChange(next);
    },
  });

  return (
    <>
      {/* Start date, and once one is set the end's day beside it: the row
          splits into the time row's two columns so the end date sits
          over the end time. */}
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
              {...typing.inputProps}
              class="task-dialog-when-input"
              value={typing.query() ?? label()}
              placeholder={m().when.placeholder}
              aria-label={m().when.label}
              data-muted={props.muted() ? "" : undefined}
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
              ref={contentRef}
              class="deadline-dialog when-popover"
              // Focus belongs to the input, so typing works the moment the
              // popover is up, whichever way it was opened.
              onOpenAutoFocus={(e) => {
                e.preventDefault();
                inputRef?.focus();
              }}
              // A click on the input while open would otherwise dismiss on
              // pointerdown and reopen on click; keep it open instead. The
              // same for focus arriving in the input.
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
              <Show when={typing.listShown()}>
                <DateSuggestList
                  typing={typing}
                  format={(value) => {
                    if (!typing.query()?.trim()) return todayOrTomorrow(value) ?? formatDay(value);
                    const day = formatDay(whenDay(value));
                    return whenTime(value) ? `${day} ${formatWhenTime(value, locale())}` : day;
                  }}
                  hint={(value) => typing.query()?.trim() ? todayOrTomorrow(value) : formatDay(value)}
                />
              </Show>
              <CalendarPicker
                kind="when"
                setOpen={props.setOpen}
                value={props.when}
                preview={typing.preview}
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
                {...endTyping.inputProps}
                class="task-dialog-when-input task-dialog-end-date-input"
                value={endTyping.query() ?? endLabel()}
                aria-label={m().when.endDate}
                data-muted={props.muted() ? "" : undefined}
                // Dim only while it shows the implied day, not typed text.
                data-implied={
                  endOffset() === 0 && endTyping.query() === null
                    ? ""
                    : undefined
                }
              />
            </Popover.Anchor>
            <Popover.Portal>
              <Popover.Content
                ref={endContentRef}
                class="deadline-dialog when-popover"
                onOpenAutoFocus={(e) => {
                  e.preventDefault();
                  endInputRef?.focus();
                }}
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
                <Show when={endTyping.listShown()}>
                  <DateSuggestList
                    typing={endTyping}
                    format={formatDay}
                    hint={todayOrTomorrow}
                  />
                </Show>
                <CalendarPicker
                  setOpen={setEndOpen}
                  value={endDay}
                  preview={endTyping.preview}
                  onPick={pickEndDay}
                  disabled={endDayDisabled}
                />
              </Popover.Content>
            </Popover.Portal>
          </Popover>
        </Show>
      </div>
      {/* Time row, only once there is a date for the time to sit on. */}
      <Show when={props.when()}>
        <WhenTimeRow
          time={time}
          onTimeChange={onTimeChange}
          duration={props.duration}
          onDurationChange={props.onDurationChange}
        />
      </Show>
    </>
  );
}
