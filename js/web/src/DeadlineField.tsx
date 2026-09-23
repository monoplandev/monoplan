// The task dialog's deadline control: a text input in its own ruled band
// under the dates band, with the same anatomy as the planned-date input
// (`WhenField`): a timer glyph inset at the left edge, the stored day read
// as "Wed 23 Sept" ("Deadline" as its placeholder when unset), an inset ✕
// that removes it, and a popover under it with Today / Tomorrow quick
// actions over the shared calendar picker (`CalendarPicker`, kind
// "deadline"; Remove). The input is read-only for now, so the popover is
// the only writer. Unlike the badge on rows and cards it shows the date
// rather than "Overdue" / "Today": a field says what is stored; the
// urgency shows as the text colour instead (`data-tone`), dropped while
// the item is done or binned (`muted`).

import { Popover } from "@kobalte/core/popover";
import { createMemo, Show } from "solid-js";
import { CalendarPicker } from "./DeadlineCalendarDialog.tsx";
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

  const pickDay = (day: string) => {
    props.onChange(day);
    props.setOpen(false);
  };

  // The day, with the year once it isn't this one; judged against the
  // shared `nowMs()` tick so it rolls over without a reload.
  const label = createMemo(() => {
    const d = props.deadline();
    if (!d) return "";
    const date = parseLocalDateParts(d);
    if (!date) return d;
    const thisYear =
      date.getFullYear() ===
      parseLocalDateParts(todayStamp(nowMs()))?.getFullYear();
    return new Intl.DateTimeFormat(locale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
      ...(thisYear ? {} : { year: "numeric" }),
    }).format(date);
  });

  // Urgency for the text colour, from the same reading the row badge
  // uses; muted items carry no urgency.
  const tone = createMemo(() => {
    if (props.muted()) return "muted";
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
            type="text"
            class="task-dialog-when-input task-dialog-deadline-input"
            readOnly
            inputMode="none"
            value={label()}
            placeholder={m().deadline.unset}
            aria-label={m().deadline.label}
            aria-haspopup="dialog"
            aria-expanded={props.open()}
            data-tone={tone()}
            data-muted={props.muted() ? "" : undefined}
            onClick={() => props.setOpen(true)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
                e.preventDefault();
                props.setOpen(true);
              }
            }}
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
                {m().deadline.today}
              </button>
              <button
                type="button"
                class="when-popover-quick-action"
                onClick={() => pickDay(addDaysToStamp(todayStamp(nowMs()), 1))}
              >
                {m().deadline.tomorrow}
              </button>
            </div>
            <CalendarPicker
              kind="deadline"
              open={props.open}
              setOpen={props.setOpen}
              value={props.deadline}
              onPick={props.onChange}
              onRemove={() => props.onChange(null)}
            />
          </Popover.Content>
        </Popover.Portal>
      </Popover>
    </div>
  );
}
