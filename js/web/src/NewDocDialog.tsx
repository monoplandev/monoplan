import { Dialog } from "@kobalte/core/dialog";
import { Show, createEffect, createSignal } from "solid-js";
import { useAppI18n } from "./i18n";
import { closeToItems, trackOverlay } from "./overlay.ts";

/** "New workspace" modal, opened from the app menu. Same portal /
 *  overlay / positioner shape as `ConfirmDialog`; asks for a name,
 *  creates the doc through `onCreate` (`runtime.createDoc`) and closes
 *  on success. A failure stays open with the error under the field so
 *  the typed name isn't lost. Enter submits, ESC / overlay-click cancel. */
export function NewDocDialog(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  /** Resolves with the new doc's id; rejects on failure. */
  onCreate: (name: string) => Promise<string>;
  /** Called after a successful create (the owner opens the doc). */
  onCreated: (docId: string) => void;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  const [name, setName] = createSignal("");
  const [pending, setPending] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  let inputRef: HTMLInputElement | undefined;

  // Fresh form every time the dialog opens.
  createEffect(() => {
    if (props.open) {
      setName("");
      setPending(false);
      setFailed(false);
    }
  });

  const submit = async (e: Event): Promise<void> => {
    e.preventDefault();
    const trimmed = name().trim();
    if (!trimmed || pending()) return;
    setPending(true);
    setFailed(false);
    try {
      const docId = await props.onCreate(trimmed);
      props.onOpenChange(false);
      props.onCreated(docId);
    } catch (err) {
      console.error("create doc failed:", err);
      setFailed(true);
    } finally {
      setPending(false);
    }
  };

  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content
            class="confirm-dialog"
            onCloseAutoFocus={closeToItems}
            onOpenAutoFocus={(e) => {
              e.preventDefault();
              requestAnimationFrame(() => inputRef?.focus());
            }}
          >
            <form onSubmit={submit}>
              <section class="confirm-dialog-section confirm-dialog-header">
                <Dialog.Title class="confirm-dialog-title">
                  {m().nav.newDoc}
                </Dialog.Title>
              </section>
              <section class="confirm-dialog-section confirm-dialog-body">
                <input
                  ref={inputRef}
                  class="name-dialog-input"
                  type="text"
                  placeholder={m().nav.newDocName}
                  aria-label={m().nav.newDocName}
                  value={name()}
                  disabled={pending()}
                  onInput={(e) => setName(e.currentTarget.value)}
                />
                <Show when={failed()}>
                  <p class="name-dialog-error" role="alert">
                    {m().nav.createDocFailed}
                  </p>
                </Show>
              </section>
              <section class="confirm-dialog-section confirm-dialog-actions">
                <button
                  type="button"
                  class="confirm-dialog-btn confirm-dialog-cancel"
                  onClick={() => props.onOpenChange(false)}
                >
                  {m().common.cancel}
                </button>
                <button
                  type="submit"
                  class="confirm-dialog-btn confirm-dialog-confirm"
                  disabled={!name().trim() || pending()}
                >
                  {m().nav.createDoc}
                </button>
              </section>
            </form>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
}
