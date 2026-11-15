import { createSignal, Show, type JSX } from "solid-js";
import { ConfirmDialog } from "./ConfirmDialog.tsx";
import { useAppI18n } from "./i18n.tsx";
import type { DocApp } from "./sync/store.ts";

/** One delete request: which doc / list, the workspace name when other
 *  people will see the result (a shared workspace), and an optional
 *  `before` hook the caller runs right before the mutation (the nav
 *  hops off a list that is on screen, since its row is about to go). */
export type DeleteTarget = {
  app: DocApp;
  listId: string;
  sharedWith?: string;
  before?: () => void;
};

/** Delete flow shared by the nav context menu and the view header
 *  (`spec/inbox-plan.md`). A plain confirm: the list's open items go to
 *  the Bin, its done and cancelled ones to Inbox, closed. One undo
 *  restores the list and its items. */
export function createDeleteListFlow(): {
  request: (target: DeleteTarget) => void;
  dialog: () => JSX.Element;
} {
  const [target, setTarget] = createSignal<DeleteTarget | null>(null);
  return {
    request: setTarget,
    dialog: () => (
      <DeleteListDialog target={target()} onClose={() => setTarget(null)} />
    ),
  };
}

export function DeleteListDialog(props: {
  target: DeleteTarget | null;
  onClose: () => void;
}) {
  const { m } = useAppI18n();
  const name = (): string =>
    props.target
      ? (props.target.app.state.listsById[props.target.listId]?.name ?? "")
      : "";
  const confirm = () => {
    const t = props.target;
    if (!t) return;
    t.before?.();
    t.app.deleteList(t.listId, true);
  };
  return (
    <ConfirmDialog
      open={props.target !== null}
      onOpenChange={(b) => {
        if (!b) props.onClose();
      }}
      title={m().nav.deleteListTitle(name())}
      confirmLabel={m().nav.deleteListConfirm}
      onConfirm={confirm}
    >
      <p class="confirm-dialog-prompt">{m().nav.deleteListToBin}</p>
      <Show when={props.target?.sharedWith}>
        {(ws) => <p class="confirm-dialog-prompt">{m().nav.deleteListShared(ws())}</p>}
      </Show>
    </ConfirmDialog>
  );
}
