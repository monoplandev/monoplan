// The task dialog's time-zone control (`spec/calendar-plan.md` "Fixed
// instants"): a row under the time row, there only while the `when` has
// a time. Blank ("Time zone" as its dim placeholder) the time is
// floating, wall time wherever the device is. Picking a zone fixes the
// time to it: the digits in the date and time fields above stay as they
// are and now read on that zone's clock, which is what those fields show
// and edit from then on. The closed field reads the zone's offset on the
// item's day and its city ("GMT+1 Paris").
//
// `TimePicker`'s manner throughout: a text input that opens a listbox of
// zones on focus, narrowed by what is typed (`zones.ts` has the list and
// the matching), arrow keys to move, Enter or a click to commit, and
// Escape, Tab and blur to put the stored value back. Each row is the
// city with the zone's name and its offset on the item's day dim after
// it. The inset ✕ and the Remove row pinned under the list both make the
// time floating again.

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
import { whenDay } from "./format.tsx";
import globeSvg from "./icons/globe.svg?raw";
import {
  deviceZone,
  zoneOption,
  zoneOptions,
  zoneSuggestions,
  type ZoneOption,
} from "./zones.ts";

const GUTTER = 4;
const MARGIN = 8;

export function ZonePicker(props: {
  /** The zone the time is fixed to; null is floating. */
  value: () => string | null;
  /** The timed `when` on the zone's clock, as the fields above show it
   *  (the device's when floating). */
  when: () => string;
  onChange: (zone: string | null) => void;
  locale: () => string;
  /** Accessible name and placeholder. */
  label: () => string;
  removeLabel: () => string;
}) {
  const baseId = createUniqueId();
  const listboxId = `${baseId}-listbox`;
  const optionId = (i: number) => `${baseId}-option-${i}`;

  const [open, setOpen] = createSignal(false);
  // Typed since opening; null until the first keystroke.
  const [query, setQuery] = createSignal<string | null>(null);
  // The row an arrow key put in the input.
  const [preview, setPreview] = createSignal<ZoneOption | null>(null);
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [pos, setPos] = createSignal({ left: 0, top: 0, width: 0 });

  let inputRef: HTMLInputElement | undefined;
  let panelRef: HTMLDivElement | undefined;
  let listRef: HTMLDivElement | undefined;

  let lastMouse: { x: number; y: number } | null = null;
  function onRowMouseMove(e: MouseEvent, index: number) {
    if (lastMouse && lastMouse.x === e.screenX && lastMouse.y === e.screenY) {
      return;
    }
    lastMouse = { x: e.screenX, y: e.screenY };
    if (selectedIndex() !== index) setSelectedIndex(index);
  }

  const day = () => whenDay(props.when());

  // The stored zone as the closed field reads: its offset on the
  // item's day, then its city ("GMT+1 Paris").
  const storedOption = createMemo(() => {
    const zone = props.value();
    return zone ? zoneOption(zone, props.locale(), day()) : null;
  });
  const stored = () => {
    const o = storedOption();
    return o ? [o.offset, o.city].filter(Boolean).join(" ") : "";
  };
  // Hover text: the id and the zone's name, which the field leaves out.
  const title = () => {
    const o = storedOption();
    return o ? [o.id, o.name].filter(Boolean).join(" · ") : undefined;
  };

  const display = () => {
    const p = preview();
    if (p) return p.city;
    return query() ?? stored();
  };

  const blank = () => (query() ?? "").trim() === "";
  // Built only once the panel opens: the full list costs some hundreds
  // of `Intl` formatters the first time.
  const items = createMemo(() =>
    open()
      ? zoneSuggestions(query() ?? "", zoneOptions(props.locale(), day()), deviceZone())
      : [],
  );
  /** Row of the stored zone in the blank list; the top (the device's
   *  zone) without one. */
  const storedIndex = () => {
    const zone = props.value();
    const i = zone ? items().findIndex((o) => o.id === zone) : -1;
    return Math.max(0, i);
  };
  const panelVisible = () => open() && items().length > 0;

  const scrollSelectedIntoView = (
    index: number,
    block: ScrollLogicalPosition = "nearest",
  ) => {
    listRef?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block });
  };

  // As wide as the field, under it, or above when the viewport runs out
  // (`TimePicker.place`).
  const place = () => {
    const panel = panelRef;
    const anchor = inputRef?.getBoundingClientRect();
    if (!panel || !anchor) return;
    const vv = window.visualViewport;
    const viewTop = vv?.offsetTop ?? 0;
    const viewBottom = vv ? vv.offsetTop + vv.height : window.innerHeight;
    const below = anchor.bottom + GUTTER;
    const roomBelow = viewBottom - MARGIN - below;
    const roomAbove = anchor.top - GUTTER - viewTop - MARGIN;
    const scrolled = listRef?.scrollTop ?? 0;
    panel.style.maxHeight = "";
    const natural = panel.offsetHeight;
    const goBelow = natural <= roomBelow || roomBelow >= roomAbove;
    const height = Math.min(natural, goBelow ? roomBelow : roomAbove);
    if (height < natural) panel.style.maxHeight = `${Math.max(0, height)}px`;
    if (listRef) listRef.scrollTop = scrolled;
    const top = goBelow ? below : anchor.top - GUTTER - height;
    const left = Math.min(
      anchor.left,
      window.innerWidth - panel.offsetWidth - MARGIN,
    );
    setPos({ top, left: Math.max(MARGIN, left), width: anchor.width });
  };

  let selectOnMouseUp = false;
  const selectAll = () => inputRef?.select();

  const openBlank = () => {
    if (open()) return;
    setQuery(null);
    setPreview(null);
    const anchor = inputRef?.getBoundingClientRect();
    if (anchor) {
      setPos({ left: anchor.left, top: anchor.bottom + GUTTER, width: anchor.width });
    }
    setOpen(true);
    setSelectedIndex(storedIndex());
  };

  const revert = () => {
    setOpen(false);
    setQuery(null);
    setPreview(null);
  };

  const commit = (zone: string | null) => {
    props.onChange(zone);
    revert();
    inputRef?.focus();
  };

  createEffect(() => {
    if (!panelVisible()) return;
    items();
    untrack(() => {
      const centre = blank();
      const index = selectedIndex();
      requestAnimationFrame(() => {
        place();
        scrollSelectedIntoView(index, centre ? "center" : "nearest");
      });
    });
  });

  // While open: Escape reverts at capture depth, so it never reaches the
  // task dialog behind; scroll / resize keep the panel under the input.
  createEffect(() => {
    if (!open()) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      revert();
    };
    const reposition = (e: Event) => {
      if (e.target instanceof Node && panelRef?.contains(e.target)) return;
      place();
    };
    const vv = window.visualViewport;
    const onViewportResize = () => {
      place();
      scrollSelectedIntoView(selectedIndex(), blank() ? "center" : "nearest");
    };
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", reposition);
    document.addEventListener("scroll", reposition, true);
    vv?.addEventListener("resize", onViewportResize);
    vv?.addEventListener("scroll", reposition);
    onCleanup(() => {
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", reposition);
      document.removeEventListener("scroll", reposition, true);
      vv?.removeEventListener("resize", onViewportResize);
      vv?.removeEventListener("scroll", reposition);
    });
  });

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" || e.key === "Tab") return;
    if (!open()) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        openBlank();
      } else if (e.key === "Enter") {
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
      queueMicrotask(selectAll);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      // Emptied and entered: no zone, as for the date inputs.
      if (query() !== null && blank()) {
        commit(null);
        return;
      }
      const o = list[selectedIndex()];
      if (o) commit(o.id);
    }
  };

  return (
    <>
      <span class="time-picker-field zone-picker-field">
        <span class="time-picker-icon" aria-hidden="true" innerHTML={globeSvg} />
        <input
          ref={inputRef}
          type="text"
          role="combobox"
          class="time-picker-input"
          autocomplete="off"
          autocorrect="off"
          spellcheck={false}
          enterkeyhint="done"
          value={display()}
          title={title()}
          placeholder={props.label()}
          aria-label={props.label()}
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
            setOpen(true);
            setSelectedIndex(v.trim() === "" ? storedIndex() : 0);
          }}
          onKeyDown={onKeyDown}
        />
        <Show when={props.value()}>
          <button
            type="button"
            class="icon-button time-picker-clear"
            aria-label={props.removeLabel()}
            title={props.removeLabel()}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => props.onChange(null)}
          >
            ✕
          </button>
        </Show>
      </span>
      <Show when={panelVisible()}>
        <Portal>
          {/* data-kb-top-layer exempts the panel from the task dialog's
              focus trap; mousedown is cancelled so a row click never
              blurs the input (which would revert before the commit). */}
          <div
            ref={panelRef}
            class="list-picker time-picker zone-picker palette"
            data-kb-top-layer
            style={{
              left: `${pos().left}px`,
              top: `${pos().top}px`,
              "min-width": `${pos().width}px`,
            }}
            onMouseDown={(e) => e.preventDefault()}
          >
            <div
              ref={listRef}
              id={listboxId}
              role="listbox"
              class="palette__results"
            >
              <For each={items()}>
                {(o, i) => (
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
                    onClick={() => commit(o.id)}
                  >
                    <span class="palette__item-name">
                      {o.city}
                      <span class="time-picker-hint">
                        {[o.name, o.offset].filter(Boolean).join(" · ")}
                      </span>
                    </span>
                  </div>
                )}
              </For>
            </div>
            <Show when={props.value()}>
              <div class="time-picker-footer">
                <button
                  type="button"
                  class="time-picker-remove"
                  tabIndex={-1}
                  onClick={() => commit(null)}
                >
                  {props.removeLabel()}
                </button>
              </div>
            </Show>
          </div>
        </Portal>
      </Show>
    </>
  );
}
