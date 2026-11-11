// `m` with a selection (or a row context menu's Move to list) opens a
// centered palette to re-file the acted-on rows into another list, of
// their own workspace or another one (a shared workspace's lists are
// tagged with its name; Personal's go untagged).
// FindPalette's shell with ListPicker's policy: an empty query lists
// every option, opening highlights the current list, and there is no
// debounce — the option set is a handful of names filtered in memory,
// not an index query.

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  Show,
} from "solid-js";
import { Portal } from "solid-js/web";
import { useAppI18n } from "./i18n.tsx";
import {
  CurrentTick,
  listGlyph,
  WorkspaceTag,
  type ListOption,
} from "./ListPicker.tsx";
import { trackOverlay } from "./overlay.ts";
import { PaletteFooter } from "./PaletteFooter.tsx";
import { matchesName } from "./search.ts";

export function MovePalette(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  options: () => ListOption[];
  /** Whether an option is the list the acted-on rows already live in
   *  (none when they span lists). Ticked, and highlighted on open so
   *  Enter straight after opening is a no-op rather than a surprise
   *  move. */
  isCurrent: (opt: ListOption) => boolean;
  /** Number of rows the pick acts on — drives the singular / plural
   *  placeholder. */
  count: () => number;
  onPick: (opt: ListOption) => void;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
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
  let inputRef: HTMLInputElement | undefined;
  let listRef: HTMLDivElement | undefined;

  const items = createMemo(() =>
    props.options().filter(
      (o) =>
        matchesName(o.name, query()) ||
        (o.workspace !== undefined && matchesName(o.workspace, query())),
    ),
  );

  function scrollSelectedIntoView(index: number) {
    const el = listRef?.querySelector(`[data-index="${index}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }

  // Reset on open: clean filter showing all lists, cursor on the current
  // list when the host supplied one.
  createEffect(() => {
    if (!props.open) return;
    setQuery("");
    const at = props.options().findIndex(props.isCurrent);
    const index = at === -1 ? 0 : at;
    setSelectedIndex(index);
    requestAnimationFrame(() => {
      inputRef?.focus();
      scrollSelectedIntoView(index);
    });
  });

  function commit(opt: ListOption) {
    props.onPick(opt);
    props.onOpenChange(false);
  }

  // While open: arrows / Enter / Escape at capture depth (stopped dead so
  // they don't leak to the workspace behind), plus click-outside to
  // dismiss.
  createEffect(() => {
    if (!props.open) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
        props.onOpenChange(false);
        return;
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter") {
        e.preventDefault();
        e.stopImmediatePropagation();
        const list = items();
        if (!list.length) return;
        if (e.key === "ArrowDown") {
          const next = (selectedIndex() + 1) % list.length;
          setSelectedIndex(next);
          scrollSelectedIntoView(next);
        } else if (e.key === "ArrowUp") {
          const prev = (selectedIndex() - 1 + list.length) % list.length;
          setSelectedIndex(prev);
          scrollSelectedIntoView(prev);
        } else {
          const opt = list[selectedIndex()];
          if (opt) commit(opt);
        }
      }
    };
    const onClick = (e: MouseEvent) => {
      const palette = document.getElementById("move-palette");
      if (palette && !palette.contains(e.target as Node)) {
        props.onOpenChange(false);
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    // Defer the click listener a tick so the click that triggered the
    // open (e.g. via the row context menu) doesn't immediately close it.
    const timer = window.setTimeout(() => {
      document.addEventListener("mousedown", onClick);
    }, 0);

    onCleanup(() => {
      document.removeEventListener("keydown", onKeyDown, true);
      window.clearTimeout(timer);
      document.removeEventListener("mousedown", onClick);
    });
  });

  return (
    <Show when={props.open}>
      <Portal>
        <div class="palette-overlay" />
        <div
          id="move-palette"
          class="palette"
          role="combobox"
          aria-expanded="true"
          aria-haspopup="listbox"
          aria-owns="move-palette-listbox"
        >
          <div class="palette__search">
            <input
              ref={(el) => {
                inputRef = el;
              }}
              type="text"
              placeholder={
                props.count() > 1
                  ? m().workspace.moveItems
                  : m().workspace.moveItem
              }
              aria-label={m().workspace.moveToList}
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
                // Filtering invalidates the old cursor position; the
                // best match is now the top row.
                setSelectedIndex(0);
                scrollSelectedIntoView(0);
              }}
              aria-autocomplete="list"
              aria-controls="move-palette-listbox"
              aria-activedescendant={
                items().length > 0
                  ? `move-palette-item-${selectedIndex()}`
                  : undefined
              }
            />
          </div>
          <div
            ref={listRef}
            id="move-palette-listbox"
            role="listbox"
            class="palette__results"
          >
            <For each={items()}>
              {(opt, i) => (
                <div
                  id={`move-palette-item-${i()}`}
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
                  <WorkspaceTag option={opt} />
                  <CurrentTick current={props.isCurrent(opt)} />
                </div>
              )}
            </For>
            <Show when={items().length === 0}>
              <div class="palette__empty">{m().workspace.noMatchingLists}</div>
            </Show>
          </div>
          <PaletteFooter enterLabel={m().find.hintMove} />
        </div>
      </Portal>
    </Show>
  );
}
