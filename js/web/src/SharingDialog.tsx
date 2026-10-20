import { Dialog } from "@kobalte/core/dialog";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems, trackOverlay } from "./overlay.ts";

/** Placeholder "Sharing" modal for a non-Home workspace, opened from its
 *  heading menu. Sharing itself (invites, members; `spec/sharing-plan.md`)
 *  is not built: the dialog reserves the entry point and says so. Same
 *  portal / overlay / positioner shape as `ConfirmDialog`. */
export function SharingDialog(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  /** The workspace's display name. */
  label: string;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content class="confirm-dialog" onCloseAutoFocus={closeToItems}>
            <section class="confirm-dialog-section confirm-dialog-header">
              <Dialog.Title class="confirm-dialog-title">
                {m().nav.sharingTitle(props.label)}
              </Dialog.Title>
              <Dialog.Description class="confirm-dialog-message">
                {m().nav.sharingSoon}
              </Dialog.Description>
            </section>
            <section class="confirm-dialog-section confirm-dialog-actions">
              <button
                type="button"
                class="confirm-dialog-btn confirm-dialog-confirm"
                onClick={() => props.onOpenChange(false)}
              >
                {m().common.close}
              </button>
            </section>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
}
