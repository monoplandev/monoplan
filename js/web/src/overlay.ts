// Central coordination between overlays (modals, dialogs, the find
// palette) and the workspace's global keyboard shortcuts.
//
// The shortcuts in Workspace.tsx / FindPalette.tsx listen on `document`,
// so a keystroke aimed at an open modal (pressing a non-editable button
// inside it, or just having the dialog focused) still bubbles through to
// the list behind it — pressing `x` toggled done, Space started a draft,
// Delete binned rows, etc. Kobalte's Dialog traps focus and swallows
// Escape, but plain keydown still reaches document.
//
// Rather than teach every shortcut about every modal, overlays opt in to
// a shared open-count and shortcuts consult it. The count (not a boolean)
// so stacked overlays — a ConfirmDialog over Settings — settle correctly.

import { createEffect, createSignal, onCleanup } from "solid-js";

const [overlayCount, setOverlayCount] = createSignal(0);

/** True while any tracked overlay is open. */
export const isOverlayOpen = () => overlayCount() > 0;

/** How many tracked overlays are open: a full-page surface that is
 *  itself tracked (the phone's menu page) reads this to tell whether a
 *  dialog sits over it. */
export const overlayDepth = () => overlayCount();

/** Mark an overlay as open while `open()` is true. Increments the shared
 *  count on open, decrements on close or unmount. Call from the overlay
 *  component's body: `trackOverlay(() => props.open)`. */
export function trackOverlay(open: () => boolean): void {
  createEffect(() => {
    if (!open()) return;
    setOverlayCount((c) => c + 1);
    onCleanup(() => setOverlayCount((c) => c - 1));
  });
}

// Where focus goes when an overlay closes. Kobalte's default is "back to
// whatever opened the dialog" (a menu item, a row's icon button, …), but
// the workspace's keyboard model wants it on the items listbox so nav
// resumes there. The workspace registers its listbox-focus routine here
// and every dialog's `onCloseAutoFocus` sends focus to it — no per-mount
// prop plumbing, and nested pickers (a calendar inside the task surface)
// simply don't opt in.
let focusHome: (() => void) | null = null;

/** Register the routine that focuses the items listbox. Call from the
 *  workspace's reactive scope; unregisters on unmount. */
export function registerFocusHome(fn: () => void): void {
  focusHome = fn;
  onCleanup(() => {
    if (focusHome === fn) focusHome = null;
  });
}

/** Focus the items listbox, if a workspace is mounted. */
export function focusItems(): void {
  focusHome?.();
}

/** Kobalte `onCloseAutoFocus` handler: send focus to the items listbox
 *  instead of the element that opened the dialog. Falls through to
 *  Kobalte's default when no workspace is registered. */
export function closeToItems(e: Event): void {
  if (!focusHome) return;
  e.preventDefault();
  focusHome();
}

/** Register a document-level keyboard shortcut that is inert whenever an
 *  overlay is open or focus sits in an editable surface (input, textarea,
 *  contenteditable) — or anywhere inside a `data-shortcuts-inert` region:
 *  the non-modal task shells (side pane, mobile page) hold buttons and
 *  badges whose keystrokes must not drive the list behind them.
 *  Centralises the guards every workspace shortcut used to repeat, and
 *  auto-cleans on unmount — so it must be called from a component/reactive
 *  scope where `onCleanup` is bound. */
export function onGlobalKey(handler: (e: KeyboardEvent) => void): void {
  const listener = (e: KeyboardEvent) => {
    if (isOverlayOpen()) return;
    const target = e.target as Element | null;
    if (
      target?.closest(
        'input, textarea, [contenteditable="true"], [data-shortcuts-inert]',
      )
    )
      return;
    handler(e);
  };
  document.addEventListener("keydown", listener);
  onCleanup(() => document.removeEventListener("keydown", listener));
}
