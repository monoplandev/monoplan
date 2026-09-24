// Mobile Find: the phone's list switcher (and item search), opened from
// the left pill in MobileShell.tsx. Shares FindPalette's query policy and
// row anatomy (findResults.tsx) but not its interaction model — there is
// no keyboard cursor, no hover, no key legend. Instead the row matching
// the view you're on carries a "current" mark, the way a native picker
// shows what's already chosen, and the search input is deliberately not
// autofocused: switching lists is the common case on a phone, and popping
// the keyboard on open would cover half the list before a tap lands.
//
// The top line carries the account/sync indicator (left) and a Settings
// icon (right): the bottom pill carries neither, so this is the phone's
// only way into both. Neither is a FindResult.
//
// Rendered as a full page under the floating pills (which stay live):
// no scrim, no card, no Close. The Find pill toggles it, a row pick or a
// pill jump dismisses it, and a hardware Escape still closes it.

import { createEffect, For, onCleanup, Show, type JSX } from "solid-js";
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

/** Rows the default menu leaves out: the fixed views the bottom pill
 *  already reaches (Focus, Inbox, Upcoming). They still match a typed
 *  query, so search stays complete. */
function onPill(item: FindResult): boolean {
  return (
    item.kind === "view" &&
    (item.id === "focus" || item.id === "inbox" || item.id === "upcoming")
  );
}

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
  /** The account/sync indicator (StatusSlot), top left of the page. Its
   *  popover / sign-in dialog open over the page, which stays put. */
  status: JSX.Element;
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

  const row = (item: FindResult) => (
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
  );

  return (
    <Show when={props.open}>
      <Portal>
        {/* Full-page surface below the pills' z band, so the left pill
            (Find toggles, the fixed views jump) stays the way out. */}
        <div class="find-sheet" role="dialog" aria-label={m().find.placeholder}>
          {/* Top line: the sync / account indicator at the left, Settings
              at the right; the search field on its own line beneath,
              above the cards. */}
          <div class="find-sheet__header">
            {props.status}
            <button
              type="button"
              class="icon-button find-sheet__settings"
              aria-label={m().nav.settings}
              onClick={() => {
                props.onOpenChange(false);
                props.onOpenSettings();
              }}
              innerHTML={mixerHzSvg}
            />
          </div>
          <div class="find-sheet__search">
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
          </div>
          <div ref={listRef} class="find-sheet__results">
            {/* Default menu (empty query): two inset-grouped cards, iOS
                style, one for the fixed views the pill doesn't reach
                (Done, Bin) and one for the user lists. Query results are
                one ranked run in a single card. */}
            <Show
              when={find.input().trim()}
              fallback={
                <>
                  <div class="find-sheet__group">
                    <For each={find.items().filter((i) => isFixedView(i) && !onPill(i))}>
                      {row}
                    </For>
                  </div>
                  <div class="find-sheet__group">
                    <For each={find.items().filter((i) => !isFixedView(i) && !onPill(i))}>
                      {row}
                    </For>
                  </div>
                </>
              }
            >
              {/* An empty query always yields the default menu (built-ins
                  are unconditional), so an empty result set here means a
                  query with no matches. */}
              <Show
                when={find.items().length > 0}
                fallback={<div class="palette__empty">{m().find.noMatches}</div>}
              >
                <div class="find-sheet__group">
                  <For each={find.items()}>{row}</For>
                </div>
              </Show>
            </Show>
          </div>
        </div>
      </Portal>
    </Show>
  );
}
