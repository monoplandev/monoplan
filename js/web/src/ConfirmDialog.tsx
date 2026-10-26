import { Dialog } from "@kobalte/core/dialog";
import { Show, type JSX } from "solid-js";
import { useAppI18n } from "./i18n";
import { closeToItems, trackOverlay } from "./overlay.ts";

/** In-page replacement for `window.confirm()`. Controlled Kobalte Dialog
 *  in the same portal/overlay/positioner shape as `AuthDialog`. The owner
 *  holds the open signal; ESC and overlay-click both cancel. Confirming
 *  fires `onConfirm` and closes; the button order (cancel left, confirm
 *  right) matches the platform convention.
 *
 *  Layout is three stacked sections — header (title + muted description),
 *  optional body (`children`), and the confirmation row — each with its
 *  own 12px padding and a divider between. `NewDocDialog` shares the same
 *  classes. */
export function ConfirmDialog(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  message: string;
  /** Short heading above the message; every confirmation carries one. */
  title: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Optional body section rendered between the header and the buttons. */
  children?: JSX.Element;
  /** Greys out the confirm button; the owner decides when the dialog's
   *  own gate (a typed name, say) is satisfied. */
  confirmDisabled?: boolean;
  /** Element to focus on open instead of the confirm button, for a body
   *  that wants the keyboard first (an input). Falls back to confirm when
   *  it returns nothing. */
  initialFocus?: () => HTMLElement | undefined;
  onConfirm: () => void;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  let confirmRef: HTMLButtonElement | undefined;
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content
            class="confirm-dialog"
            onCloseAutoFocus={closeToItems}
            onOpenAutoFocus={(e) => {
              // Kobalte focuses the first tabbable (Cancel, on the left);
              // put focus on the confirm button instead so Enter confirms,
              // unless the owner names a body element that should go first.
              e.preventDefault();
              requestAnimationFrame(() =>
                (props.initialFocus?.() ?? confirmRef)?.focus(),
              );
            }}
          >
            <section class="confirm-dialog-section confirm-dialog-header">
              <Dialog.Title class="confirm-dialog-title">
                {props.title}
              </Dialog.Title>
              <Dialog.Description class="confirm-dialog-message">
                {props.message}
              </Dialog.Description>
            </section>
            <Show when={props.children}>
              <section class="confirm-dialog-section confirm-dialog-body">
                {props.children}
              </section>
            </Show>
            <section class="confirm-dialog-section confirm-dialog-actions">
              <button
                type="button"
                class="confirm-dialog-btn confirm-dialog-cancel"
                onClick={() => props.onOpenChange(false)}
              >
                {props.cancelLabel ?? m().common.cancel}
              </button>
              <button
                ref={confirmRef}
                type="button"
                class="confirm-dialog-btn confirm-dialog-confirm"
                disabled={props.confirmDisabled}
                onClick={() => {
                  props.onConfirm();
                  props.onOpenChange(false);
                }}
              >
                {props.confirmLabel ?? m().common.confirm}
              </button>
            </section>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
}
