import { createEffect, createSignal, on, Show, type JSX } from "solid-js";
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
 *  (`spec/inbox-plan.md`). Always a dialog: the choice is where the
 *  list's items go, open in Inbox (the default) or into the Bin. Every
 *  item moves to Inbox either way, so one undo restores the list. */
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
  const [toBin, setToBin] = createSignal(false);
  // Every request starts on "move to Inbox", whatever the last one chose.
  createEffect(
    on(
      () => props.target,
      (t) => {
        if (t) setToBin(false);
      },
    ),
  );
  const name = (): string =>
    props.target
      ? (props.target.app.state.listsById[props.target.listId]?.name ?? "")
      : "";
  const openCount = (): number =>
    props.target
      ? (props.target.app.state.listOpen[props.target.listId]?.length ?? 0)
      : 0;
  const confirm = () => {
    const t = props.target;
    if (!t) return;
    t.before?.();
    t.app.deleteList(t.listId, toBin());
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
      <label class="confirm-dialog-check">
        <input
          type="radio"
          name="delete-list-fate"
          checked={!toBin()}
          onChange={() => setToBin(false)}
        />
        <span>{m().nav.deleteListToInbox(openCount())}</span>
      </label>
      <label class="confirm-dialog-check">
        <input
          type="radio"
          name="delete-list-fate"
          checked={toBin()}
          onChange={() => setToBin(true)}
        />
        <span>{m().nav.deleteListToBin}</span>
      </label>
      <Show when={props.target?.sharedWith}>
        {(ws) => <p class="confirm-dialog-prompt">{m().nav.deleteListShared(ws())}</p>}
      </Show>
    </ConfirmDialog>
  );
}
