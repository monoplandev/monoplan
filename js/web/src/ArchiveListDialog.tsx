import { createEffect, createSignal, on, type JSX } from "solid-js";
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

/** Archive `listId`, optionally moving its open items to the end of
 *  Inbox first (in their list order). One action batch, so a single
 *  undo walks both back. Archive itself stays the pure ListMeta write
 *  of `spec/data-model.md` "Archived lists"; the move is the user's
 *  explicit choice from the dialog. */
export function archiveListNow(
  app: DocApp,
  listId: string,
  moveOpenToInbox: boolean,
): void {
  app.withActionBatch(() => {
    if (moveOpenToInbox) {
      const ids = [...(app.state.listOpen[listId] ?? [])];
      const base = app.state.listOpen.inbox?.length ?? 0;
      ids.forEach((id, i) => app.moveItem(id, "inbox", base + i));
    }
    app.setListArchived(listId, true);
  });
}

/** Archive flow shared by the nav context menu and the view header.
 *  A list with no open items archives at once (non-destructive,
 *  undoable, so no confirmation); one with open items gets the
 *  dialog, whose pre-checked box moves them to Inbox so archiving
 *  never silently hides live work. */
export function createArchiveListFlow(): {
  request: (target: ArchiveTarget) => void;
  dialog: () => JSX.Element;
} {
  const [target, setTarget] = createSignal<ArchiveTarget | null>(null);
  const request = (t: ArchiveTarget) => {
    if ((t.app.state.listOpen[t.listId]?.length ?? 0) === 0) {
      t.before?.();
      archiveListNow(t.app, t.listId, false);
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
  const [moveOpen, setMoveOpen] = createSignal(true);
  // Every request starts with the box checked, whatever the last one chose.
  createEffect(
    on(
      () => props.target,
      (t) => {
        if (t) setMoveOpen(true);
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
    archiveListNow(t.app, t.listId, moveOpen());
  };
  return (
    <ConfirmDialog
      open={props.target !== null}
      onOpenChange={(b) => {
        if (!b) props.onClose();
      }}
      title={m().nav.archiveListTitle(name())}
      message={m().nav.archiveListConfirm}
      confirmLabel={m().nav.archiveList}
      onConfirm={confirm}
    >
      <label class="confirm-dialog-check">
        <input
          type="checkbox"
          checked={moveOpen()}
          onChange={(e) => setMoveOpen(e.currentTarget.checked)}
        />
        <span>{m().nav.archiveListMoveOpen(openCount())}</span>
      </label>
    </ConfirmDialog>
  );
}
