// The typed half of a date input that anchors a calendar popover
// (`WhenField.tsx`'s start and end dates). `createDateTyping` holds what
// has been typed, the days it could mean (`dateSuggest.ts`) and the
// cursor among them, and hands back the input's props; `DateSuggestList`
// is the listbox the host shows in the popover in place of its calendar
// while `listShown()`.
//
// A click selects the input's text so typing replaces it, and typing
// opens the popover. Arrow keys move the cursor, Enter or a click takes
// the row and closes. Enter on emptied text calls `onClear` (a host
// without one just closes). Text that names nothing leaves the calendar
// up; Escape, Tab and blur drop the typed text, so the input shows the
// stored day again. Focus stays in the input throughout; ArrowDown steps
// into the calendar while that is what shows.

import { createEffect, createMemo, createSignal, createUniqueId, For, Show } from "solid-js";
import { dateSuggestions } from "./dateSuggest.ts";

export interface DateTypingOptions {
  open: () => boolean;
  setOpen: (v: boolean) => void;
  input: () => HTMLInputElement | undefined;
  /** The popover's content, for stepping into the calendar. */
  content: () => HTMLElement | undefined;
  /** The instant phrases are read against ("friday" is the first one
   *  from here). */
  reference: () => Date;
  locale: () => string;
  includeTime?: boolean;
  /** Days the field can take; the rest are left out of the list. */
  allowed?: (day: string) => boolean;
  /** Write the picked date or datetime; the popover is closed after. */
  onPick: (day: string) => void;
  /** Enter on emptied text. */
  onClear?: () => void;
}

export type DateTyping = ReturnType<typeof createDateTyping>;

export function createDateTyping(opts: DateTypingOptions) {
  const listboxId = `${createUniqueId()}-listbox`;
  const optionId = (i: number) => `${listboxId}-option-${i}`;

  // What has been typed since the input last showed the stored day; null
  // until the first keystroke.
  const [query, setQuery] = createSignal<string | null>(null);
  const [cursor, setCursor] = createSignal(0);
  const suggestions = createMemo(() => {
    const q = query();
    if (q === null) return [];
    const days = dateSuggestions(q, { now: opts.reference(), locale: opts.locale(), includeTime: opts.includeTime });
    const allowed = opts.allowed;
    return allowed ? days.filter(allowed) : days;
  });
  const listShown = () => opts.open() && suggestions().length > 0;

  // However the popover closed (a pick, Escape, a click away), the typed
  // text goes with it.
  createEffect(() => {
    if (!opts.open()) setQuery(null);
  });

  const pick = (day: string) => {
    opts.onPick(day);
    opts.setOpen(false);
  };

  // Select-all is deferred past the pending mouseup when a click brought
  // focus: the browser collapses a selection made on focus as the button
  // comes up (as in `TimePicker`).
  let selectOnMouseUp = false;

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = suggestions().length;
      if (listShown()) {
        setCursor((cursor() + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
      } else if (!opts.open()) {
        opts.setOpen(true);
      } else if (e.key === "ArrowDown") {
        // The calendar is showing: step into it.
        opts.content()?.querySelector<HTMLElement>("button")?.focus();
      }
      return;
    }
    if (e.key !== "Enter") return;
    // Always swallowed, so it never reaches the dialog's title field.
    e.preventDefault();
    const day = listShown() ? suggestions()[cursor()] : undefined;
    if (day) {
      pick(day);
      // Focus never left, so reselect: the next keystroke replaces the
      // day just written rather than trailing it.
      queueMicrotask(() => opts.input()?.select());
    } else if (query() !== null && query()?.trim() === "") {
      opts.onClear?.();
      opts.setOpen(false);
    } else if (!opts.open()) {
      opts.setOpen(true);
    }
  };

  /** Spread onto the input; the host adds `ref`, `value`
   *  (`query() ?? stored label`), class and label. */
  const inputProps = {
    type: "text",
    role: "combobox",
    autocomplete: "off",
    autocorrect: "off",
    autocapitalize: "off",
    spellcheck: false,
    enterkeyhint: "done",
    "aria-autocomplete": "list",
    get "aria-haspopup"() {
      return listShown() ? "listbox" : "dialog";
    },
    get "aria-expanded"() {
      return opts.open();
    },
    get "aria-controls"() {
      return listShown() ? listboxId : undefined;
    },
    get "aria-activedescendant"() {
      return listShown() ? optionId(cursor()) : undefined;
    },
    onFocus: (e: FocusEvent & { currentTarget: HTMLInputElement }) => {
      selectOnMouseUp = true;
      e.currentTarget.select();
    },
    onMouseUp: (e: MouseEvent) => {
      if (!selectOnMouseUp) return;
      selectOnMouseUp = false;
      e.preventDefault();
    },
    onBlur: () => {
      selectOnMouseUp = false;
      setQuery(null);
    },
    onInput: (e: InputEvent & { currentTarget: HTMLInputElement }) => {
      setQuery(e.currentTarget.value);
      setCursor(0);
      opts.setOpen(true);
    },
    onClick: () => opts.setOpen(true),
    onKeyDown,
  } as const;

  return {
    query,
    cursor,
    setCursor,
    suggestions,
    listShown,
    listboxId,
    optionId,
    pick,
    inputProps,
  };
}

/** The suggestions as a listbox, in the time picker's row style. */
export function DateSuggestList(props: {
  typing: DateTyping;
  format: (day: string) => string;
  /** Optional dim note after a row's label ("Today"). */
  hint?: (day: string) => string | null;
}) {
  const t = () => props.typing;
  return (
    // mousedown is cancelled so a row click never blurs the input, which
    // would drop the typed text (and with it this list) before the click
    // could land.
    <div
      id={t().listboxId}
      role="listbox"
      class="date-suggest"
      onMouseDown={(e) => e.preventDefault()}
    >
      <For each={t().suggestions()}>
        {(day, i) => (
          <div
            id={t().optionId(i())}
            role="option"
            aria-selected={i() === t().cursor()}
            class="palette__item"
            classList={{ "palette__item--selected": i() === t().cursor() }}
            onMouseMove={() => {
              if (t().cursor() !== i()) t().setCursor(i());
            }}
            onClick={() => t().pick(day)}
          >
            <span class="palette__item-name">
              {props.format(day)}
              <Show when={props.hint?.(day)}>
                {(hint) => <span class="time-picker-hint">{hint()}</span>}
              </Show>
            </span>
          </div>
        )}
      </For>
    </div>
  );
}
