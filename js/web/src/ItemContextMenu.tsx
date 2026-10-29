// The item context menu, shared by every row surface: the list / Focus /
// Done / Bin rows and board cards (`Row.tsx`) and the agenda's rows
// (`Upcoming.tsx`). Actions act on the target set: the selection when the
// right-clicked row is part of it, else that row alone (the row becomes
// the sole selection as the menu opens). The host puts its own
// `<ContextMenu.Trigger>` in `children`; this renders the root and the
// portalled content around it.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import checkSvg from "./icons/check.svg?raw";
import type { DndSelection } from "./dnd/solid";
import { trackOverlay } from "./overlay.ts";
import { statusLabel, useAppI18n } from "./i18n.tsx";
import { itemUrl } from "./url.ts";
import type { ViewKey } from "./prefs.ts";
import { isDraftId } from "./draftId.ts";
import {
  isBinned,
  isCancelled,
  isClosed,
  isClosedState,
  isEvent,
  isOpen,
  EVENT_STATES,
  statusValue,
  WORKFLOW_STATES,
  type ItemActions,
  type ItemView,
  type WorkflowState,
} from "./sync/store.ts";

/** The add-to-focus toggle shows on any open item outside the Focus /
 *  Done / Bin views: list + board lanes, and the agenda's task rows.
 *  Inside the Focus lens the row carries a remove affordance instead.
 *  Drafts never show either. */
export const canPinToFocusIn = (
  viewKind: ViewKey["kind"],
  it: ItemView,
): boolean =>
  (viewKind === "list" || viewKind === "upcoming") &&
  isOpen(it) &&
  !isDraftId(it.id);

export function ItemContextMenu(props: {
  item: () => ItemView;
  /** The item's doc, or the cross-doc layer's routed actions in a view
   *  that mixes docs (`sync/docs.ts`); a multi-row target may then span
   *  docs, and the layer commits once per doc. */
  app: ItemActions;
  selection: DndSelection;
  viewKind: ViewKey["kind"];
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  onOpen?: (id: string) => void;
  /** Jump to the item's other appearance and select it there: from the
   *  Focus lens to its home list, or from a list / board to the Focus
   *  lens. Only offered for items that appear in both (`spec/focus.md`). */
  onReveal?: (id: string, where: "list" | "focus") => void;
  /** Open the workspace's move palette on the target set, the menu twin
   *  of the `m` shortcut. */
  onMoveToList?: (ids: readonly string[]) => void;
  /** Resolves a list id (in the item's doc) to its display label (see
   *  Workspace `listLabel`). */
  listLabel?: (listId: string, docId?: string) => string;
  /** Mirrors the menu's open state to the host, which can't read it off
   *  its own trigger (`Row.tsx` overrides Kobalte's `data-expanded` with
   *  the row's inline-edit state). The row keeps its "…" button showing
   *  while the menu it opened is up. */
  onMenuOpenChange?: (open: boolean) => void;
  /** The host's `<ContextMenu.Trigger>`. */
  children: JSX.Element;
}) {
  const { m } = useAppI18n();
  // Open state of the menu, mirrored into the shared overlay count so
  // global keyboard shortcuts stand down while it's up.
  const [menuOpen, setMenuOpen] = createSignal(false);
  trackOverlay(menuOpen);

  // Whether this row currently has a visible Focus ref. The Focus entry
  // list is small (bounded), so the membership scan is cheap and only
  // re-runs when the Focus lens actually changes. See spec/focus.md.
  const focused = createMemo(() => props.app.isFocused(props.item().id));
  const canPinToFocus = (): boolean =>
    canPinToFocusIn(props.viewKind, props.item());
  const inFocusView = (): boolean =>
    props.viewKind === "focus" && !isDraftId(props.item().id);
  // "Show in <list>" where the row is away from its home list: the Focus
  // lens, and the agenda's task rows (an event's home is the agenda).
  const canShowInList = (): boolean =>
    inFocusView() ||
    (props.viewKind === "upcoming" && !isEvent(props.item()));

  // If the right-clicked row is already in the multi-select, act on the
  // whole selection; otherwise act on this row alone. The onOpenChange
  // hook below makes sure that an unselected row becomes the sole
  // selection before the menu actually opens.
  const targetIds = (): string[] => {
    const id = props.item().id;
    return props.selection.isSelected(id)
      ? props.selection.getSelectedKeys().map(String)
      : [id];
  };
  const binTargets = (): string[] =>
    targetIds().filter((k) => {
      const it = props.app.getItem(k);
      return it !== undefined && !isBinned(it);
    });
  const onBin = () => {
    const ids = binTargets();
    if (ids.length === 0) return;
    props.app.setBinnedMany(ids, true);
  };
  const onMarkDone = () => {
    const ids = targetIds();
    if (ids.length === 0) return;
    props.app.setDoneMany(ids, true);
  };
  // Un-done reopens both Done and Cancelled items (the core's closed →
  // Backlog rule), so Mark not done and Reopen share this.
  const onMarkNotDone = () => {
    const ids = targetIds();
    if (ids.length === 0) return;
    if (props.viewKind === "focus") props.app.undoneIntoFocus(ids);
    else props.app.setDoneMany(ids, false);
  };
  const onCancel = () => {
    const ids = targetIds();
    if (ids.length === 0) return;
    props.app.setLifecycleMany(ids, "cancelled");
  };
  // Status submenu: move the whole target set to one workflow state in a
  // single commit. Targets already in that state are skipped by the core,
  // so a mixed selection converges without re-stamping the ones in place.
  const onSetStatus = (state: WorkflowState) => {
    const ids = targetIds();
    if (ids.length === 0) return;
    // Reopening from inside Focus re-pins first, as un-checking does
    // (closing dropped the ref); the state write then joins the same
    // undo step.
    const reopened =
      props.viewKind === "focus" && !isClosedState(state)
        ? ids.filter((id) => {
            const it = props.app.getItem(id);
            return it !== undefined && isClosed(it);
          })
        : [];
    props.app.withActionBatch(() => {
      if (reopened.length > 0) props.app.undoneIntoFocus(reopened);
      props.app.setLifecycleMany(ids, state);
    });
  };
  // Status pick on an event row. Its submenu offers Event / Done /
  // Cancelled (`EVENT_STATES`); "Event" stands for every open state, so
  // picking it only reopens the targets that are closed. Writing Backlog
  // over the lot would flatten an open state another selected row holds.
  const onPickStatus = (state: WorkflowState) => {
    if (!isEvent(props.item()) || isClosedState(state)) {
      onSetStatus(state);
      return;
    }
    const closed = targetIds().filter((id) => {
      const it = props.app.getItem(id);
      return it !== undefined && isClosed(it) && !isBinned(it);
    });
    if (closed.length === 0) return;
    if (props.viewKind === "focus") props.app.undoneIntoFocus(closed);
    else props.app.setDoneMany(closed, false);
  };
  // Focus toggle from the context menu acts on the whole target set (the
  // selection when this row is part of it, else this row alone) in a single
  // commit, mirroring Mark done / Mark not done. Direction follows the
  // right-clicked row's current focus state; the core skips any target that
  // is already focused / not Open, so a mixed selection resolves cleanly.
  const onToggleFocusSelection = () => {
    const ids = targetIds();
    if (ids.length === 0) return;
    if (focused()) props.app.removeFromFocusMany(ids);
    else props.app.addToFocusMany(ids);
  };
  // Remove from focus within the Focus view. Every row here is already
  // focused, so unlike the list-view toggle this only ever removes — but
  // it must still act on the whole target set, not just the clicked row.
  const onRemoveFromFocusSelection = () => {
    const ids = targetIds();
    if (ids.length === 0) return;
    props.app.removeFromFocusMany(ids);
  };
  // Restore from bin: clear binned only, preserving done state. A
  // done-then-binned item lands back in the Done view; a plain binned
  // item back in its list. The user can flip done off explicitly via
  // the row checkbox or "Mark as not done" if needed.
  const onRestore = () => {
    const ids = targetIds().filter((id) => {
      const it = props.app.getItem(id);
      return it !== undefined && isBinned(it);
    });
    if (ids.length === 0) return;
    props.app.setBinnedMany(ids, false);
  };
  const onDelete = () => {
    const ids = targetIds().filter((id) => {
      const it = props.app.getItem(id);
      return it !== undefined && isBinned(it);
    });
    if (ids.length === 0) return;
    props.app.deleteBinnedMany(ids);
  };
  const onDuplicate = () => {
    props.duplicateBlock(targetIds());
  };
  const onCopy = () => {
    props.copyBlock(targetIds());
  };
  // One shareable `#item_` URL per targeted row (`spec/urls.md`).
  const onCopyLink = () => {
    void navigator.clipboard.writeText(targetIds().map(itemUrl).join("\n"));
  };
  const onOpenChange = (open: boolean) => {
    // Register the menu in the shared overlay count so the workspace's
    // document-level shortcuts (Enter → open dialog, x → done, …) go inert
    // while it's open — otherwise pressing Enter to pick a menu item also
    // triggers the list's Enter handler behind it.
    setMenuOpen(open);
    props.onMenuOpenChange?.(open);
    if (!open) return;
    const id = props.item().id;
    if (!props.selection.isSelected(id)) {
      props.selection.selectOnly(id);
    }
  };
  return (
    <ContextMenu onOpenChange={onOpenChange}>
      {props.children}
      <ContextMenu.Portal>
        <ContextMenu.Content class="context-menu-content">
          <Show when={isOpen(props.item())}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onOpen?.(props.item().id)}
            >
              <span>{m().common.open}</span>
              <kbd class="menu-shortcut">↵</kbd>
            </ContextMenu.Item>
          </Show>
          <Show when={!isClosed(props.item())}>
            <ContextMenu.Item class="context-menu-item" onSelect={onMarkDone}>
              <span>{m().workspace.markDone}</span>
              <kbd class="menu-shortcut">X</kbd>
            </ContextMenu.Item>
          </Show>
          <Show when={isClosed(props.item())}>
            <ContextMenu.Item class="context-menu-item" onSelect={onMarkNotDone}>
              <span>
                {isCancelled(props.item())
                  ? m().workspace.reopen
                  : m().workspace.markNotDone}
              </span>
              <kbd class="menu-shortcut">X</kbd>
            </ContextMenu.Item>
          </Show>
          <Show when={!isClosed(props.item())}>
            <ContextMenu.Item class="context-menu-item" onSelect={onCancel}>
              <span>{m().workspace.markCancelled}</span>
              <kbd class="menu-shortcut">⇧X</kbd>
            </ContextMenu.Item>
          </Show>
          {/* Every workflow state in one place, the tick marking this
              row's current one. Hidden while binned: the bin mask
              overrides the state and Restore is the way out. An event
              has no lanes, so its open ladder is one choice: Event /
              Done / Cancelled (`spec/events-plan.md`). */}
          <Show when={!isBinned(props.item())}>
            <ContextMenu.Sub gutter={4}>
              <ContextMenu.SubTrigger class="context-menu-item">
                <span>{m().workspace.status}</span>
                <span class="menu-sub-arrow" aria-hidden="true">
                  ›
                </span>
              </ContextMenu.SubTrigger>
              <ContextMenu.Portal>
                <ContextMenu.SubContent class="context-menu-content">
                  <ContextMenu.RadioGroup value={statusValue(props.item())}>
                    <For
                      each={
                        isEvent(props.item()) ? EVENT_STATES : WORKFLOW_STATES
                      }
                    >
                      {(state) => (
                        <ContextMenu.RadioItem
                          value={state}
                          class="context-menu-item"
                          // Radio items keep the menu open by default;
                          // picking a state is a one-shot move, so close.
                          // `onSelect` rather than the group's `onChange`
                          // so re-picking this row's state still moves
                          // the rest of a mixed selection.
                          closeOnSelect
                          onSelect={() => onPickStatus(state)}
                        >
                          <span>
                            {statusLabel(m(), state, isEvent(props.item()))}
                          </span>
                          <ContextMenu.ItemIndicator
                            class="menu-check"
                            aria-hidden="true"
                            innerHTML={checkSvg}
                          />
                        </ContextMenu.RadioItem>
                      )}
                    </For>
                  </ContextMenu.RadioGroup>
                </ContextMenu.SubContent>
              </ContextMenu.Portal>
            </ContextMenu.Sub>
          </Show>
          <Show when={canPinToFocus()}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={onToggleFocusSelection}
            >
              {focused() ? m().focus.remove : m().focus.add}
              <kbd class="menu-shortcut">S</kbd>
            </ContextMenu.Item>
          </Show>
          <Show when={inFocusView()}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={onRemoveFromFocusSelection}
            >
              {m().focus.remove}
              <kbd class="menu-shortcut">S</kbd>
            </ContextMenu.Item>
          </Show>
          {/* Cross-appearance jump. Only for items that exist on both
              sides: from the Focus lens to the item's home list, and from
              a list / board row that currently carries a Focus ref. */}
          <Show when={props.onReveal && canShowInList()}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onReveal?.(props.item().id, "list")}
            >
              <span>
                {m().focus.showInList(
                  props.listLabel?.(props.item().listId, props.item().docId) ??
                    props.item().listId,
                )}
              </span>
            </ContextMenu.Item>
          </Show>
          <Show when={props.onReveal && canPinToFocus() && focused()}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onReveal?.(props.item().id, "focus")}
            >
              <span>{m().focus.showInFocus}</span>
            </ContextMenu.Item>
          </Show>
          <Show when={props.onMoveToList}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onMoveToList?.(targetIds())}
            >
              <span>{m().common.move}</span>
              <kbd class="menu-shortcut">M</kbd>
            </ContextMenu.Item>
          </Show>
          <ContextMenu.Item class="context-menu-item" onSelect={onCopy}>
            <span>{m().common.copy}</span>
            <kbd class="menu-shortcut">⌘C</kbd>
          </ContextMenu.Item>
          <Show when={targetIds().length === 1}>
            <ContextMenu.Item class="context-menu-item" onSelect={onCopyLink}>
              <span>{m().common.copyLink}</span>
            </ContextMenu.Item>
          </Show>
          <Show when={isOpen(props.item())}>
            <ContextMenu.Item class="context-menu-item" onSelect={onDuplicate}>
              <span>{m().workspace.duplicate}</span>
              <kbd class="menu-shortcut">⌘D</kbd>
            </ContextMenu.Item>
          </Show>
          <Show when={!isBinned(props.item())}>
            <ContextMenu.Item class="context-menu-item" onSelect={onBin}>
              <span>{m().workspace.moveToBin}</span>
              <kbd class="menu-shortcut">⌫</kbd>
            </ContextMenu.Item>
          </Show>
          <Show when={isBinned(props.item())}>
            <ContextMenu.Item class="context-menu-item" onSelect={onRestore}>
              {m().common.restore}
            </ContextMenu.Item>
            <ContextMenu.Item class="context-menu-item" onSelect={onDelete}>
              <span>{m().common.delete}</span>
              <kbd class="menu-shortcut">⌫</kbd>
            </ContextMenu.Item>
          </Show>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu>
  );
}
