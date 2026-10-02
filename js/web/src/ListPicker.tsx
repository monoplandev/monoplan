// The task dialog's move-to-list control: FindPalette's shape at popover
// scale. Kobalte's Combobox makes the trigger *be* the filter input, which
// loses the plain "this item lives in X" reading of the header, so this is
// hand-rolled — a text trigger that opens a filter box above a keyboard-
// driven listbox, sharing the palette's row anatomy and CSS.
//
// It differs from FindPalette in three deliberate ways, all of them policy
// rather than machinery: an empty query lists every option (the palette
// shows nothing until you type), opening highlights the current list rather
// than the first row, and there is no debounce — the option set is a
// handful of names filtered in memory, not an index query.
//
// Deliberately selection-only — lists are created from the nav, never
// from here.

import {
  createEffect,
  createMemo,
  createSignal,
  createUniqueId,
  For,
  on,
  onCleanup,
  Show,
} from "solid-js";
import { Portal } from "solid-js/web";
import archiveSvg from "./icons/archive.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import caretSortSvg from "./icons/caret-sort.svg?raw";
import fileSvg from "./icons/file.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { matchesName } from "./search.ts";
import { LIST_EVENTS } from "./sync/store.ts";

/** One entry in the move-to-list picker: the reserved `inbox` and
 *  `events` lists plus every user list. `icon` is a list's chosen emoji
 *  grapheme when it has one; the picker falls back to the same glyphs the
 *  nav uses. */
export type ListOption = { id: string; name: string; icon?: string };

/** Fallback glyph for a list with no chosen emoji: the nav's own marks
 *  for the reserved lists (Inbox's archive box, the calendar for Events),
 *  the default file glyph otherwise. */
export const listGlyph = (id: string): string =>
  id === "inbox" ? archiveSvg : id === LIST_EVENTS ? calendarSvg : fileSvg;

/** Gap between the trigger and the panel, and the minimum breathing room
 *  kept against every viewport edge. */
const GUTTER = 4;
const MARGIN = 8;

export function ListPicker(props: {
  options: () => ListOption[];
  /** Currently filed-under list id, or null while a new capture has none. */
  value: () => string | null;
  onChange: (id: string) => void;
}) {
  const { m } = useAppI18n();
  const baseId = createUniqueId();
  const listboxId = `${baseId}-listbox`;
  const optionId = (i: number) => `${baseId}-option-${i}`;

  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [selectedIndex, setSelectedIndex] = createSignal(0);

  // Hover-to-select only on real pointer motion: keyboard scrolling slides
  // rows under a stationary mouse and fires mouseenter / synthetic
  // mousemove, which would snap the selection back to the cursor. Same
  // guard as FindPalette.
  let lastMouse: { x: number; y: number } | null = null;
  function onRowMouseMove(e: MouseEvent, index: number) {
    if (lastMouse && lastMouse.x === e.screenX && lastMouse.y === e.screenY) {
      return;
    }
    lastMouse = { x: e.screenX, y: e.screenY };
    if (selectedIndex() !== index) setSelectedIndex(index);
  }
  // Viewport coordinates for the portaled panel, seeded from the trigger
  // at open (so it never paints at 0,0) and refined once its height is
  // measurable.
  const [pos, setPos] = createSignal({ left: 0, top: 0 });

  const selected = createMemo<ListOption | null>(
    () => props.options().find((o) => o.id === props.value()) ?? null,
  );
  const items = createMemo(() =>
    props.options().filter((o) => matchesName(o.name, query())),
  );

  let triggerRef: HTMLButtonElement | undefined;
  let panelRef: HTMLDivElement | undefined;
  let inputRef: HTMLInputElement | undefined;
  let listRef: HTMLDivElement | undefined;

  const scrollSelectedIntoView = (index: number) => {
    const el = listRef?.querySelector(`[data-index="${index}"]`);
    el?.scrollIntoView({ block: "nearest" });
  };

  // Anchor the panel under the trigger. Portaled to <body> rather than
  // positioned inside the header, because .task-dialog is a scroll
  // container and would clip it — which also means the trigger can move
  // under us, hence the scroll / resize repositioning below.
  const place = () => {
    const panel = panelRef;
    const anchor = triggerRef?.getBoundingClientRect();
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

  const close = (restoreFocus = true) => {
    setOpen(false);
    if (restoreFocus) triggerRef?.focus();
  };

  const commit = (opt: ListOption) => {
    props.onChange(opt.id);
    close();
  };

  const toggle = () => {
    if (open()) {
      close();
      return;
    }
    // Every open starts from a clean filter showing all lists, with the
    // item's current list under the cursor — so Enter straight after
    // opening is a no-op rather than a surprise move.
    setQuery("");
    const at = props.options().findIndex((o) => o.id === props.value());
    setSelectedIndex(at === -1 ? 0 : at);
    const anchor = triggerRef?.getBoundingClientRect();
    if (anchor) setPos({ left: anchor.left, top: anchor.bottom + GUTTER });
    setOpen(true);
  };

  // Land focus in the filter box on open, and correct the seeded position
  // now that the panel has a measurable height. rAF so both run against
  // the committed DOM.
  createEffect(() => {
    if (!open()) return;
    requestAnimationFrame(() => {
      place();
      inputRef?.focus();
      scrollSelectedIntoView(selectedIndex());
    });
  });
  // Filtering changes the panel's height, which matters when it had to
  // flip above the trigger.
  createEffect(on(items, () => open() && place(), { defer: true }));

  // While open: Escape and outside-click dismissal. Escape is taken at
  // capture depth and stopped dead — otherwise it reaches the task
  // dialog behind us, which would close the whole thing rather than just
  // the picker.
  createEffect(() => {
    if (!open()) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      close();
    };
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (panelRef?.contains(target) || triggerRef?.contains(target)) return;
      // Focus goes wherever the click lands, so don't yank it back.
      close(false);
    };
    // Capture phase so scrolling the task dialog itself counts, not just
    // the window — the panel is portaled out of that scroll container.
    const reposition = () => place();

    document.addEventListener("keydown", onKeyDown, true);
    // Defer the outside-click listener a tick so the click that opened
    // the panel doesn't immediately close it.
    const timer = window.setTimeout(() => {
      document.addEventListener("mousedown", onPointerDown);
    }, 0);
    window.addEventListener("resize", reposition);
    document.addEventListener("scroll", reposition, true);

    onCleanup(() => {
      document.removeEventListener("keydown", onKeyDown, true);
      window.clearTimeout(timer);
      document.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("resize", reposition);
      document.removeEventListener("scroll", reposition, true);
    });
  });

  const onInputKeyDown = (e: KeyboardEvent) => {
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
      return;
    }
    if (e.key === "Enter") {
      // Swallowed either way: Enter must not reach the task dialog's
      // title field behind the panel.
      e.preventDefault();
      const opt = list[selectedIndex()];
      if (opt) commit(opt);
    }
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        class="badge task-dialog-list"
        aria-label={m().workspace.moveToList}
        aria-haspopup="listbox"
        aria-expanded={open()}
        aria-controls={open() ? listboxId : undefined}
        data-expanded={open() ? "" : undefined}
        onClick={toggle}
      >
        {/* Same glyph rules as the option rows below, so the trigger reads
            as the selected row lifted out of the panel. Nothing renders
            while a new capture has no list yet. */}
        <Show when={selected()}>
          {(opt) => (
            <Show
              when={opt().icon}
              fallback={
                <span
                  class="task-dialog-list-icon"
                  aria-hidden="true"
                  innerHTML={listGlyph(opt().id)}
                />
              }
            >
              {(icon) => (
                <span
                  class="task-dialog-list-icon task-dialog-list-icon-emoji"
                  aria-hidden="true"
                >
                  {icon()}
                </span>
              )}
            </Show>
          )}
        </Show>
        <span class="task-dialog-list-value">{selected()?.name}</span>
        <span
          class="task-dialog-list-caret"
          aria-hidden="true"
          innerHTML={caretSortSvg}
        />
      </button>
      <Show when={open()}>
        <Portal>
          {/* data-kb-top-layer exempts the panel from the task dialog's
              focus trap and its aria-hide-outside sweep — without it
              Kobalte pulls focus straight back out of the filter box. */}
          <div
            ref={panelRef}
            class="list-picker palette"
            data-kb-top-layer
            style={{ left: `${pos().left}px`, top: `${pos().top}px` }}
          >
            <div class="palette__search">
              <input
                ref={inputRef}
                type="text"
                role="combobox"
                autocomplete="off"
                aria-expanded="true"
                aria-controls={listboxId}
                aria-autocomplete="list"
                aria-activedescendant={
                  items().length ? optionId(selectedIndex()) : undefined
                }
                placeholder={m().workspace.searchLists}
                aria-label={m().workspace.searchLists}
                value={query()}
                onInput={(e) => {
                  setQuery(e.currentTarget.value);
                  // Filtering invalidates the old cursor position; the
                  // best match is now the top row.
                  setSelectedIndex(0);
                  scrollSelectedIntoView(0);
                }}
                onKeyDown={onInputKeyDown}
              />
            </div>
            <div
              ref={listRef}
              id={listboxId}
              role="listbox"
              class="palette__results"
            >
              <For each={items()}>
                {(opt, i) => (
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
                    onClick={() => commit(opt)}
                  >
                    {/* Same glyphs the nav uses: the archive mark for the
                        reserved Home list, otherwise a list's chosen emoji
                        or the default file glyph. */}
                    <Show
                      when={opt.icon}
                      fallback={
                        <span
                          class="palette__item-icon"
                          aria-hidden="true"
                          innerHTML={listGlyph(opt.id)}
                        />
                      }
                    >
                      {(icon) => (
                        <span
                          class="palette__item-icon palette__item-icon-emoji"
                          aria-hidden="true"
                        >
                          {icon()}
                        </span>
                      )}
                    </Show>
                    <span class="palette__item-name">{opt.name}</span>
                    <Show when={opt.id === props.value()}>
                      <span class="palette__item-list">
                        {m().workspace.currentList}
                      </span>
                    </Show>
                  </div>
                )}
              </For>
              <Show when={items().length === 0}>
                <div class="palette__empty">
                  {m().workspace.noMatchingLists}
                </div>
              </Show>
            </div>
          </div>
        </Portal>
      </Show>
    </>
  );
}
