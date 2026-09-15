// The task dialog's planned-date control: a text input in the dates band
// that reads the set `when` (same short label as `WhenBadge`, "Date" as
// its placeholder when unset) and opens a popover holding the shared
// calendar picker (`CalendarPicker`, with the optional time field and
// Remove). The input is read-only for now: typed dates are a later step,
// so the popover is the only writer. Anchored on the input rather than a
// Popover.Trigger button so it can become editable without changing shape.

import { Popover } from "@kobalte/core/popover";
import { createMemo } from "solid-js";
import { CalendarPicker } from "./DeadlineCalendarDialog.tsx";
import { formatWhenBadge, nowMs, todayStamp } from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";

export function WhenField(props: {
  when: () => string | null;
  muted: () => boolean;
  onChange: (value: string | null) => void;
  open: () => boolean;
  setOpen: (v: boolean) => void;
}) {
  const { m, locale } = useAppI18n();
  let inputRef: HTMLInputElement | undefined;

  const label = createMemo(() => {
    const w = props.when();
    if (!w) return "";
    return (
      formatWhenBadge(
        w,
        todayStamp(nowMs()),
        { today: m().when.today, tomorrow: m().when.tomorrow },
        locale(),
      )?.label ?? w
    );
  });

  return (
    <Popover
      open={props.open()}
      onOpenChange={props.setOpen}
      placement="bottom-start"
      gutter={6}
    >
      <Popover.Anchor as="span" class="task-dialog-when-anchor">
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
            if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
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
          <CalendarPicker
            kind="when"
            open={props.open}
            setOpen={props.setOpen}
            value={props.when}
            onPick={props.onChange}
            onRemove={() => props.onChange(null)}
          />
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
}
