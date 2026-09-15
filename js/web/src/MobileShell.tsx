// Mobile chrome: two floating glass pills along the bottom edge.
//
// On phones the desktop sidebar (and its footer chrome) is not rendered (see
// Workspace.tsx). The left pill leads with icon-only jumps to the three
// fixed views (Focus, Inbox, Upcoming — the same order as the sidebar's
// top group, minus Done / Bin), then Find (FindSheet.tsx, which doubles
// as the list switcher: it lists every view on an empty query and marks
// the current one), Settings, and the account/sync indicator (`status`,
// mirroring the desktop sidebar footer); the right pill is Add. No
// custom drawer: the DOM can't fake a native sheet convincingly, so we
// don't try.

import { Show, type JSX } from "solid-js";
import { useAppI18n } from "./i18n.tsx";
import archiveSvg from "./icons/archive.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import caretUpDownSvg from "./icons/caret-up-down.svg?raw";
import drawingPinSvg from "./icons/drawing-pin.svg?raw";
import mixerHzSvg from "./icons/mixer-hz.svg?raw";
import plusSvg from "./icons/plus.svg?raw";
import type { ViewKey } from "./prefs.ts";

export function MobileBars(props: {
  /** Current view + navigation for the fixed-view buttons; the one
      matching `view` is marked active. */
  view: ViewKey;
  setView: (v: ViewKey) => void;
  onFind: () => void;
  onOpenSettings: () => void;
  /** null hides the add pill (views that can't capture). */
  onAdd: (() => void) | null;
  addDisabled: boolean;
  /** The account/sync indicator (StatusSlot), rendered last in the left
      pill. Passed in rather than owned here so it stays mounted exactly
      once per session (its first-run auth prompt fires on mount). */
  status: JSX.Element;
}) {
  const { m } = useAppI18n();
  return (
    <>
      <nav class="mobile-bar mobile-bar-left glass" aria-label={m().common.menu}>
        <button
          type="button"
          class="mobile-bar-btn"
          data-active={props.view.kind === "focus" ? "" : undefined}
          aria-label={m().nav.focus}
          onClick={() => props.setView({ kind: "focus" })}
          innerHTML={drawingPinSvg}
        />
        <button
          type="button"
          class="mobile-bar-btn"
          data-active={
            props.view.kind === "list" && props.view.id === "inbox"
              ? ""
              : undefined
          }
          aria-label={m().nav.inbox}
          onClick={() => props.setView({ kind: "list", id: "inbox" })}
          innerHTML={archiveSvg}
        />
        <button
          type="button"
          class="mobile-bar-btn"
          data-active={props.view.kind === "upcoming" ? "" : undefined}
          aria-label={m().nav.upcoming}
          onClick={() => props.setView({ kind: "upcoming" })}
          innerHTML={calendarSvg}
        />
        <button
          type="button"
          class="mobile-bar-btn"
          aria-label={m().find.placeholder}
          onClick={props.onFind}
          innerHTML={caretUpDownSvg}
        />
        <button
          type="button"
          class="mobile-bar-btn"
          aria-label={m().nav.settings}
          onClick={props.onOpenSettings}
          innerHTML={mixerHzSvg}
        />
        {props.status}
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
