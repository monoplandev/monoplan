// Mobile Find: the phone's list switcher (and item search), opened from
// the left pill in MobileShell.tsx. Shares FindPalette's query policy and
// row anatomy (findResults.tsx) but not its interaction model — there is
// no keyboard cursor, no hover, no key legend. Instead the row matching
// the view you're on carries a "current" mark, the way a native picker
// shows what's already chosen, and the search input is deliberately not
// autofocused: switching lists is the common case on a phone, and popping
// the keyboard on open would cover half the list before a tap lands.
//
// The default menu (empty query) ends with a Settings row: the bottom
// pill no longer carries Settings, so this is the phone's only way in.
// It is not a FindResult and never appears in query results.
//
// Rendered as a full-screen surface above the floating pills (the
// palette's z band) with an explicit Close, since there is no scrim edge
// to tap outside of.

import { createEffect, For, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import type { DocApp } from "./sync/store.ts";
import type { ViewKey } from "./prefs.ts";
import { useAppI18n } from "./i18n.tsx";
import { trackOverlay } from "./overlay.ts";
import mixerHzSvg from "./icons/mixer-hz.svg?raw";
import {
  createFindState,
  findResultLifecycle,
  FindResultBody,
  isFixedView,
  type FindResult,
} from "./findResults.tsx";

/** Whether a result row denotes the view currently on screen. Items are
 *  never "current" — they live inside a view rather than being one. */
function isCurrent(item: FindResult, view: ViewKey): boolean {
  if (item.kind === "view") {
    if (item.id === "inbox") return view.kind === "list" && view.id === "inbox";
    return view.kind === item.id;
  }
  if (item.kind === "list") return view.kind === "list" && view.id === item.id;
  return false;
}

export function FindSheet(props: {
  app: DocApp;
  open: boolean;
  view: ViewKey;
  onOpenChange: (open: boolean) => void;
  onSelect?: (result: FindResult) => void;
  /** Opens the Settings dialog; the sheet closes first. */
  onOpenSettings: () => void;
  /** Count badges, same sources and rules as the desktop nav (see
   *  `Nav`): Focus shows only when non-zero, Bin always, Inbox always
   *  ("-" for zero), other lists only under `showListCounts`. */
  focusCount: number;
  binCount: number;
  openCountsByList: Record<string, number>;
  showListCounts: boolean;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  const find = createFindState(props.app);

  // Trailing count for a row, or null for rows that carry none (Upcoming,
  // Done, item results, user lists with counts switched off).
  const countLabel = (item: FindResult): string | null => {
    const openCount = (id: string) => {
      const n = props.openCountsByList[id] ?? 0;
      return n > 0 ? String(n) : "-";
    };
    if (item.kind === "view") {
      if (item.id === "focus") return props.focusCount > 0 ? String(props.focusCount) : null;
      if (item.id === "bin") return String(props.binCount);
      if (item.id === "inbox") return openCount("inbox");
      return null;
    }
    if (item.kind === "list") return props.showListCounts ? openCount(item.id) : null;
    return null;
  };
  let listRef: HTMLDivElement | undefined;

  // Fresh query on every open, and bring the current row into view so a
  // long list of lists opens on "where you are" rather than the top.
  createEffect(() => {
    if (!props.open) return;
    find.reset();
    requestAnimationFrame(() => {
      listRef
        ?.querySelector('[aria-current="true"]')
        ?.scrollIntoView({ block: "center" });
    });
  });

  // A hardware keyboard's Escape still dismisses, matching every other
  // overlay; capture-phase so the workspace shortcuts behind never see it.
  createEffect(() => {
    if (!props.open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      props.onOpenChange(false);
    };
    document.addEventListener("keydown", onKeyDown, true);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown, true));
  });

  function selectItem(item: FindResult) {
    props.onSelect?.(item);
    props.onOpenChange(false);
  }

  return (
    <Show when={props.open}>
      <Portal>
        {/* Scrim + floating card: the outer layer dims the page (and hides
            the pills beneath); a tap on it, outside the card, dismisses. */}
        <div
          class="find-sheet"
          onClick={(e) => {
            if (e.target === e.currentTarget) props.onOpenChange(false);
          }}
        >
        <div class="find-sheet__panel" role="dialog" aria-label={m().find.placeholder}>
          <div class="find-sheet__header">
            <input
              class="find-sheet__input"
              type="search"
              placeholder={m().find.placeholder}
              value={find.input()}
              onInput={(e) => find.setInput(e.currentTarget.value)}
              enterkeyhint="search"
              autocapitalize="off"
              autocorrect="off"
            />
            <button
              type="button"
              class="find-sheet__close"
              onClick={() => props.onOpenChange(false)}
            >
              {m().common.close}
            </button>
          </div>
          <div ref={listRef} class="find-sheet__results">
            <For each={find.items()}>
              {(item, index) => (
                <>
                {/* Default menu only: the sidebar's break between the
                    fixed views and the lists group (Inbox + user lists),
                    drawn above the first non-fixed row. Query results
                    are one ranked run and get no break. */}
                <Show
                  when={
                    !find.input().trim() &&
                    index() > 0 &&
                    !isFixedView(item) &&
                    isFixedView(find.items()[index() - 1]!)
                  }
                >
                  <div class="find-sheet__divider" role="separator" />
                </Show>
                <button
                  type="button"
                  class="palette__item find-sheet__row"
                  classList={{
                    "palette__item--binned": findResultLifecycle(item) === "binned",
                  }}
                  aria-current={isCurrent(item, props.view) ? "true" : undefined}
                  onClick={() => selectItem(item)}
                >
                  <FindResultBody app={props.app} item={item} />
                  <Show when={countLabel(item)}>
                    {(count) => <span class="find-sheet__count">{count()}</span>}
                  </Show>
                </button>
                </>
              )}
            </For>
            {/* An empty query always yields the default menu (built-ins are
                unconditional), so an empty result set means a query with no
                matches. */}
            <Show when={find.items().length === 0}>
              <div class="palette__empty">{m().find.noMatches}</div>
            </Show>
            {/* Default menu only: Settings sits below the lists, behind
                its own break, as the last thing in the switcher. */}
            <Show when={!find.input().trim()}>
              <div class="find-sheet__divider" role="separator" />
              <button
                type="button"
                class="palette__item find-sheet__row"
                onClick={() => {
                  props.onOpenChange(false);
                  props.onOpenSettings();
                }}
              >
                <span class="palette__item-icon" innerHTML={mixerHzSvg} aria-hidden="true" />
                <span class="palette__item-name">{m().nav.settings}</span>
              </button>
            </Show>
          </div>
        </div>
        </div>
      </Portal>
    </Show>
  );
}
