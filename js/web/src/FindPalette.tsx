// cmd/ctrl+f opens an overlay with a search input and a keyboard-
// navigable result list. Backed by the local SearchEngine attached to
// the workspace's DocApp — the palette is a thin view over it. See
// spec/search.md for index semantics and ranking. The query / result-set
// policy and row anatomy live in findResults.tsx, shared with the mobile
// FindSheet; this file owns the desktop interaction model (a keyboard
// cursor, hover-to-select, the footer key legend).

import { createEffect, createSignal, onCleanup, For, Show } from "solid-js";
import { Portal } from "solid-js/web";
import type { DocsLayer } from "./sync/docs.ts";
import { useAppI18n } from "./i18n.tsx";
import { isOverlayOpen, trackOverlay } from "./overlay.ts";
import { PaletteFooter } from "./PaletteFooter.tsx";
import {
  createFindState,
  findResultLifecycle,
  FindResultBody,
  type FindResult,
} from "./findResults.tsx";


export function FindPalette(props: {
  docs: DocsLayer;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelect?: (result: FindResult) => void;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  const find = createFindState(props.docs);
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;
  let listRef: HTMLDivElement | undefined;

  // Global open shortcuts. preventDefault to suppress the browser's native
  // page-find UI — we own these bindings while the app is mounted.
  //   Cmd/Ctrl+F: require exactly one of meta/ctrl and no shift/alt so OS
  //     shortcuts layered on top (e.g. macOS Cmd+Ctrl+F fullscreen) pass
  //     through. Safe to fire even while typing.
  //   `/`: bare key, so it must not steal a literal slash typed into a row
  //     or input — skip when focus sits in an editable surface.
  const onGlobalKeyDown = (e: KeyboardEvent) => {
    // Don't open on top of another modal (Settings, a confirm dialog).
    // The palette itself counts as open here, but that only blocks a
    // redundant re-open while it's already up.
    if (isOverlayOpen()) return;
    if (e.code === "KeyF") {
      if (e.shiftKey || e.altKey) return;
      if (e.metaKey === e.ctrlKey) return;
    } else if (e.key === "/") {
      if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      const target = e.target as Element | null;
      if (target?.closest('input, textarea, [contenteditable="true"]')) return;
    } else {
      return;
    }
    if (e.cancelable) e.preventDefault();
    props.onOpenChange(true);
  };
  document.addEventListener("keydown", onGlobalKeyDown);
  onCleanup(() => document.removeEventListener("keydown", onGlobalKeyDown));

  const items = find.items;

  // Reset selection whenever the result set changes.
  createEffect(() => {
    items();
    setSelectedIndex(0);
  });

  // Reset state and focus input when opening.
  createEffect(() => {
    if (props.open) {
      find.reset();
      setSelectedIndex(0);
      requestAnimationFrame(() => inputRef?.focus());
    }
  });

  // While open: arrows / Enter / Escape, plus click-outside to dismiss.
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
          const item = list[selectedIndex()];
          if (item) selectItem(item);
        }
      }
    };
    const onClick = (e: MouseEvent) => {
      const palette = document.getElementById("find-palette");
      if (palette && !palette.contains(e.target as Node)) {
        props.onOpenChange(false);
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    // Defer the click listener a tick so the click that triggered the
    // open (e.g. via menu) doesn't immediately close it.
    const timer = window.setTimeout(() => {
      document.addEventListener("mousedown", onClick);
    }, 0);

    onCleanup(() => {
      document.removeEventListener("keydown", onKeyDown, true);
      window.clearTimeout(timer);
      document.removeEventListener("mousedown", onClick);
    });
  });

  // Hover-to-select, but only on real pointer motion. Keyboard navigation
  // scrolls the list under a stationary mouse, and browsers then fire
  // mouseenter / (synthetic) mousemove on whichever row slid under the
  // cursor — without this check the selection would snap back to the
  // mouse the moment the list scrolls. Comparing screen coordinates
  // filters those out: a scroll moves the rows, not the pointer.
  let lastMouse: { x: number; y: number } | null = null;
  function onRowMouseMove(e: MouseEvent, index: number) {
    if (lastMouse && lastMouse.x === e.screenX && lastMouse.y === e.screenY) {
      return;
    }
    lastMouse = { x: e.screenX, y: e.screenY };
    if (selectedIndex() !== index) setSelectedIndex(index);
  }

  function scrollSelectedIntoView(index: number) {
    const el = listRef?.querySelector(`[data-index="${index}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }

  function selectItem(item: FindResult) {
    props.onSelect?.(item);
    props.onOpenChange(false);
  }

  return (
    <Show when={props.open}>
      <Portal>
        <div class="palette-overlay" />
        <div
          id="find-palette"
          class="palette"
          role="combobox"
          aria-expanded="true"
          aria-haspopup="listbox"
          aria-owns="find-palette-listbox"
        >
          <div class="palette__search">
            <input
              ref={(el) => {
                inputRef = el;
              }}
              type="text"
              placeholder={m().find.placeholder}
              value={find.input()}
              onInput={(e) => find.setInput(e.currentTarget.value)}
              aria-autocomplete="list"
              aria-controls="find-palette-listbox"
              aria-activedescendant={
                items().length > 0
                  ? `find-palette-item-${selectedIndex()}`
                  : undefined
              }
            />
          </div>
          <div
            ref={listRef}
            id="find-palette-listbox"
            role="listbox"
            class="palette__results"
          >
            <For each={items()}>
              {(item, i) => (
                <div
                  id={`find-palette-item-${i()}`}
                  data-index={i()}
                  role="option"
                  aria-selected={i() === selectedIndex()}
                  class="palette__item"
                  classList={{
                    "palette__item--selected": i() === selectedIndex(),
                    "palette__item--binned": findResultLifecycle(item) === "binned",
                  }}
                  onMouseMove={(e) => onRowMouseMove(e, i())}
                  onClick={() => selectItem(item)}
                >
                  <FindResultBody docs={props.docs} item={item} />
                </div>
              )}
            </For>
            {/* An empty query always yields the default menu (built-ins are
                unconditional), so an empty result set means a query with no
                matches. */}
            <Show when={items().length === 0}>
              <div class="palette__empty">{m().find.noMatches}</div>
            </Show>
          </div>
          <PaletteFooter enterLabel={m().find.hintOpen} />
        </div>
      </Portal>
    </Show>
  );
}
