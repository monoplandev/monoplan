// The task dialog's deadline control: a text input in its own ruled band
// under the dates band, the planned-date input (`WhenField`) without the
// time: a timer glyph inset at the left edge, the stored day read as
// "Wed 23 Sept" ("Deadline" as its placeholder when unset), an inset ✕
// that removes it, and a popover under it with the shared calendar picker
// (`CalendarPicker`, kind "deadline"; Remove). It is typed into the same
// way (`dateTyping.tsx`): once the text names a date the popover lists the
// first few days it could mean above the calendar, Today / Tomorrow being
// the list while no deadline is set, and the highlighted one previews its
// day in the calendar. A deadline is a day, so typed times are not read.
// Enter on emptied text removes it. Unlike the badge on rows and cards it
// shows the date rather than "Overdue" / "Today": a field says what is
// stored; the urgency shows as the text colour instead (`data-tone`),
// dropped while the item is done or binned (`muted`) and while the text
// is being typed.

import { Popover } from "@kobalte/core/popover";
import { createMemo, Show } from "solid-js";
import { CalendarPicker } from "./CalendarPicker.tsx";
import { createDateTyping, DateSuggestList } from "./dateTyping.tsx";
import {
  addDaysToStamp,
  formatDeadlineBadge,
  nowMs,
  parseLocalDateParts,
  todayStamp,
} from "./format.tsx";
import timerSvg from "./icons/timer.svg?raw";
import { useAppI18n } from "./i18n.tsx";

export function DeadlineField(props: {
  deadline: () => string | null;
  muted: () => boolean;
  onChange: (stamp: string | null) => void;
  open: () => boolean;
  setOpen: (v: boolean) => void;
}) {
  const { m, locale } = useAppI18n();
  let inputRef: HTMLInputElement | undefined;
  let contentRef: HTMLElement | undefined;

  // The typed side of the input (`dateTyping.tsx`), as the planned date's
  // but date-only: its list stands above the calendar while the text
  // names a day.
  const typing = createDateTyping({
    open: props.open,
    setOpen: props.setOpen,
    input: () => inputRef,
    content: () => contentRef,
    reference: () => new Date(nowMs()),
    locale,
    defaults: () =>
      props.deadline()
        ? []
        : [todayStamp(nowMs()), addDaysToStamp(todayStamp(nowMs()), 1)],
    onPick: (day) => props.onChange(day),
    onClear: () => {
      if (props.deadline()) props.onChange(null);
    },
  });
  const todayOrTomorrow = (day: string) => {
    const today = todayStamp(nowMs());
    if (day === today) return m().deadline.today;
    return day === addDaysToStamp(today, 1) ? m().deadline.tomorrow : null;
  };

  // The day as the planned-date input reads it ("Wed 23 Sept"; in another
  // year the year takes the weekday's place); judged against the shared
  // `nowMs()` tick so it rolls over without a reload.
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
    const d = props.deadline();
    return d ? formatDay(d) : "";
  });

  // Urgency for the text colour, from the same reading the row badge
  // uses; muted items carry no urgency, and neither does text still
  // being typed.
  const tone = createMemo(() => {
    if (props.muted()) return "muted";
    if (typing.query() !== null) return undefined;
    const d = props.deadline();
    if (!d) return undefined;
    const info = formatDeadlineBadge(
      d,
      todayStamp(nowMs()),
      {
        overdue: m().deadline.overdue,
        today: m().deadline.today,
        tomorrow: m().deadline.tomorrow,
      },
      locale(),
    );
    return info?.urgency ?? "future";
  });

  return (
    <div class="task-dialog-dates-row">
      <Popover
        open={props.open()}
        onOpenChange={props.setOpen}
        placement="bottom-start"
        gutter={6}
        // As the planned-date popover: slide over the input rather than
        // run off the screen when neither side has room.
        overlap
      >
        <Popover.Anchor as="span" class="task-dialog-when-anchor">
          {/* Timer glyph inset in the input's left edge; decorative, the
              input carries the label. */}
          <span
            class="task-dialog-when-icon"
            aria-hidden="true"
            innerHTML={timerSvg}
          />
          <input
            ref={inputRef}
            {...typing.inputProps}
            class="task-dialog-when-input task-dialog-deadline-input"
            value={typing.query() ?? label()}
            placeholder={m().deadline.unset}
            aria-label={m().deadline.label}
            data-tone={tone()}
            data-muted={props.muted() ? "" : undefined}
          />
          {/* Inset ✕ at the input's right edge. mousedown is cancelled so
              the click never moves focus off the input. */}
          <Show when={props.deadline()}>
            <button
              type="button"
              class="icon-button task-dialog-when-clear"
              aria-label={m().deadline.remove}
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
            <Show when={typing.listShown()}>
              <DateSuggestList
                typing={typing}
                format={(day) =>
                  typing.query()?.trim()
                    ? formatDay(day)
                    : (todayOrTomorrow(day) ?? formatDay(day))
                }
                hint={(day) =>
                  typing.query()?.trim() ? todayOrTomorrow(day) : formatDay(day)
                }
              />
            </Show>
            <CalendarPicker
              kind="deadline"
              setOpen={props.setOpen}
              value={props.deadline}
              preview={typing.preview}
              onPick={props.onChange}
              onRemove={() => props.onChange(null)}
            />
          </Popover.Content>
        </Popover.Portal>
      </Popover>
    </div>
  );
}
