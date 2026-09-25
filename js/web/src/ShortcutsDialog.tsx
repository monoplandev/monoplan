// Keyboard-shortcut cheat sheet, toggled with `?`. A plain reference list —
// the shortcuts themselves live in Workspace.tsx / Row.tsx; this just
// documents them. Registered with trackOverlay so it suppresses the global
// shortcuts while open, like the other dialogs; `?` is handled on the
// dialog itself so it closes the sheet too.

import { Dialog } from "@kobalte/core/dialog";
import { createEffect, For, onCleanup } from "solid-js";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems, trackOverlay } from "./overlay.ts";

export function ShortcutsDialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);

  // Listen on `document` rather than the content element: the sheet has
  // no tabbable controls, so where focus lands on open is Kobalte's call
  // and the keystroke may never reach the content node.
  createEffect(() => {
    if (!props.open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "?") return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      e.preventDefault();
      props.onOpenChange(false);
    };
    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });

  const rows = (): { label: string; key: string | string[] }[] => {
    const s = m().shortcuts;
    return [
      { label: s.newItem, key: "Space" },
      { label: s.openItem, key: "Enter" },
      { label: s.toggleDone, key: "X" },
      { label: s.toggleCancelled, key: "⇧X" },
      { label: s.toggleFocus, key: "F" },
      { label: s.moveToList, key: "M" },
      { label: s.duplicate, key: "⌘D" },
      { label: s.copy, key: "⌘C" },
      { label: s.undo, key: "⌘Z" },
      { label: s.redo, key: "⌘⇧Z" },
      { label: s.bin, key: "⌫" },
      { label: s.switchList, key: "[ ]" },
      { label: s.goToView, key: ["1", "2", "3", "4"] },
      { label: s.switchLane, key: "← →" },
      { label: s.find, key: ["⌘F", "/"] },
      { label: s.showShortcuts, key: "?" },
    ];
  };

  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content
            class="shortcuts-dialog"
            onCloseAutoFocus={closeToItems}
          >
            <Dialog.Title class="shortcuts-dialog-title">
              {m().shortcuts.title}
            </Dialog.Title>
            <div class="shortcuts-dialog-list">
              <For each={rows()}>
                {(r) => (
                  <div class="shortcuts-dialog-row">
                    <span>{r.label}</span>
                    <span class="shortcuts-dialog-keys">
                      <For each={Array.isArray(r.key) ? r.key : [r.key]}>
                        {(k) => <kbd class="menu-shortcut">{k}</kbd>}
                      </For>
                    </span>
                  </div>
                )}
              </For>
            </div>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
}
