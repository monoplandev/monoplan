// Mobile chrome: two floating glass pills along the bottom edge.
//
// On phones the desktop sidebar (and its footer chrome) is not rendered (see
// Workspace.tsx). The left pill leads with icon-only jumps to the three
// fixed views (Focus, Inbox, Upcoming, the same order as the sidebar's
// top group, minus Done / Bin), then Menu (FindSheet.tsx: a page of its
// own, with a `#menu` route and history entry, that doubles as the list
// switcher and item search: on an empty query it lists Done, Bin and the
// user lists (the views this pill already reaches are left out), marks
// the current one, and carries the account/sync indicator and Settings;
// the page sits under these pills and the Menu button opens it, marked
// active while it's up like any view's pill); the right pill is Add, which the menu page
// hides (nothing there to capture into). No custom drawer: the DOM can't
// fake a native sheet convincingly, so we don't try.
//
// Inbox and Upcoming carry a count bubble on the icon's corner, fed by the
// same sources as the desktop nav (`openCountsByList["inbox"]` and
// `attentionBadge`), so a glance at the pill says whether anything is
// waiting. Zero renders nothing rather than "-": there is no room for a
// placeholder on an icon, and an absent bubble already reads as clear.

import { Show } from "solid-js";
import type { AttentionBadge } from "./dayGroups.ts";
import { useAppI18n } from "./i18n.tsx";
import archiveSvg from "./icons/archive.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import starSvg from "./icons/star.svg?raw";
import hamburgerMenuSvg from "./icons/hamburger-menu.svg?raw";
import plusSvg from "./icons/plus.svg?raw";
import type { ViewKey } from "./prefs.ts";

export function MobileBars(props: {
  /** Current view + navigation for the fixed-view buttons; the one
      matching `view` is marked active. */
  view: ViewKey;
  setView: (v: ViewKey) => void;
  /** Opens the menu page; `menuOpen` marks the button while it's up. */
  onMenu: () => void;
  menuOpen: boolean;
  /** null hides the add pill (views that can't capture). */
  onAdd: (() => void) | null;
  addDisabled: boolean;
  /** Inbox's open-item count (Backlog + Live); 0 hides the bubble. */
  inboxCount: number;
  /** Upcoming's "needs you today" count and urgency tone (see
      `attentionBadge`); 0 hides the bubble. */
  upcomingBadge: AttentionBadge;
}) {
  const { m } = useAppI18n();
  // One active pill at a time: while the menu page is up it covers the
  // view, so the view's own button gives up its mark to the Menu button.
  const viewActive = (on: boolean) => (on && !props.menuOpen ? "" : undefined);
  return (
    <>
      {/* Left pill: icon over a small text label per button (the label
          is the accessible name, so no aria-label). */}
      <nav class="mobile-bar mobile-bar-left glass" aria-label={m().common.menu}>
        <button
          type="button"
          class="mobile-bar-btn mobile-bar-btn--labelled"
          data-active={viewActive(props.view.kind === "focus")}
          onClick={() => props.setView({ kind: "focus" })}
        >
          <span class="mobile-bar-icon" innerHTML={starSvg} aria-hidden="true" />
          <span class="mobile-bar-label">{m().nav.focus}</span>
        </button>
        <button
          type="button"
          class="mobile-bar-btn mobile-bar-btn--labelled"
          data-active={viewActive(
            props.view.kind === "list" && props.view.id === "inbox",
          )}
          onClick={() => props.setView({ kind: "list", id: "inbox" })}
        >
          <span class="mobile-bar-icon" aria-hidden="true">
            <span innerHTML={archiveSvg} />
            <Show when={props.inboxCount > 0}>
              <span class="mobile-bar-count">{props.inboxCount}</span>
            </Show>
          </span>
          <span class="mobile-bar-label">{m().nav.inbox}</span>
        </button>
        <button
          type="button"
          class="mobile-bar-btn mobile-bar-btn--labelled"
          data-active={viewActive(props.view.kind === "upcoming")}
          onClick={() => props.setView({ kind: "upcoming" })}
        >
          <span class="mobile-bar-icon" aria-hidden="true">
            <span innerHTML={calendarSvg} />
            <Show when={props.upcomingBadge.count > 0}>
              <span class="mobile-bar-count" data-tone={props.upcomingBadge.tone}>
                {props.upcomingBadge.count}
              </span>
            </Show>
          </span>
          <span class="mobile-bar-label">{m().nav.upcoming}</span>
        </button>
        <button
          type="button"
          class="mobile-bar-btn mobile-bar-btn--labelled"
          data-active={props.menuOpen ? "" : undefined}
          aria-current={props.menuOpen ? "page" : undefined}
          onClick={props.onMenu}
        >
          <span class="mobile-bar-icon" innerHTML={hamburgerMenuSvg} aria-hidden="true" />
          <span class="mobile-bar-label">{m().common.menu}</span>
        </button>
      </nav>
      <Show when={props.onAdd}>
        {(onAdd) => (
          <div class="mobile-bar mobile-bar-right glass">
            <button
              type="button"
              class="mobile-bar-btn"
              aria-label={m().common.add}
              disabled={props.addDisabled}
              onClick={(e) => {
                // Stop the dnd's document-level collapse handler from
                // immediately closing the new draft (same as the FAB did).
                e.stopImmediatePropagation();
                onAdd()();
              }}
              innerHTML={plusSvg}
            />
          </div>
        )}
      </Show>
    </>
  );
}
