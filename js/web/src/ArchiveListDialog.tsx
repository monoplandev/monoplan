import { createSignal, type JSX } from "solid-js";
import { ConfirmDialog } from "./ConfirmDialog.tsx";
import { useAppI18n } from "./i18n.tsx";
import type { DocApp } from "./sync/store.ts";

/** One archive request: which doc / list, plus an optional `before`
 *  hook the caller runs right before the mutation (the nav hops off a
 *  list that is on screen). */
export type ArchiveTarget = {
  app: DocApp;
  listId: string;
  before?: () => void;
};

/** Archive flow shared by the nav context menu and the view header.
 *  Archive is the pure ListMeta write of `spec/data-model.md` "Archived
 *  lists": the list is shelved with its items in place. A list with no
 *  open items archives at once (non-destructive, undoable, so no
 *  confirmation); one with open items gets a confirm naming how many
 *  stay with it, so archiving never silently hides live work. */
export function createArchiveListFlow(): {
  request: (target: ArchiveTarget) => void;
  dialog: () => JSX.Element;
} {
  const [target, setTarget] = createSignal<ArchiveTarget | null>(null);
  const request = (t: ArchiveTarget) => {
    if ((t.app.state.listOpen[t.listId]?.length ?? 0) === 0) {
      t.before?.();
      t.app.setListArchived(t.listId, true);
      return;
    }
    setTarget(t);
  };
  return {
    request,
    dialog: () => (
      <ArchiveListDialog target={target()} onClose={() => setTarget(null)} />
    ),
  };
}

export function ArchiveListDialog(props: {
  target: ArchiveTarget | null;
  onClose: () => void;
}) {
  const { m } = useAppI18n();
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
    t.app.setListArchived(t.listId, true);
  };
  return (
    <ConfirmDialog
      open={props.target !== null}
      onOpenChange={(b) => {
        if (!b) props.onClose();
      }}
      title={m().nav.archiveListTitle(name())}
      confirmLabel={m().nav.archiveList}
      onConfirm={confirm}
    >
      <p class="confirm-dialog-prompt">{m().nav.archiveListKeepsOpen(openCount())}</p>
    </ConfirmDialog>
  );
}
