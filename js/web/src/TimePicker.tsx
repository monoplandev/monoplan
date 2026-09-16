// The task dialog's time control: a plain text input that shows the stored
// time ("1:03 PM" / "13:03" in the app's hour cycle) and, while focused,
// a listbox of suggestions under it — Kobalte's Select look, ListPicker's
// machinery (portaled panel, aria-activedescendant cursor), with the input
// living in the dates band rather than inside the panel.
//
// Focus or a click opens the panel on every quarter hour with the stored
// time under the cursor and the input's text selected, so typing replaces
// it; the typed text narrows the list to what it names (`timeSuggest.ts`
// has the grammar). With no time stored the input shows the host's dim
// placeholder, and a ✕ after the input clears a stored time back to it. Arrow keys move the cursor and put the row's label
// in the input (selected again, still without narrowing the list: the
// typed query and the shown text are separate). Enter commits the row
// under the cursor, clicking a row commits it, and both leave focus in
// the input. Everything else is a revert: Escape, Tab (moves on without
// committing) and any blur put the stored value back. A committed value
// is always a complete hour + minute or null: there is no partial state
// to write.

import {
  createEffect,
  createMemo,
  createSignal,
  createUniqueId,
  For,
  onCleanup,
  Show,
  untrack,
} from "solid-js";
import { Portal } from "solid-js/web";
import { timeFormatter, type TimeParts } from "./format.tsx";
import {
  nearestQuarterIndex,
  timeSuggestions,
  type TimeSuggestion,
} from "./timeSuggest.ts";

/** Gap between the input and the panel, and the minimum breathing room
 *  kept against every viewport edge. */
const GUTTER = 4;
const MARGIN = 8;

export function TimePicker(props: {
  /** The stored time; null is all-day. */
  value: () => Required<TimeParts> | null;
  onChange: (time: Required<TimeParts> | null) => void;
  cycle: () => 12 | 24;
  locale: () => string;
  /** Accessible name. */
  label: string;
  /** Placeholder shown for the null value. */
  placeholder: () => string;
  /** Accessible name of the ✕ that clears the time. */
  clearLabel: string;
  /** Raw SVG for a decorative glyph inset at the input's left edge; the
   *  input pads past it. */
  icon?: string;
  class?: string;
  /** Optional dim note after each option's label; for the end-of-span
   *  picker this is the length that option would give. */
  optionHint?: (t: TimeSuggestion) => string | null;
  /** Anchor for an end-of-span picker: the blank list runs from this
   *  time + 15 minutes round the clock, and typed readings order by
   *  distance after it (`timeSuggest.ts`). */
  after?: () => Required<TimeParts> | null;
}) {
  const baseId = createUniqueId();
  const listboxId = `${baseId}-listbox`;
  const optionId = (i: number) => `${baseId}-option-${i}`;

  const [open, setOpen] = createSignal(false);
  // What has been typed since opening; null until the first keystroke,
  // when the input still shows the stored value. Drives the list.
  const [query, setQuery] = createSignal<string | null>(null);
  // The row an arrow key put in the input, shown in place of the query
  // until the next keystroke.
  const [preview, setPreview] = createSignal<TimeSuggestion | null>(null);
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [pos, setPos] = createSignal({ left: 0, top: 0 });

  let inputRef: HTMLInputElement | undefined;
  let panelRef: HTMLDivElement | undefined;
  let listRef: HTMLDivElement | undefined;

  // Same hover guard as ListPicker: keyboard scrolling slides rows under
  // a stationary mouse and must not snap the cursor back to it.
  let lastMouse: { x: number; y: number } | null = null;
  function onRowMouseMove(e: MouseEvent, index: number) {
    if (lastMouse && lastMouse.x === e.screenX && lastMouse.y === e.screenY) {
      return;
    }
    lastMouse = { x: e.screenX, y: e.screenY };
    if (selectedIndex() !== index) setSelectedIndex(index);
  }

  // Rows and the input read "1 PM" on the hour rather than "1:00 PM":
  // the minutes carry nothing there. A 24-hour clock keeps "13:00", where
  // a bare "13" does not read as a time.
  const format = (t: TimeSuggestion) => {
    const date = new Date(2000, 0, 1, t.hour, t.minute);
    if (t.minute === 0 && props.cycle() === 12) {
      return new Intl.DateTimeFormat(props.locale(), {
        hour: "numeric",
        hourCycle: "h12",
      }).format(date);
    }
    return timeFormatter(props.locale()).format(date);
  };

  const display = createMemo(() => {
    const p = preview();
    if (p) return format(p);
    const q = query();
    if (q !== null) return q;
    const v = props.value();
    return v ? format(v) : "";
  });

  const blank = () => (query() ?? "").trim() === "";
  const after = () => props.after?.() ?? null;
  const items = createMemo(() =>
    timeSuggestions(query() ?? "", props.cycle(), after()),
  );
  /** Row of the stored time (9:00 without one; an hour on in an
   *  anchored list) in the blank list. */
  const storedIndex = () => nearestQuarterIndex(props.value(), after());
  const panelVisible = () => open() && items().length > 0;

  const scrollSelectedIntoView = (
    index: number,
    block: ScrollLogicalPosition = "nearest",
  ) => {
    listRef?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block });
  };

  // Under the input, or above it when the viewport runs out. Portaled to
  // <body> because the task dialog is a scroll container that would clip
  // it, which is also why scroll / resize reposition below.
  const place = () => {
    const panel = panelRef;
    const anchor = inputRef?.getBoundingClientRect();
    if (!panel || !anchor) return;
    const below = anchor.bottom + GUTTER;
    const fitsBelow = below + panel.offsetHeight <= window.innerHeight - MARGIN;
    const top = fitsBelow
      ? below
      : Math.max(MARGIN, anchor.top - GUTTER - panel.offsetHeight);
    const left = Math.min(
      anchor.left,
      window.innerWidth - panel.offsetWidth - MARGIN,
    );
    setPos({ top, left: Math.max(MARGIN, left) });
  };

  // Select-all is deferred past the pending mouseup when a click brought
  // focus: the browser collapses a selection made on mousedown / focus
  // as the button comes up.
  let selectOnMouseUp = false;
  const selectAll = () => inputRef?.select();

  const openBlank = () => {
    if (open()) return;
    setQuery(null);
    setPreview(null);
    setSelectedIndex(storedIndex());
    const anchor = inputRef?.getBoundingClientRect();
    if (anchor) setPos({ left: anchor.left, top: anchor.bottom + GUTTER });
    setOpen(true);
  };

  const revert = () => {
    setOpen(false);
    setQuery(null);
    setPreview(null);
  };

  const commit = (t: TimeSuggestion | null) => {
    props.onChange(t);
    setOpen(false);
    setQuery(null);
    setPreview(null);
    inputRef?.focus();
  };

  // Correct the seeded position once the panel has a height, and bring
  // the cursor row into view: centred for the long blank list, nearest
  // for a typed narrowing.
  createEffect(() => {
    if (!panelVisible()) return;
    items(); // a narrowing changes the height, which matters once flipped
    untrack(() => {
      const centre = blank();
      const index = selectedIndex();
      requestAnimationFrame(() => {
        place();
        scrollSelectedIntoView(index, centre ? "center" : "nearest");
      });
    });
  });

  // While open: Escape reverts, at capture depth and stopped dead so it
  // never reaches the task dialog behind (which would close the whole
  // thing). Scroll / resize keep the panel under the input.
  createEffect(() => {
    if (!open()) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      revert();
    };
    const reposition = () => place();
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", reposition);
    document.addEventListener("scroll", reposition, true);
    onCleanup(() => {
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", reposition);
      document.removeEventListener("scroll", reposition, true);
    });
  });

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") return; // the document listener above
    if (e.key === "Tab") return; // leaves without committing; blur reverts
    if (!open()) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        openBlank();
      } else if (e.key === "Enter") {
        // Swallowed so it never reaches the dialog's title field.
        e.preventDefault();
      }
      return;
    }
    const list = items();
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!list.length) return;
      const next =
        e.key === "ArrowDown"
          ? (selectedIndex() + 1) % list.length
          : (selectedIndex() - 1 + list.length) % list.length;
      setSelectedIndex(next);
      scrollSelectedIntoView(next);
      setPreview(list[next] ?? null);
      // After the value write lands so the whole label is selected.
      queueMicrotask(selectAll);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const t = list[selectedIndex()];
      if (t) commit(t);
    }
  };

  return (
    <>
      <span class="time-picker-field">
        <Show when={props.icon}>
          {(svg) => (
            <span
              class="time-picker-icon"
              aria-hidden="true"
              innerHTML={svg()}
            />
          )}
        </Show>
        <input
          ref={inputRef}
          type="text"
          role="combobox"
          class={props.class}
          autocomplete="off"
          autocorrect="off"
          spellcheck={false}
          // Digits are what a time needs; the grammar accepts "130" for
          // 1:30 because this keypad has no colon.
          inputMode="numeric"
          enterkeyhint="done"
          value={display()}
          placeholder={props.placeholder()}
          aria-label={props.label}
          aria-expanded={panelVisible()}
          aria-controls={panelVisible() ? listboxId : undefined}
          aria-autocomplete="list"
          aria-activedescendant={
            panelVisible() ? optionId(selectedIndex()) : undefined
          }
          onFocus={() => {
            openBlank();
            selectOnMouseUp = true;
            selectAll();
          }}
          onMouseUp={(e) => {
            if (!selectOnMouseUp) return;
            selectOnMouseUp = false;
            e.preventDefault();
          }}
          onClick={() => {
            // A click on the already-focused input (after an Enter commit
            // closed the panel) reopens it the same way focus does.
            if (open()) return;
            openBlank();
            selectAll();
          }}
          onBlur={() => {
            selectOnMouseUp = false;
            revert();
          }}
          onInput={(e) => {
            const v = e.currentTarget.value;
            setPreview(null);
            setQuery(v);
            // Best match under the cursor; the stored value again once the
            // text is cleared back to the full list.
            setSelectedIndex(v.trim() === "" ? storedIndex() : 0);
            setOpen(true);
          }}
          onKeyDown={onKeyDown}
        />
        {/* mousedown is cancelled so the click never blurs the input (which
            would revert the panel under it) and focus stays put. */}
        <Show when={props.value()}>
          <button
            type="button"
            class="icon-button time-picker-clear"
            aria-label={props.clearLabel}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => commit(null)}
          >
            ✕
          </button>
        </Show>
      </span>
      <Show when={panelVisible()}>
        <Portal>
          {/* data-kb-top-layer exempts the panel from the task dialog's
              focus trap and aria-hide sweep. mousedown is cancelled so a
              row click (or a scrollbar drag) never blurs the input, which
              would revert before the click could commit. */}
          <div
            ref={panelRef}
            class="list-picker time-picker palette"
            data-kb-top-layer
            style={{ left: `${pos().left}px`, top: `${pos().top}px` }}
            onMouseDown={(e) => e.preventDefault()}
          >
            <div
              ref={listRef}
              id={listboxId}
              role="listbox"
              class="palette__results"
            >
              <For each={items()}>
                {(t, i) => (
                  <div
                    id={optionId(i())}
                    data-index={i()}
                    role="option"
                    aria-selected={i() === selectedIndex()}
                    class="palette__item"
                    classList={{
                      "palette__item--selected": i() === selectedIndex(),
                    }}
                    onMouseMove={(e) => onRowMouseMove(e, i())}
                    onClick={() => commit(t)}
                  >
                    <span class="palette__item-name">
                      {format(t)}
                      <Show when={props.optionHint?.(t)}>
                        {(hint) => (
                          <span class="time-picker-hint">{hint()}</span>
                        )}
                      </Show>
                    </span>
                  </div>
                )}
              </For>
            </div>
          </div>
        </Portal>
      </Show>
    </>
  );
}
