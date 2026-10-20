import { Dialog } from "@kobalte/core/dialog";
import { Show } from "solid-js";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems, trackOverlay } from "./overlay.ts";
import { createDocSyncFacts } from "./syncFacts.tsx";
import type { DocApp } from "./sync/store.ts";

/** Per-workspace "Advanced" modal, opened from a workspace heading's menu
 *  (`DocSection` in `nav.tsx`). Same portal / overlay / positioner shape
 *  as `ConfirmDialog`. Today it carries the doc's own sync facts — status,
 *  seq, epoch, fingerprint, counts, id — so every open doc gets the
 *  oversight the footer popover only gives Home. The account's device
 *  list is slated to move in here from Settings alongside per-doc
 *  frontiers. */
export function DocAdvancedDialog(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  app: DocApp;
  /** The heading's display name (Personal / doc name / untitled). */
  label: string;
  /** WebSocket link state, shared by every open doc. */
  online: boolean;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  const facts = createDocSyncFacts(props.app, () => props.online);
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content class="confirm-dialog" onCloseAutoFocus={closeToItems}>
            <section class="confirm-dialog-section confirm-dialog-header">
              <Dialog.Title class="confirm-dialog-title">
                {m().nav.advanced}
              </Dialog.Title>
              <Dialog.Description class="confirm-dialog-message">
                {props.label}
              </Dialog.Description>
            </section>
            <section class="confirm-dialog-section confirm-dialog-body doc-advanced-facts">
              <div class="status-line">
                <span class="status-dot" data-state={facts.status()} aria-hidden="true" />
                <span>{facts.statusLabel()}</span>
              </div>
              <div class="status-line status-muted">
                {m().nav.seqLabel(facts.seqLabel())}
              </div>
              <Show when={facts.epochLabel()}>
                {(n) => (
                  <div class="status-line status-muted">{m().nav.epochLabel(n())}</div>
                )}
              </Show>
              <div class="status-fingerprint status-muted status-mono">
                {facts.fingerprintHex()}
              </div>
              <div class="status-line status-muted">
                {m().nav.itemsListsCount(facts.itemsCount(), facts.listsCount())}
              </div>
              <div class="status-line status-muted">
                <span>{m().nav.docId}</span>
                <span class="status-mono doc-advanced-id">{props.app.docId}</span>
              </div>
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
