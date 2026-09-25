import { createEffect, createMemo, createSignal, on, Show } from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import checkSvg from "./icons/check.svg?raw";
import crossSvg from "./icons/cross.svg?raw";
import drawingPinFilledSvg from "./icons/drawing-pin-filled.svg?raw";
import noteSvg from "./icons/note.svg?raw";
import { DndSelection } from "./dnd/solid";
import { trackOverlay } from "./overlay.ts";
import { DeadlineBadge } from "./DeadlineBadge.tsx";
import { WhenBadge } from "./WhenBadge.tsx";
import {
  addDaysToStamp,
  formatDateTime,
  formatDoneStamp,
  formatRelative,
  nowMs,
  todayStamp,
  whenFromParts,
  whenTime,
} from "./format.tsx";
import { laneLabel, useAppI18n } from "./i18n.tsx";
import { pasteAsPlainText } from "./plainTextPaste.ts";
import { openLinkOnClick, setLinkifiedText } from "./linkify.ts";
import { itemUrl } from "./url.ts";
import type { ViewKey } from "./prefs.ts";
import {
  isBinned,
  isCancelled,
  isClosed,
  isOpen,
  type DocApp,
  type ItemView,
} from "./sync/store.ts";

// Draft items live only in the dnd's items list — never in the engine —
// until the user commits them. The id prefix is the discriminator the
// Row uses to switch between "edit existing" and "create new" save paths
// on collapse.
export const DRAFT_ID_PREFIX = "__draft__";
export const isDraftId = (id: string): boolean => id.startsWith(DRAFT_ID_PREFIX);

// Surface the most recent state-changing timestamp. Binned wins over
// closed because it's the later transition: a done-then-binned item shows
// when it was binned in the Bin view; a plain done or cancelled item shows
// the register's closing transition time in the Done view. Open rows show
// none.
function lifecycleTimestamp(it: ItemView): number | undefined {
  return it.binnedAt ?? (isClosed(it) ? it.lifecycleAt : undefined);
}

export function Row(props: {
  item: () => ItemView;
  expanded: () => boolean;
  app: DocApp;
  selection: DndSelection;
  viewKind: ViewKey["kind"];
  duplicateBlock: (sourceIds: readonly string[]) => void;
  copyBlock: (sourceIds: readonly string[]) => void;
  /** Called by a draft row from its collapse effect with the trimmed
   *  edit text and the notes textarea contents. Empty text means drop
   *  the draft; non-empty means the workspace should commit it as a
   *  real item. `chain` is true when the collapse was driven by Enter —
   *  the workspace re-opens a fresh draft so capture continues until
   *  Escape / blur / empty-Enter. */
  onDraftSettle?: (text: string, chain: boolean) => void;
  /** Open this item in the detail dialog. `focus` picks which field the
   *  dialog lands the caret in — the note badge opens straight to notes. */
  onOpen?: (id: string) => void;
  /** When true (mobile), a plain tap on the row opens the dialog instead
   *  of only selecting — inline editing is unpleasant on touch. */
  openOnTap?: () => boolean;
  /** Board cards pin the deadline badge to a footer at the bottom of the card;
   *  list rows show it inline after the title instead. */
  deadlineInFooter?: boolean;
  /** Done / Focus views: when true, badge the row with its origin list name.
   *  Resolved via `listLabel` so `main` shows the Home label. */
  showList?: () => boolean;
  /** Resolves a list id to its display label (see Workspace `listLabel`). */
  listLabel?: (listId: string) => string;
  /** Flat list view: when true, badge each open row with its lifecycle
   *  state (client-local per-list display option). The board shows state
   *  as the lane itself, so cards never take it. */
  showState?: () => boolean;
  /** Open the shared calendar modal to set a deadline on the target set.
   *  `initial` seeds the calendar (this row's current deadline, or null). */
  onSetDeadline?: (ids: readonly string[], initial: string | null) => void;
  /** Open the shared calendar modal (with its time field) to set a
   *  planned date on the target set. `initial` seeds it. */
  onSetWhen?: (ids: readonly string[], initial: string | null) => void;
  /** Jump to the item's other appearance and select it there: from the
   *  Focus lens to its home list, or from a list / board to the Focus
   *  lens. Only offered for items that appear in both (`spec/focus.md`). */
  onReveal?: (id: string, where: "list" | "focus") => void;
  /** Open the workspace's move palette on the target set (the selection
   *  when this row is part of it, else this row alone) — the menu twin of
   *  the `m` shortcut. */
  onMoveToList?: (ids: readonly string[]) => void;
}) {
  const { m, locale } = useAppI18n();
  // Origin-list badge text for the flat cross-list views (Focus and Done),
  // shown when the view's "show list" toggle is on. Null otherwise, or
  // when no label resolves.
  const originList = createMemo(() => {
    return (props.viewKind === "focus" || props.viewKind === "done") &&
      props.showList?.()
      ? (props.listLabel?.(props.item().listId) || null)
      : null;
  });
  // Lifecycle-state badge text for the flat list view, when the list's
  // "show state" option is on. Open items only: done rows already read
  // as done (strike-through + timestamp) and a lingering done row would
  // otherwise flash a "Done" pill for its last three seconds.
  const stateBadge = createMemo(() => {
    return props.viewKind === "list" &&
      !props.deadlineInFooter &&
      props.showState?.() &&
      isOpen(props.item())
      ? laneLabel(m(), props.item().state)
      : null;
  });
  // Lifecycle stamp shown on the row. Only the Done and Bin views carry
  // one: a just-ticked row lingering in a list / Focus / board lane is
  // about to leave, and stamping it would only flash "just now".
  const rowStamp = () =>
    props.viewKind === "done" || props.viewKind === "bin"
      ? lifecycleTimestamp(props.item())
      : undefined;
  // The global Done view leads with the stamp, sat right of the checkbox
  // and without the check glyph (the ticked box already says done). Bin
  // rows and board Done cards keep it trailing with the other badges.
  const leadingStamp = () => props.viewKind === "done" && !props.deadlineInFooter;
  // Open state of this row's context menu, mirrored into the shared
  // overlay count so global keyboard shortcuts stand down while it's up.
  const [menuOpen, setMenuOpen] = createSignal(false);
  trackOverlay(menuOpen);

  // Whether the item carries any notes text. Whitespace-only notes don't
  // count — the dialog would open to an empty-looking editor.
  const hasNotes = () => props.item().notes.trim().length > 0;
  // Purely an indicator, like the focus badge: it has no click behaviour
  // of its own, so a press on it is just a press on the row.
  const NotesBadge = () => (
    <span
      class="badge row-notes-badge"
      title={m().workspace.hasNotes}
      aria-label={m().workspace.hasNotes}
      innerHTML={noteSvg}
    />
  );

  // Whether this row currently has a visible Focus ref. `focusOrder` is a
  // small (bounded) store array, so the membership scan is cheap and only
  // re-runs when the Focus lens actually changes. See spec/focus.md.
  const focused = createMemo(() =>
    props.app.state.focusOrder.includes(props.item().id),
  );
  // The add-to-focus toggle shows on any open item outside the Focus /
  // Done / Bin views (i.e. list + board lanes). Inside the Focus lens the
  // row carries a remove (×) affordance instead. Drafts never show either.
  const canPinToFocus = (): boolean =>
    props.viewKind === "list" &&
    isOpen(props.item()) &&
    !isDraftId(props.item().id);
  const inFocusView = (): boolean =>
    props.viewKind === "focus" && !isDraftId(props.item().id);

  // Any inline (non-footer) badge visible on this list row? Mirrors the
  // individual `<Show>` guards below so the `.row-badges` container only
  // renders when it has children — an empty flex box would still eat a
  // slot of the row's gap.
  const showInlineBadges = () =>
    Boolean(originList()) ||
    Boolean(stateBadge()) ||
    (!props.deadlineInFooter &&
      ((!leadingStamp() && Boolean(rowStamp())) ||
        (!props.expanded() &&
          ((canPinToFocus() && focused()) ||
            (isOpen(props.item()) &&
              (Boolean(props.item().when) || Boolean(props.item().deadline)))))));
  let textRef!: HTMLSpanElement;
  // Set by the Enter keydown handler before it dispatches the synthetic
  // Escape that drives collapse. The collapse effect reads (and resets)
  // it so the workspace can tell Enter-commit from Escape/blur and chain
  // a fresh draft only on Enter.
  let chainOnSettle = false;

  // Order matters: this effect must run *before* the model-mirror effect
  // below. On collapse, both fire in the same tick — if the mirror runs
  // first, it overwrites the user's edit with the stale model text and
  // we save nothing.
  createEffect(
    on(
      props.expanded,
      (now, prev) => {
        if (!prev && now) {
          // Inline expansion is now only reached by a new draft (existing
          // items open the dialog instead), so the caret always lands at
          // the end of the text — typing appends rather than overwriting
          // a select-all.
          // Swap plain text for linkified anchors so URLs become clickable
          // while the row is editable. The collapse path & mirror effect
          // restore plain text, so anchors only exist in expanded rows.
          setLinkifiedText(textRef, props.item().text);
          queueMicrotask(() => {
            textRef.focus();
            const sel = window.getSelection();
            const range = document.createRange();
            range.selectNodeContents(textRef);
            range.collapse(false);
            sel?.removeAllRanges();
            sel?.addRange(range);
          });
          return;
        }
        if (prev && !now) {
          const chain = chainOnSettle;
          chainOnSettle = false;
          const next = (textRef.textContent ?? "").trim();
          // Draft path: the row is a transient pseudo-item that has no
          // engine-side record yet. Hand the trimmed text back to the
          // workspace, which decides commit (addItemAt) vs drop. Skip the
          // editItemText path — there's no item to edit.
          if (isDraftId(props.item().id)) {
            props.onDraftSettle?.(next, chain);
            // When chaining, the workspace will open a fresh draft and
            // its expand effect will steal focus to the new row's
            // contentEditable; bouncing focus to the listbox here would
            // race that. Skip the listbox refocus on the chain path.
            if (!chain) {
              const listbox = textRef.closest<HTMLElement>('[role="listbox"]');
              listbox?.focus();
            }
            return;
          }
          const current = props.item().text;
          if (!next) {
            textRef.textContent = current;
          } else if (next !== current) {
            props.app.editItemText(props.item().id, next);
          }
          // Focus is still on the now-non-editable span; bounce it back
          // to the dnd listbox so arrow-key nav works without a click.
          const listbox = textRef.closest<HTMLElement>('[role="listbox"]');
          listbox?.focus();
        }
      },
      { defer: true },
    ),
  );

  // Mirror the model into the DOM while not expanded. While expanded
  // we leave the DOM alone so live edits aren't clobbered by reactive
  // updates from peer text changes. Plain text only — the expand path
  // swaps in linkified anchors so URLs are only clickable while editing.
  createEffect(() => {
    if (!props.expanded()) textRef.textContent = props.item().text;
  });

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
  // Deadline actions apply to the whole target set (the selection when the
  // row is part of it, else this row alone), matching the done/bin actions.
  const onDeadlineToday = () => {
    const stamp = todayStamp(nowMs());
    props.app.withActionBatch(() => {
      for (const id of targetIds()) props.app.setItemDeadline(id, stamp);
    });
  };
  const onDeadlineTomorrow = () => {
    const stamp = addDaysToStamp(todayStamp(nowMs()), 1);
    props.app.withActionBatch(() => {
      for (const id of targetIds()) props.app.setItemDeadline(id, stamp);
    });
  };
  const onDeadlineRemove = () => {
    props.app.withActionBatch(() => {
      for (const id of targetIds()) props.app.setItemDeadline(id, null);
    });
  };
  const onSetDate = () => {
    props.onSetDeadline?.(targetIds(), props.item().deadline ?? null);
  };
  // Planned-date actions, same target-set rule. Today / Tomorrow keep
  // this row's time part, if any, so a timed item moves days intact.
  const keepTime = (day: string) => whenFromParts(day, whenTime(props.item().when ?? ""));
  const onWhenToday = () => {
    const stamp = keepTime(todayStamp(nowMs()));
    props.app.withActionBatch(() => {
      for (const id of targetIds()) props.app.setItemWhen(id, stamp);
    });
  };
  const onWhenTomorrow = () => {
    const stamp = keepTime(addDaysToStamp(todayStamp(nowMs()), 1));
    props.app.withActionBatch(() => {
      for (const id of targetIds()) props.app.setItemWhen(id, stamp);
    });
  };
  const onWhenRemove = () => {
    props.app.withActionBatch(() => {
      for (const id of targetIds()) props.app.setItemWhen(id, null);
    });
  };
  const onSetWhenDate = () => {
    props.onSetWhen?.(targetIds(), props.item().when ?? null);
  };
  const onOpenChange = (open: boolean) => {
    // Register the menu in the shared overlay count so the workspace's
    // document-level shortcuts (Enter → open dialog, x → done, …) go inert
    // while it's open — otherwise pressing Enter to pick a menu item also
    // triggers the list's Enter handler behind it.
    setMenuOpen(open);
    if (!open) return;
    const id = props.item().id;
    if (!props.selection.isSelected(id)) {
      props.selection.selectOnly(id);
    }
  };
  return (
    <ContextMenu onOpenChange={onOpenChange}>
      <ContextMenu.Trigger
        class={props.deadlineInFooter ? "row row-card" : "row"}
        data-done={isClosed(props.item()) ? "" : undefined}
        data-cancelled={isCancelled(props.item()) ? "" : undefined}
        data-binned={isBinned(props.item()) ? "" : undefined}
        data-expanded={props.expanded() ? "" : undefined}
        on:dblclick={(e) => {
          // Double-click opens the detail dialog. Listen at the row level
          // (not the text span) so double-clicks in the row's padding count
          // too. Native (non-delegated) + stopPropagation so it runs in the
          // bubble phase and suppresses the dnd listbox's own dblclick
          // (which would otherwise start an inline expansion). Drafts and
          // any already-expanded row keep their inline editor.
          if (props.expanded() || isDraftId(props.item().id)) return;
          e.preventDefault();
          e.stopPropagation();
          props.onOpen?.(props.item().id);
        }}
        on:click={(e) => {
          // Mobile: a plain tap opens the detail dialog. Selection still
          // happens via the dnd's own touch handling; we just add the
          // open. Skip drafts, expanded rows, and taps that land on their
          // own controls (checkbox, links, the open/note buttons).
          if (!props.openOnTap?.()) return;
          if (props.expanded() || isDraftId(props.item().id)) return;
          const t = e.target as HTMLElement | null;
          if (t?.closest("input, a, button")) return;
          props.onOpen?.(props.item().id);
        }}
      >
        <input
          type="checkbox"
          class="task-check"
          tabIndex={-1}
          checked={isClosed(props.item())}
          data-cancelled={isCancelled(props.item()) ? "" : undefined}
          onMouseDown={(e) => {
            // Clicking still focuses the checkbox despite tabIndex=-1, and
            // the lingering focus makes a later Space press re-toggle it.
            // Preventing mousedown's default suppresses the focus move while
            // the click (and toggle) still fire.
            e.preventDefault();
          }}
          onChange={(e) => {
            const id = props.item().id;
            // Un-checking a lingering row in Focus re-pins it (closing
            // dropped its ref); elsewhere it's a plain lifecycle flip. A
            // cancelled row un-checks the same way (closed → Backlog).
            if (!e.currentTarget.checked && props.viewKind === "focus") {
              props.app.undoneIntoFocus([id]);
            } else {
              props.app.setDone(id, e.currentTarget.checked);
            }
          }}
        />
        <Show when={leadingStamp() && rowStamp()}>
          {(ts) => (
            <span
              class="row-timestamp row-timestamp-lead"
              title={formatDateTime(ts(), locale())}
            >
              {formatDoneStamp(ts(), nowMs(), locale())}
            </span>
          )}
        </Show>
        <div class="row-body">
          <span
            ref={textRef}
            class="row-text"
            contentEditable={props.expanded()}
            onClick={(e) => {
              if (!props.expanded()) return;
              // Plain clicks on links open them (a new tab, or an in-app
              // jump for the app's own item / list links); modifier-clicks
              // fall through so the user can still place the caret inside
              // a link to edit it. See `openLinkOnClick`.
              openLinkOnClick(e, textRef);
              e.stopPropagation();
            }}
            onPointerDown={(e) => {
              if (props.expanded()) e.stopPropagation();
            }}
            on:paste={(e) => {
              if (!props.expanded()) return;
              // Strip formatting: paste plain text only, so rich HTML
              // never enters the editor (it'd render styled until the
              // row collapses and we save `textContent`).
              pasteAsPlainText(e);
            }}
            on:input={() => {
              // Browsers (Chrome, Firefox) leave a stray <br> behind when
              // the user deletes the last character of a contenteditable,
              // which defeats the `:empty::before` placeholder. Strip it
              // when the visible text is empty so the placeholder returns.
              if (textRef.textContent === "" && textRef.firstChild) {
                textRef.replaceChildren();
              }
            }}
            on:blur={(e) => {
              // iOS Safari's form-assistant bar (the prev/next/Done strip
              // above the keyboard) blurs the contenteditable on Done
              // without firing a keydown — the row would otherwise stay
              // expanded with the keyboard gone. Treat focus leaving the
              // row entirely as a commit, mirroring the Enter path. Skip
              // when focus is moving to another element inside the same
              // row (e.g. tapping into the notes textarea) so the user can
              // still hop fields without collapsing.
              if (!props.expanded()) return;
              const next = e.relatedTarget as Node | null;
              const row = (e.currentTarget as HTMLElement).closest(".row");
              if (next && row?.contains(next)) return;
              // First Escape: dnd is expanded → collapse. Second Escape:
              // dnd is now collapsed → clears selection. Without the
              // second one the row's pre-expand selection chrome flashes
              // visible on desktop until the next click selects another
              // row on mouseup; the dnd bails on mousedown while expanded
              // so selection only updates on the click. The Enter path
              // doesn't hit this because there's no follow-up click.
              const target = e.currentTarget as HTMLElement;
              target.dispatchEvent(
                new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
              );
              target.dispatchEvent(
                new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
              );
            }}
            on:keydown={(e) => {
              // Native (non-delegated) so the bubble order is span → dnd
              // listbox; Solid's delegated `onKeyDown` fires at document
              // level *after* the listbox sees the event, so a delegated
              // stopPropagation here would be too late (Cmd+A would still
              // hit the dnd's select-all).
              if (!props.expanded()) return;
              if (e.key === "Enter" && !e.shiftKey) {
                // Suppress the newline; bounce off the dnd's Escape
                // handler (bound on its listbox) to drive collapse, which
                // triggers the save effect above. Stop propagation so the
                // workspace's document-level Enter handler doesn't see the
                // original event after the synchronous Escape dispatch has
                // already flipped contentEditable off on this span — the
                // editable-surface guard there would no longer match and
                // it would re-expand the row.
                e.preventDefault();
                e.stopPropagation();
                // Mark this as the Enter-commit path so the collapse
                // effect tells the workspace to chain another draft.
                // Escape and blur reach the same collapse without setting
                // this flag, so they end the chain.
                chainOnSettle = true;
                (e.currentTarget as HTMLElement).dispatchEvent(
                  new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
                );
                return;
              }
              // Don't let the dnd intercept keys the contenteditable owns.
              if (e.key !== "Escape") e.stopPropagation();
            }}
          />
          {/* Has-notes badge: a corner-fold glyph marking that the item
              carries notes. Sits inline right after the
              title — the text is the shrinking flex child of .row-body,
              so the badge hugs the last word of a short title and sits
              just past the ellipsis of a truncated one. List rows only:
              a card title that wraps (without clamping) is full width,
              which would strand the badge at the card's edge, so cards
              keep it in the footer below instead. */}
          <Show when={!props.deadlineInFooter && !props.expanded() && hasNotes()}>
            <NotesBadge />
          </Show>
        </div>
        {/* Board cards gather every badge on a full-width bottom line —
            a direct child of the card so the badges start at the card's
            left padding edge (under the checkbox), not the title. List
            rows show them inline after the title instead. Once an item
            leaves Open its deadline stops mattering — done/binned cards
            show only their lifecycle timestamp. */}
        <Show
          when={
            props.deadlineInFooter &&
            !props.expanded() &&
            ((isOpen(props.item()) && (props.item().when || props.item().deadline)) ||
              (canPinToFocus() && focused()) ||
              hasNotes() ||
              rowStamp())
          }
        >
          <div class="row-footer">
            <Show when={hasNotes()}>
              <NotesBadge />
            </Show>
            <Show when={canPinToFocus() && focused()}>
              <span
                class="badge row-focus-badge"
                title={m().focus.badge}
                aria-label={m().focus.badge}
                innerHTML={drawingPinFilledSvg}
              />
            </Show>
            <Show when={isOpen(props.item()) && props.item().when}>
              {(w) => <WhenBadge when={w()} />}
            </Show>
            <Show when={isOpen(props.item()) && props.item().deadline}>
              {(d) => <DeadlineBadge deadline={d()} />}
            </Show>
            <Show when={rowStamp()}>
              {(ts) => (
                <span
                  class="badge row-timestamp"
                  title={formatDateTime(ts(), locale())}
                >
                  <Show when={props.viewKind === "done"}>
                    <span
                      class="row-timestamp-icon"
                      innerHTML={isCancelled(props.item()) ? crossSvg : checkSvg}
                    />
                  </Show>
                  {props.viewKind === "done"
                    ? formatDoneStamp(ts(), nowMs(), locale())
                    : formatRelative(ts(), nowMs(), locale())}
                </span>
              )}
            </Show>
          </div>
        </Show>
        {/* List rows: inline badges after the title, grouped in their own
            flex container so they sit 4px apart while the row's own 8px
            gap still separates the group from the title. */}
        <Show when={showInlineBadges()}>
          <div class="row-badges">
            {/* Focus-membership badge — a static, non-interactive pin glyph
                shown only when the item is pinned to Focus; nothing otherwise.
                There is no hover toggle: adding / removing goes through the row
                context menu (spec/focus.md). */}
            <Show
              when={
                !props.deadlineInFooter && canPinToFocus() && focused() && !props.expanded()
              }
            >
              <span
                class="badge row-focus-badge"
                title={m().focus.badge}
                aria-label={m().focus.badge}
                innerHTML={drawingPinFilledSvg}
              />
            </Show>
            <Show
              when={
                !props.deadlineInFooter &&
                !props.expanded() &&
                isOpen(props.item()) &&
                props.item().when
              }
            >
              {(w) => <WhenBadge when={w()} />}
            </Show>
            <Show
              when={
                !props.deadlineInFooter &&
                !props.expanded() &&
                isOpen(props.item()) &&
                props.item().deadline
              }
            >
              {(d) => <DeadlineBadge deadline={d()} />}
            </Show>
            <Show when={stateBadge()}>
              {(label) => <span class="badge row-state">{label()}</span>}
            </Show>
            <Show when={originList()}>
              {(name) => (
                <span class="badge row-list" title={name()}>
                  <span class="row-list-name">{name()}</span>
                </span>
              )}
            </Show>
            {/* Trailing stamp: Bin rows only (the Done view leads with
                its stamp next to the checkbox, board cards use the footer). */}
            <Show when={!props.deadlineInFooter && !leadingStamp() && rowStamp()}>
              {(ts) => (
                <span class="badge row-timestamp" title={formatDateTime(ts(), locale())}>
                  {formatRelative(ts(), nowMs(), locale())}
                </span>
              )}
            </Show>
          </div>
        </Show>
      </ContextMenu.Trigger>
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
          <Show when={canPinToFocus()}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={onToggleFocusSelection}
            >
              {focused() ? m().focus.remove : m().focus.add}
              <kbd class="menu-shortcut">F</kbd>
            </ContextMenu.Item>
          </Show>
          <Show when={inFocusView()}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={onRemoveFromFocusSelection}
            >
              {m().focus.remove}
              <kbd class="menu-shortcut">F</kbd>
            </ContextMenu.Item>
          </Show>
          {/* Cross-appearance jump. Only for items that exist on both
              sides: from the Focus lens to the item's home list, and from
              a list / board row that currently carries a Focus ref. */}
          <Show when={props.onReveal && inFocusView()}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onReveal?.(props.item().id, "list")}
            >
              <span>
                {m().focus.showInList(
                  props.listLabel?.(props.item().listId) ?? props.item().listId,
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
          {/* Dates only matter while the item is open (the badges hide
              for done/binned rows too, see above). When leads Deadline,
              as the badges do. */}
          <Show when={isOpen(props.item())}>
            <ContextMenu.Sub gutter={4}>
              <ContextMenu.SubTrigger class="context-menu-item">
                <span>{m().when.label}</span>
                <span class="menu-sub-arrow" aria-hidden="true">
                  ›
                </span>
              </ContextMenu.SubTrigger>
              <ContextMenu.Portal>
                <ContextMenu.SubContent class="context-menu-content">
                  <Show when={props.item().when}>
                    <ContextMenu.Item
                      class="context-menu-item"
                      onSelect={onWhenRemove}
                    >
                      <span>{m().when.remove}</span>
                    </ContextMenu.Item>
                  </Show>
                  <Show when={props.onSetWhen}>
                    <ContextMenu.Item
                      class="context-menu-item"
                      onSelect={onSetWhenDate}
                    >
                      <span>{m().when.setDate}</span>
                    </ContextMenu.Item>
                  </Show>
                  <ContextMenu.Item
                    class="context-menu-item"
                    onSelect={onWhenToday}
                  >
                    <span>{m().when.today}</span>
                  </ContextMenu.Item>
                  <ContextMenu.Item
                    class="context-menu-item"
                    onSelect={onWhenTomorrow}
                  >
                    <span>{m().when.tomorrow}</span>
                  </ContextMenu.Item>
                </ContextMenu.SubContent>
              </ContextMenu.Portal>
            </ContextMenu.Sub>
          </Show>
          <Show when={isOpen(props.item())}>
            <ContextMenu.Sub gutter={4}>
              <ContextMenu.SubTrigger class="context-menu-item">
                <span>{m().deadline.label}</span>
                <span class="menu-sub-arrow" aria-hidden="true">
                  ›
                </span>
              </ContextMenu.SubTrigger>
              <ContextMenu.Portal>
                <ContextMenu.SubContent class="context-menu-content">
                  <Show when={props.item().deadline}>
                    <ContextMenu.Item
                      class="context-menu-item"
                      onSelect={onDeadlineRemove}
                    >
                      <span>{m().deadline.remove}</span>
                    </ContextMenu.Item>
                  </Show>
                  <Show when={props.onSetDeadline}>
                    <ContextMenu.Item
                      class="context-menu-item"
                      onSelect={onSetDate}
                    >
                      <span>{m().deadline.setDate}</span>
                    </ContextMenu.Item>
                  </Show>
                  <ContextMenu.Item
                    class="context-menu-item"
                    onSelect={onDeadlineToday}
                  >
                    <span>{m().deadline.today}</span>
                  </ContextMenu.Item>
                  <ContextMenu.Item
                    class="context-menu-item"
                    onSelect={onDeadlineTomorrow}
                  >
                    <span>{m().deadline.tomorrow}</span>
                  </ContextMenu.Item>
                </ContextMenu.SubContent>
              </ContextMenu.Portal>
            </ContextMenu.Sub>
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
          <ContextMenu.Item class="context-menu-item" onSelect={onCopyLink}>
            <span>{m().common.copyLink}</span>
          </ContextMenu.Item>
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
