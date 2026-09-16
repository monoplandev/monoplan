// The "open task" detail surface: a centered dialog (full-screen sheet on
// mobile, or an inline pane when the desktop side panel is showing) driven
// purely by an item id, so the same component can later back a native
// detached window. Notes live here only — the inline row editor is
// text-only "quick entry". The title is buffered locally and flushed to
// the engine on close and before stepping to a neighbour (last write on
// close wins). Notes stream as deltas both ways (spec/notes-plan.md Phase
// 2): every input event sends the editor's change to the core, and a live
// peer edit arrives as a delta that is applied under the caret instead of
// replacing the field.

import { Dialog } from "@kobalte/core/dialog";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Match,
  onCleanup,
  Show,
  Switch,
  untrack,
} from "solid-js";
import { Portal } from "solid-js/web";
import { DeadlineField } from "./DeadlineField.tsx";
import { WhenField } from "./WhenField.tsx";
import { ListPicker, type ListOption } from "./ListPicker.tsx";
import caretSortSvg from "./icons/caret-sort.svg?raw";
import checkSvg from "./icons/check.svg?raw";
import dotsHorizontalSvg from "./icons/dots-horizontal.svg?raw";
import drawingPinSvg from "./icons/drawing-pin.svg?raw";
import drawingPinFilledSvg from "./icons/drawing-pin-filled.svg?raw";
import sidebarRightSvg from "./icons/sidebar-right.svg?raw";
import { formatDialogStamp, nowMs } from "./format.tsx";
import { useAppI18n, laneLabel } from "./i18n.tsx";
import {
  collapsedCaretOffset,
  openLinkOnClick,
  placeCaretAtEnd,
  placeCaretAtOffset,
  placeCaretAtStart,
  setLinkifiedText,
} from "./linkify.ts";
import {
  applyDelta,
  diffToDelta,
  transformOffset,
  type NotesDeltaOp,
} from "./notesDelta.ts";
import { pasteAsPlainText } from "./plainTextPaste.ts";
import { itemUrl } from "./url.ts";
import { trackOverlay } from "./overlay.ts";
import {
  isBinned,
  isDone,
  OPEN_STATES,
  type DocApp,
  type ItemView,
  type ListView,
  type WorkflowState,
} from "./sync/store.ts";

export function TaskDialog(props: {
  /** The open item's id, or null when closed. */
  itemId: () => string | null;
  setItemId: (id: string | null) => void;
  /** New-item mode: a target open lane to capture into (`backlog` is the
   *  list view's default). Mutually exclusive with `itemId`; nothing is
   *  written until a non-empty title is committed on close. `index`,
   *  when set, inserts at that position in the list's Open projection
   *  (Space capture below a board card); omitted appends. */
  newItem?: () => {
    listId: string;
    state: WorkflowState;
    index?: number;
    /** Log an already-completed item: created open, marked done on commit. */
    done?: boolean;
  } | null;
  setNewItem?: (
    v: {
      listId: string;
      state: WorkflowState;
      index?: number;
      done?: boolean;
    } | null,
  ) => void;
  app: DocApp;
  /** Active (non-archived) user lists — the move/capture destinations. */
  lists: () => ListView[];
  /** True when the user has entered the shown item (Enter on a row, a
   *  row's open control, a Find pick, a link, a click into the pane):
   *  the non-modal shells land the caret. False while the side pane is
   *  merely following the list selection, which leaves focus on the list.
   *  Notifies (without changing) on a re-entry of the same item, which
   *  re-lands the caret. Treated as true when omitted, and ignored for a
   *  new-item capture, which always lands the caret. */
  entered?: () => boolean;
  /** Called as the dialog closes so the owner can restore focus (to the
   *  list). Fires from Kobalte's close-auto-focus hook, which we take over
   *  to steer focus back to the listbox instead of the trigger. */
  onClosed?: () => void;
  /** Side-panel shell only: Enter in the title, Escape, or ⌘Enter hands
   *  keyboard focus back to the list/board. The owner focuses its items
   *  listbox and un-enters the item, which the pane keeps showing as the
   *  followed row; the inverse of `onFocused`. */
  onReleaseFocus?: () => void;
  /** Side-panel shell only: focus entered the pane (a click into it)
   *  while it was following the selection. The owner enters the shown
   *  item, which is what the address bar follows (`spec/urls.md`). */
  onFocused?: () => void;
  /** Pushes the in-progress title into a UI-only channel so the list row
   *  mirrors the edit live — without a sync op per keystroke. The real
   *  write still happens once, via the close/flush path. */
  onLiveText?: (text: string) => void;
  /** Fires with the id of a freshly committed new item, so the caller can
   *  select/scroll to it (used by the board's "+" capture). */
  onCreated?: (id: string) => void;
  /** Desktop side panel host. When it resolves to an element the surface
   *  renders inline there (non-modal: the list stays live, clicking another
   *  row swaps the open item in place) instead of as the centred dialog.
   *  Ignored on mobile, which keeps its page shell. */
  panelMount?: () => HTMLElement | null;
  /** Desktop only: move the surface between its modal and side-panel
   *  shells, keeping the open item (unlike the app menu's toggle, which
   *  closes it). Renders the header's sidebar button when given. */
  onSwapShell?: () => void;
  /** Jump to the view that shows this item (the Workspace reveal path:
   *  switch view, select + scroll the row). The dialog closes so the
   *  revealed row isn't hidden behind the modal. */
  onReveal?: (id: string, where: "list" | "focus") => void;
}) {
  const { m, locale } = useAppI18n();

  // Shell selection. Mobile: a plain full-screen page under the floating
  // pills. Desktop with the side panel showing: an inline pane portaled
  // into it. Otherwise: the centred modal Dialog.
  const isMobileMq = window.matchMedia("(max-width: 768px) and (pointer: coarse)");
  const [isMobile, setIsMobile] = createSignal(isMobileMq.matches);
  const onMq = (e: MediaQueryListEvent) => setIsMobile(e.matches);
  isMobileMq.addEventListener("change", onMq);
  onCleanup(() => isMobileMq.removeEventListener("change", onMq));
  const panelHost = createMemo(() =>
    isMobile() ? null : (props.panelMount?.() ?? null),
  );
  const panelMode = () => panelHost() !== null;

  const newItemTarget = createMemo(() => props.newItem?.() ?? null);
  const isNew = createMemo(
    () => props.itemId() === null && newItemTarget() !== null,
  );
  const open = createMemo(
    () => props.itemId() !== null || newItemTarget() !== null,
  );
  // The modal dialog suppresses the workspace's global shortcuts while
  // open; the inline pane is non-modal by design, so the list behind it
  // keeps its keys (the editable-surface guard still covers typing here).
  trackOverlay(() => open() && !panelMode());

  const item = createMemo<ItemView | undefined>(() => {
    const id = props.itemId();
    return id ? props.app.state.itemsById[id] : undefined;
  });

  // Whether the open item currently has a visible Focus ref (spec/focus.md).
  const focused = createMemo(() => {
    const id = props.itemId();
    return id ? props.app.state.focusOrder.includes(id) : false;
  });
  const toggleFocus = () => {
    const id = props.itemId();
    if (!id) return;
    if (focused()) props.app.removeFromFocus(id);
    else props.app.addToFocus(id);
  };

  // If the open item vanishes (deleted here or by a peer), close.
  createEffect(() => {
    if (props.itemId() !== null && !item()) props.setItemId(null);
  });

  // A new capture takes over from an open item. Only reachable through the
  // non-modal shells (the modal blocks the Add buttons); the load effect
  // below flushes the item's edits before the capture form loads.
  createEffect(() => {
    if (newItemTarget() && untrack(() => props.itemId()) !== null) {
      props.setItemId(null);
    }
  });

  // Move-to-list options: Inbox followed by every *active* user list —
  // archived lists are not offered as destinations. If the open item's
  // home list is itself archived, it is appended so the picker still
  // renders the current list's name (and moving *out* to an active list
  // remains possible).
  const listOptions = createMemo<ListOption[]>(() => {
    const opts: ListOption[] = [
      { id: "inbox", name: m().nav.inbox },
      ...props.lists().map((l) => ({ id: l.id, name: l.name, icon: l.icon })),
    ];
    const currentId = item()?.listId;
    if (currentId && !opts.some((o) => o.id === currentId)) {
      const current = props.app.state.listsById[currentId];
      if (current) {
        opts.push({ id: current.id, name: current.name, icon: current.icon });
      }
    }
    return opts;
  });
  const moveItemToList = (targetId: string, currentListId: string) => {
    const id = props.itemId();
    if (!id || targetId === currentListId) return;
    const idx = props.app.state.listOpen[targetId]?.length ?? 0;
    props.app.moveItem(id, targetId, idx);
  };

  // The list option a new item is currently targeting (drives the header
  // picker's selected value).
  const newItemListOption = createMemo<ListOption | null>(() => {
    const nw = newItemTarget();
    if (!nw) return null;
    return listOptions().find((o) => o.id === nw.listId) ?? null;
  });
  // Re-target a new-item capture at a different list. The insert index is
  // dropped — a position in the old list's Open projection is meaningless in
  // the new one, so the item appends.
  const setNewItemList = (targetId: string) => {
    const nw = newItemTarget();
    if (!nw || targetId === nw.listId) return;
    props.setNewItem?.({ listId: targetId, state: nw.state, done: nw.done });
  };
  // Toggle whether a new capture is logged as already-done.
  const setNewItemDone = (done: boolean) => {
    const nw = newItemTarget();
    if (!nw) return;
    props.setNewItem?.({ ...nw, done });
  };
  // Re-target a new capture's lifecycle from the status badge. Done routes
  // through the `done` flag (create open, mark done on commit — same as the
  // header checkbox); an open state re-targets the lane and clears it.
  const setNewItemState = (state: WorkflowState) => {
    const nw = newItemTarget();
    if (!nw) return;
    if (state === "done") props.setNewItem?.({ ...nw, done: true });
    else props.setNewItem?.({ ...nw, state, done: false });
  };

  const [text, setText] = createSignal("");
  const [notes, setNotes] = createSignal("");
  // New-item mode's deadline buffer: nothing exists to write to until the
  // capture commits, so picks are held here and applied after creation.
  const [newDeadline, setNewDeadline] = createSignal<string | null>(null);
  // New-item mode's planned-date buffer, same deal.
  const [newWhen, setNewWhen] = createSignal<string | null>(null);
  // New-item mode's duration buffer, applied after `newWhen`.
  const [newDuration, setNewDuration] = createSignal<number | null>(null);
  // New-item mode's pin-to-Focus buffer, same deal as `newDeadline`.
  const [newFocus, setNewFocus] = createSignal(false);
  // Deadline calendar popover open state, shared by both DeadlineField modes.
  const [deadlineCalOpen, setDeadlineCalOpen] = createSignal(false);
  const [whenCalOpen, setWhenCalOpen] = createSignal(false);
  // The title and notes editors are contenteditable (not textareas) so that
  // http(s) URLs render as clickable anchors, matching the row quick-entry
  // editor. Their content is set imperatively from the buffers on load — it
  // is never value-bound, so reactive updates can't clobber a live caret.
  let titleRef: HTMLDivElement | undefined;
  let notesRef: HTMLDivElement | undefined;

  // Which id the buffers currently hold. A plain var (not a signal): it's
  // written from the load effect, never read reactively.
  let loadedId: string | null = null;

  // Notes delta bridge. `synced` is the notes text the core holds for the
  // subscribed item (what the editor last sent or received); `notes()`
  // tracks the editor DOM. Their diff is the pending local delta.
  let subscribedId: string | null = null;
  let synced = "";
  // IME state: nothing crosses the boundary mid-composition (a lone
  // surrogate would be re-encoded on the way in). `compositionBase` is
  // the synced text when the composition began; inbound deltas that
  // land during it are applied to `synced` and remembered so the
  // composed text can be re-placed on top of them at the end.
  let composing = false;
  let compositionBase = "";
  let inboundDuringComposition: NotesDeltaOp[] | null = null;

  const unsubscribeNotes = () => {
    if (!subscribedId) return;
    props.app.unsubscribeNotes(subscribedId);
    subscribedId = null;
  };
  // Subscribe the editor to `id`; returns the core's current notes text.
  // A missing item (deleted under us) falls back to the store's copy and
  // leaves the editor unsubscribed, so its writes go the whole-string way.
  const subscribeNotes = (id: string, fallback: string): string => {
    unsubscribeNotes();
    try {
      synced = props.app.subscribeNotes(id);
      subscribedId = id;
    } catch (e) {
      // Loud on purpose: unsubscribed, the editor silently degrades to
      // one whole-string write on close, which looks like lost typing.
      console.error("subscribeNotes failed; notes will save on close only:", e);
      synced = fallback;
    }
    composing = false;
    inboundDuringComposition = null;
    return synced;
  };
  // Send the editor's pending local edit as a delta. Skipped mid-IME.
  const commitLocalEdit = () => {
    if (!subscribedId || composing) return;
    const next = notes();
    const ops = diffToDelta(synced, next);
    if (!ops) return;
    try {
      props.app.applyNotesDelta(subscribedId, ops);
      synced = next;
    } catch (e) {
      console.error("applyNotesDelta failed:", e);
      resyncNotesFromCore();
    }
  };
  // Reload the editor from the core (after a full resync, or a delta
  // that did not fit), keeping the caret where it was if it is in the
  // notes.
  const resyncNotesFromCore = () => {
    if (!subscribedId) return;
    const id = subscribedId;
    const caret = notesRef ? collapsedCaretOffset(notesRef) : null;
    const text = subscribeNotes(id, props.app.state.itemsById[id]?.notes ?? "");
    setNotes(text);
    loadEditor(notesRef, text);
    if (caret !== null && notesRef && document.activeElement === notesRef) {
      placeCaretAtOffset(notesRef, Math.min(caret, text.length));
    }
  };
  // Apply an inbound delta to the buffer and the DOM, moving the caret
  // with it. The DOM is re-rendered wholesale (notes are short and the
  // linkifier owns the markup); the caret survives via its offset.
  const applyInboundToEditor = (ops: readonly NotesDeltaOp[]) => {
    const caret = notesRef ? collapsedCaretOffset(notesRef) : null;
    const next = applyDelta(synced, ops);
    synced = next;
    setNotes(next);
    loadEditor(notesRef, next);
    if (caret !== null && notesRef && document.activeElement === notesRef) {
      placeCaretAtOffset(notesRef, transformOffset(caret, ops));
    }
  };
  const offNotesDelta = props.app.onNotesDelta((id, ops) => {
    if (!subscribedId) return;
    if (ops === null) {
      resyncNotesFromCore();
      return;
    }
    if (id !== subscribedId) return;
    if (composing) {
      // Keep `synced` truthful; the DOM catches up at compositionend.
      synced = applyDelta(synced, ops);
      inboundDuringComposition = inboundDuringComposition
        ? [...inboundDuringComposition, ...ops]
        : [...ops];
      return;
    }
    // Fold any unsent local edit in first so the inbound delta lands on
    // the text the core converted it against.
    commitLocalEdit();
    applyInboundToEditor(ops);
  });
  onCleanup(offNotesDelta);
  // Every editor change refreshes the buffer and sends the pending
  // delta. Both forms (new-item capture, existing-item edit) render
  // their own notes editor; they share these handlers.
  const onNotesInput = () => {
    setNotes(editorText(notesRef));
    commitLocalEdit();
  };
  const onNotesCompositionStart = () => {
    if (!subscribedId) return;
    commitLocalEdit();
    composing = true;
    compositionBase = synced;
    inboundDuringComposition = null;
  };
  const onNotesCompositionEnd = () => {
    if (!composing) return;
    composing = false;
    setNotes(editorText(notesRef));
    const inbound = inboundDuringComposition;
    inboundDuringComposition = null;
    if (!inbound || !subscribedId) {
      commitLocalEdit();
      return;
    }
    // Remote text arrived mid-composition. Re-place the composed edit on
    // top of the new text: shift its position through the inbound
    // delta, and keep its deletion only if nothing remote touched that
    // range (otherwise the remote text wins and the composed text is
    // inserted beside it).
    const local = diffToDelta(compositionBase, notes());
    if (!local) {
      applyInboundToEditor([]);
      return;
    }
    let retain = 0;
    let del = 0;
    let insert = "";
    for (const op of local) {
      if ("retain" in op) retain = op.retain;
      else if ("delete" in op) del = op.delete;
      else insert = op.insert;
    }
    const start = transformOffset(retain, inbound);
    const end = transformOffset(retain + del, inbound);
    const safeDelete = end - start === del ? del : 0;
    const ops: NotesDeltaOp[] = [];
    if (start > 0) ops.push({ retain: start });
    if (safeDelete > 0) ops.push({ delete: safeDelete });
    if (insert) ops.push({ insert });
    try {
      if (ops.length > 0) props.app.applyNotesDelta(subscribedId, ops);
      synced = applyDelta(synced, ops);
      setNotes(synced);
      loadEditor(notesRef, synced);
      if (notesRef && document.activeElement === notesRef) {
        placeCaretAtOffset(notesRef, start + insert.length);
      }
    } catch (e) {
      console.error("applyNotesDelta failed:", e);
      resyncNotesFromCore();
    }
  };
  // Durable capture + push of a typing burst happens on an idle timer;
  // leaving the editor, hiding the page, or unloading runs it now.
  const flushNotesNow = () => {
    commitLocalEdit();
    props.app.flushNotes();
  };
  const onVisibility = () => {
    if (document.visibilityState === "hidden") flushNotesNow();
  };
  document.addEventListener("visibilitychange", onVisibility);
  window.addEventListener("pagehide", flushNotesNow);
  onCleanup(() => {
    document.removeEventListener("visibilitychange", onVisibility);
    window.removeEventListener("pagehide", flushNotesNow);
  });

  // Read the plain text out of a contenteditable editor, stripping the stray
  // <br> browsers leave behind when the last character is deleted so the
  // :empty placeholder returns.
  const editorText = (el?: HTMLDivElement): string => {
    if (!el) return "";
    if (el.textContent === "" && el.firstChild) el.replaceChildren();
    return el.textContent ?? "";
  };

  // Push a buffer value into a contenteditable editor as linkified content.
  const loadEditor = (el: HTMLDivElement | undefined, value: string) => {
    if (!el) return;
    setLinkifiedText(el, value);
    ensureTrailingBreak(el);
  };

  // A "\n" at the very end of a pre-wrap block doesn't get its own line
  // box, so the caret has nowhere to land after a trailing newline. A
  // trailing <br> gives it one; textContent ignores <br>, so the saved
  // string is unaffected.
  const ensureTrailingBreak = (el: HTMLDivElement) => {
    if (!(el.textContent ?? "").endsWith("\n")) return;
    if (el.lastChild instanceof HTMLBRElement) return;
    el.appendChild(document.createElement("br"));
  };

  // Replace the selection inside `el` with a literal "\n" text node. Done by
  // hand rather than execCommand("insertText", "\n") because WebKit (every
  // iOS browser) treats that as a block split or drops it, so the newline
  // never reaches textContent and never saves. Returns false when the
  // selection isn't inside `el`.
  const insertNewline = (el: HTMLDivElement | undefined): boolean => {
    if (!el) return false;
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return false;
    const range = sel.getRangeAt(0);
    if (!el.contains(range.commonAncestorContainer)) return false;
    range.deleteContents();
    const nl = document.createTextNode("\n");
    range.insertNode(nl);
    range.setStartAfter(nl);
    range.collapse(true);
    sel.removeAllRanges();
    sel.addRange(range);
    ensureTrailingBreak(el);
    setNotes(editorText(el));
    commitLocalEdit();
    return true;
  };

  // Write the buffered editor contents back to `id` if they differ. Empty
  // title is ignored (keep the existing text), mirroring the inline editor.
  // Reads the `text` / `notes` buffers (kept in step with the editors by
  // their input handlers) rather than the DOM: by the time a target switch
  // reaches the load effect below, the editors may already show the next
  // target — or, across the new-item / edit forms, be different elements.
  const flush = (id: string | null) => {
    if (!id) return;
    const it = props.app.state.itemsById[id];
    if (!it) return;
    const t = text().trim();
    if (t && t !== it.text) props.app.editItemText(id, t);
    const n = notes();
    if (subscribedId === id) {
      // Streaming: the core already holds every sent edit. Send what is
      // still pending, fall back to a whole-string write for anything a
      // composition left unsent, then make the burst durable.
      commitLocalEdit();
      if (n !== synced) {
        props.app.editItemNotes(id, n);
        synced = n;
      }
      props.app.flushNotes();
    } else if (n !== it.notes) {
      props.app.editItemNotes(id, n);
    }
  };

  // Settle whatever the buffers currently hold: commit a pending capture,
  // or write an open item's edits back. Idempotent — a second pass finds
  // nothing changed (or, for a capture already committed, no target).
  const settle = () => {
    if (loadedId === "new") commitNew();
    else flush(loadedId);
  };

  // Load the buffers and the editor DOM when the open target changes (an
  // item, a fresh new-item capture, or nothing). Content is set
  // imperatively — never value-bound — so reactive re-renders can't
  // clobber a live caret. The outgoing target is settled first: the modal
  // shells only ever leave via `close()` (which already flushed), but the
  // inline pane swaps targets directly when another row is clicked.
  createEffect(() => {
    const id = props.itemId();
    const nw = newItemTarget();
    const key = id ?? (nw ? "new" : null);
    if (key === loadedId) return;
    untrack(settle);
    unsubscribeNotes();
    loadedId = key;
    // Closed: the editors unmount; forgetting the target means reopening
    // the same item re-pushes its content into fresh editors.
    if (key === null) return;
    const it = id ? props.app.state.itemsById[id] : undefined;
    const t = it?.text ?? "";
    // The core's text is the truth for an open item (the store's copy
    // is the same string one drain behind); a capture has none yet.
    const n = id ? subscribeNotes(id, it?.notes ?? "") : "";
    setText(t);
    setNotes(n);
    setNewDeadline(null);
    setNewWhen(null);
    setNewFocus(false);
    // The editors mount when the surface opens; defer so their refs exist,
    // then push — but only if this target is still the one showing.
    queueMicrotask(() => {
      const curId = props.itemId();
      const curKey = curId ?? (props.newItem?.() ? "new" : null);
      if (curKey !== key) return;
      loadEditor(titleRef, t);
      loadEditor(notesRef, n);
    });
  });

  // Unmounting mid-edit (the shell swapping, the workspace tearing down)
  // must not drop buffered edits.
  onCleanup(() => {
    settle();
    unsubscribeNotes();
  });

  // Commit new-item mode: create the item in its target lane's workflow
  // state iff the title is non-empty, then close. A capture without an
  // explicit slot lands at the TOP of the lane (index 0) — matching the
  // list view's inline-draft default — rather than appending.
  const commitNew = () => {
    const nw = newItemTarget();
    if (nw) {
      const t = text().trim();
      if (t) {
        const at = nw.index ?? 0;
        const id =
          nw.state !== "backlog"
            ? props.app.addItemInStateAt(nw.listId, t, nw.state, at)
            : props.app.addItemAt(nw.listId, t, at);
        const n = notes();
        if (n.trim()) props.app.editItemNotes(id, n);
        const d = newDeadline();
        if (d) props.app.setItemDeadline(id, d);
        const w = newWhen();
        if (w) props.app.setItemWhen(id, w);
        const dur = newDuration();
        if (w && dur) props.app.setItemDuration(id, dur);
        // A Done capture can't hold a Focus ref (auto-remove-on-Done,
        // spec/focus.md), so the pin buffer only applies to open captures.
        if (newFocus() && !nw.done) props.app.addToFocus(id);
        // Logged-as-done capture: create open, then mark done in a second op
        // (mirrors a drag-into-Done). Stamps doneAt = now.
        if (nw.done) props.app.setDone(id, true);
        props.onCreated?.(id);
      }
    }
    props.setNewItem?.(null);
  };

  const close = () => {
    if (isNew()) {
      commitNew();
      return;
    }
    flush(loadedId);
    props.setItemId(null);
  };

  // Side pane capture: the pane is non-modal, so there's no overlay to
  // catch a click-away. Pointing anywhere else in the app (a row, the
  // nav, the board) commits the capture like Enter / Escape do, rather
  // than leaving a half-typed item stranded in the pane. Portaled layers
  // (the lifecycle / deadline menus, the list picker) live outside the
  // app root, so opening or clicking inside them doesn't count as leaving.
  let shellRef: HTMLElement | undefined;
  createEffect(() => {
    if (!panelMode() || !isNew()) return;
    const root = document.getElementById("root");
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node | null;
      if (!target || !root?.contains(target)) return;
      if (shellRef?.contains(target)) return;
      commitNew();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    onCleanup(() =>
      document.removeEventListener("pointerdown", onPointerDown, true),
    );
  });

  // Title/notes keyboard nav, shared by the edit and new-item forms. The
  // editors are contenteditable, so "caret at start/end" is derived from
  // the collapsed selection offset rather than textarea selection props.
  const onTitleKeyDown = (e: KeyboardEvent) => {
    // Enter commits (the title is one line); Shift+Enter is left to the
    // browser, but the title never wraps to multiple lines in use. The
    // modal / mobile shells close on commit. The side pane stays open on
    // the item and just returns focus to the list/board — the pane is
    // ambient, so "done editing" shouldn't blank it. A capture in the
    // pane still commits via close() (there's no item to stay on yet).
    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      !e.metaKey &&
      !e.ctrlKey &&
      !e.altKey &&
      !e.isComposing
    ) {
      e.preventDefault();
      if (panelMode() && !isNew()) {
        flush(loadedId);
        props.onReleaseFocus?.();
        return;
      }
      close();
      return;
    }
    // ArrowDown at the very end of the title drops into the notes field.
    if (
      e.key !== "ArrowDown" ||
      e.shiftKey ||
      e.altKey ||
      e.metaKey ||
      e.ctrlKey ||
      e.isComposing ||
      !titleRef ||
      !notesRef
    )
      return;
    const off = collapsedCaretOffset(titleRef);
    if (off === null || off !== (titleRef.textContent?.length ?? 0)) return;
    e.preventDefault();
    placeCaretAtStart(notesRef);
  };
  const onNotesKeyDown = (e: KeyboardEvent) => {
    // Plain Enter inserts a real newline character (kept as text so
    // textContent round-trips on save), instead of the default block split.
    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      !e.metaKey &&
      !e.ctrlKey &&
      !e.altKey &&
      !e.isComposing
    ) {
      e.preventDefault();
      insertNewline(notesRef);
      return;
    }
    // ArrowUp at the very start of the notes jumps back up to the title.
    if (
      e.key !== "ArrowUp" ||
      e.shiftKey ||
      e.altKey ||
      e.metaKey ||
      e.ctrlKey ||
      e.isComposing ||
      !titleRef ||
      !notesRef
    )
      return;
    const off = collapsedCaretOffset(notesRef);
    if (off === null || off !== 0) return;
    e.preventDefault();
    placeCaretAtEnd(titleRef);
  };

  // iOS soft keyboards can deliver Return without a keydown the handler
  // above sees (or with key "Unidentified"); the editing intent still
  // arrives here as insertParagraph / insertLineBreak. Swap the default
  // block split for the same literal newline.
  const onNotesBeforeInput = (e: InputEvent) => {
    if (
      e.inputType !== "insertParagraph" &&
      e.inputType !== "insertLineBreak"
    )
      return;
    if (e.isComposing) return;
    if (insertNewline(notesRef)) e.preventDefault();
  };

  // Land the caret on open at the end of the title. rAF defers past the
  // load effect that linkifies the title value.
  const focusOnOpen = () => {
    requestAnimationFrame(() => {
      if (titleRef) placeCaretAtEnd(titleRef);
    });
  };

  // Cmd/Ctrl+Enter anywhere in the surface = save & close. The non-modal
  // shells (mobile page, side pane) also take Escape, which Kobalte
  // handles for the dialog. In the side pane Escape on an existing item
  // steps out rather than closing: the edit is flushed and focus returns
  // to the list with the row still selected, so the pane keeps showing
  // it. A second Escape on the list clears the selection, and the
  // workspace closes the pane behind it. A capture in the pane still
  // closes (there's no item to stay on yet), as does the mobile page.
  const onShellKeyDown = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      // The pane can't close out from under a selected row (it would
      // just show it again), so commit-and-close means commit and hand
      // focus back, as Enter in the title does.
      if (panelMode() && !isNew()) {
        flush(loadedId);
        props.onReleaseFocus?.();
        return;
      }
      close();
      return;
    }
    if (e.key === "Escape" && (isMobile() || panelMode())) {
      e.preventDefault();
      e.stopPropagation();
      if (panelMode() && !isNew()) {
        flush(loadedId);
        props.onReleaseFocus?.();
        return;
      }
      close();
    }
  };

  // Header buttons shared by the new-item and edit forms: the shell swap
  // (desktop only) and the close ✕. The panel shell drops the ✕ — the
  // pane is dismissed by leaving the selection (Escape steps out to the
  // list first, and a second Escape clears it), and the swap button
  // stands in the corner instead.
  const shellButtons = () => (
    <>
      <Show when={!isMobile() && props.onSwapShell}>
        <button
          type="button"
          class="icon-button"
          aria-label={
            panelMode() ? m().sidePanel.toModal : m().sidePanel.toPanel
          }
          onClick={() => props.onSwapShell?.()}
          innerHTML={sidebarRightSvg}
        />
      </Show>
      <Show when={!panelMode()}>
        <button
          type="button"
          class="icon-button task-dialog-close"
          aria-label={m().common.close}
          onClick={close}
        >
          ✕
        </button>
      </Show>
    </>
  );

  // The surface body, shared by both shells below.
  const body = () => (
    <>
    <Show when={isNew()}>
      <header class="task-dialog-header">
        <div class="task-dialog-header-meta">
          {/* Checked = this capture is logged as already-done. Pre-set
              by the Done lane "+" and the Done view's "Log" button;
              flip it off to file the item as a normal open task. */}
          <input
            type="checkbox"
            class="task-check"
            checked={newItemTarget()?.done ?? false}
            aria-label={
              newItemTarget()?.done
                ? m().workspace.markNotDone
                : m().workspace.markDone
            }
            onChange={(e) => setNewItemDone(e.currentTarget.checked)}
          />
          {/* Stamp mirroring the edit dialog's created/completed
              text: names the checkbox's current meaning. */}
          <span class="task-dialog-created">
            {newItemTarget()?.done
              ? m().workspace.loggingDoneStamp
              : m().workspace.newItemStamp}
          </span>
        </div>
        <div class="task-dialog-header-actions">{shellButtons()}</div>
      </header>
      <div class="task-dialog-body">
        <div class="task-dialog-content">
          <div
            ref={(el) => {
              titleRef = el;
              // Set the literal attribute value (not Solid's folded
              // valueless `contenteditable`) so the workspace's
              // `[contenteditable="true"]` shortcut guard matches.
              el.setAttribute("contenteditable", "true");
              setLinkifiedText(el, text());
            }}
            class="task-dialog-title"
            role="textbox"
            data-done={newItemTarget()?.done ? "" : undefined}
            data-placeholder={
              newItemTarget()?.done
                ? m().workspace.logCompleted
                : m().board.addItem
            }
            onInput={() => setText(editorText(titleRef))}
            onKeyDown={onTitleKeyDown}
            onPaste={pasteAsPlainText}
            onClick={(e) => openLinkOnClick(e, titleRef)}
          />
          {/* List selector leads the badge row, then the deadline
              badge; picks land in the local buffers and are written
              after the item commits. The pin toggle buffers the
              same way (`newFocus`). */}
          <div class="task-dialog-badges">
            <ListPicker
              options={listOptions}
              value={() => newItemListOption()?.id ?? null}
              onChange={setNewItemList}
            />
            <LifecycleBadge
              value={() => {
                const nw = newItemTarget();
                return nw?.done ? "done" : (nw?.state ?? "backlog");
              }}
              onChange={setNewItemState}
            />
            <DeadlineField
              deadline={newDeadline}
              muted={() => newItemTarget()?.done ?? false}
              onChange={setNewDeadline}
              open={deadlineCalOpen}
              setOpen={setDeadlineCalOpen}
            />
            <Show when={!(newItemTarget()?.done ?? false)}>
              <PinToggle
                pinned={newFocus}
                onToggle={() => setNewFocus((v) => !v)}
              />
            </Show>
          </div>
          {/* Date section: the planned date on its own ruled band
              between the badges and the notes. */}
          <div class="task-dialog-dates">
            <WhenField
              when={newWhen}
              duration={newDuration}
              muted={() => newItemTarget()?.done ?? false}
              onChange={(value) => {
                setNewWhen(value);
                // Mirror the core: no date, no length.
                if (!value) setNewDuration(null);
              }}
              onDurationChange={setNewDuration}
              open={whenCalOpen}
              setOpen={setWhenCalOpen}
            />
          </div>
          <div
            ref={(el) => {
              notesRef = el;
              el.setAttribute("contenteditable", "true");
              setLinkifiedText(el, notes());
            }}
            class="task-dialog-notes"
            role="textbox"
            aria-multiline="true"
            data-placeholder={m().workspace.notes}
            on:input={onNotesInput}
            onBlur={flushNotesNow}
            on:compositionstart={onNotesCompositionStart}
            on:compositionend={onNotesCompositionEnd}
            onKeyDown={onNotesKeyDown}
            on:beforeinput={onNotesBeforeInput}
            onPaste={pasteAsPlainText}
            onClick={(e) => openLinkOnClick(e, notesRef)}
          />
        </div>
      </div>
    </Show>
    <Show when={item()}>
      {(it) => (
        <>
          <header class="task-dialog-header">
            <div class="task-dialog-header-meta">
              <input
                type="checkbox"
                class="task-check"
                checked={isDone(it())}
                onChange={(e) =>
                  props.app.setDone(it().id, e.currentTarget.checked)
                }
              />
              {/* Created stamp, swapping to the completion stamp
                  once the item is ticked off. */}
              <span class="task-dialog-created">
                {isDone(it())
                  ? m().workspace.completedStamp(
                      formatDialogStamp(it().lifecycleAt, nowMs(), locale(), { inline: true }),
                    )
                  : m().workspace.createdStamp(
                      formatDialogStamp(it().createdAt, nowMs(), locale(), { inline: true }),
                    )}
              </span>
            </div>
            <div class="task-dialog-header-actions">
              <DropdownMenu>
                <DropdownMenu.Trigger
                  class="icon-button"
                  aria-label={m().common.menu}
                  innerHTML={dotsHorizontalSvg}
                />
                <DropdownMenu.Portal>
                  <DropdownMenu.Content class="dropdown-menu-content task-dialog-menu-content">
                    <DropdownMenu.Item
                      class="dropdown-menu-item"
                      onSelect={() => {
                        void navigator.clipboard.writeText(itemUrl(it().id));
                      }}
                    >
                      {m().common.copyLink}
                    </DropdownMenu.Item>
                    {/* Home-list jump, mirroring the Focus row's
                        "Show in <list>". Binned items have no list row
                        to land on (the Bin holds them). */}
                    <Show when={props.onReveal && !isBinned(it())}>
                      <DropdownMenu.Item
                        class="dropdown-menu-item"
                        onSelect={() => {
                          props.onReveal?.(it().id, "list");
                          props.setItemId(null);
                        }}
                      >
                        {m().focus.showInList(
                          listOptions().find((o) => o.id === it().listId)?.name ??
                            it().listId,
                        )}
                      </DropdownMenu.Item>
                    </Show>
                    <Show when={!isBinned(it())}>
                      <DropdownMenu.Item
                        class="dropdown-menu-item"
                        onSelect={() => {
                          props.app.setBinnedMany([it().id], true);
                          props.setItemId(null);
                        }}
                      >
                        {m().workspace.moveToBin}
                      </DropdownMenu.Item>
                    </Show>
                    {/* Binned items: the bin's two exits live here rather
                        than as buttons in the body, mirroring the row
                        context menu. Both close the dialog — the item
                        either leaves the bin or stops existing. */}
                    <Show when={isBinned(it())}>
                      <DropdownMenu.Item
                        class="dropdown-menu-item"
                        onSelect={() => {
                          props.app.setBinnedMany([it().id], false);
                          props.setItemId(null);
                        }}
                      >
                        {m().common.restore}
                      </DropdownMenu.Item>
                      <DropdownMenu.Item
                        class="dropdown-menu-item"
                        onSelect={() => {
                          props.app.deleteBinnedMany([it().id]);
                          props.setItemId(null);
                        }}
                      >
                        {m().common.delete}
                      </DropdownMenu.Item>
                    </Show>
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu>
              {shellButtons()}
            </div>
          </header>

          <div class="task-dialog-body">
            <div class="task-dialog-content">
              <div
                ref={(el) => {
                  titleRef = el;
                  el.setAttribute("contenteditable", "true");
                  setLinkifiedText(el, text());
                }}
                class="task-dialog-title"
              role="textbox"
              data-done={isDone(it()) ? "" : undefined}
              onInput={() => {
                const v = editorText(titleRef);
                setText(v);
                props.onLiveText?.(v);
              }}
              onKeyDown={onTitleKeyDown}
              onPaste={pasteAsPlainText}
              onClick={(e) => openLinkOnClick(e, titleRef)}
            />

          {/* Badge row: the move-to-list picker first, then the
              always-visible deadline badge — clicking it opens a
              quick popover (Set date… / Tomorrow / Remove date).
              The pin toggle beside it adds / removes the Focus ref;
              hidden on Done / Binned items, which can't hold one
              (spec/focus.md). */}
          <div class="task-dialog-badges">
            <ListPicker
              options={listOptions}
              value={() => it().listId}
              onChange={(id) => moveItemToList(id, it().listId)}
            />
            {/* Lifecycle status badge: hidden while binned (the bin mask
                overrides the workflow state; Restore is the way out). */}
            <Show when={!isBinned(it())}>
              <LifecycleBadge
                value={() => it().state}
                onChange={(state) => props.app.setLifecycle(it().id, state)}
              />
            </Show>
            <DeadlineField
              deadline={() => it().deadline ?? null}
              muted={() => isDone(it()) || isBinned(it())}
              onChange={(stamp) =>
                props.app.setItemDeadline(it().id, stamp)
              }
              open={deadlineCalOpen}
              setOpen={setDeadlineCalOpen}
            />
            <Show when={!isDone(it()) && !isBinned(it())}>
              <PinToggle pinned={focused} onToggle={toggleFocus} />
            </Show>
          </div>

          {/* Date section: the planned date on its own ruled band
              between the badges and the notes. */}
          <div class="task-dialog-dates">
            <WhenField
              when={() => it().when ?? null}
              duration={() => it().duration ?? null}
              muted={() => isDone(it()) || isBinned(it())}
              onChange={(value) => props.app.setItemWhen(it().id, value)}
              onDurationChange={(minutes) =>
                props.app.setItemDuration(it().id, minutes)
              }
              open={whenCalOpen}
              setOpen={setWhenCalOpen}
            />
          </div>

          <div
            ref={(el) => {
              notesRef = el;
              el.setAttribute("contenteditable", "true");
              setLinkifiedText(el, notes());
            }}
            class="task-dialog-notes"
            role="textbox"
            aria-multiline="true"
            data-placeholder={m().workspace.notes}
            on:input={onNotesInput}
            onBlur={flushNotesNow}
            on:compositionstart={onNotesCompositionStart}
            on:compositionend={onNotesCompositionEnd}
            onKeyDown={onNotesKeyDown}
            on:beforeinput={onNotesBeforeInput}
            onPaste={pasteAsPlainText}
            onClick={(e) => openLinkOnClick(e, notesRef)}
          />

            </div>
          </div>
        </>
      )}
    </Show>
    </>
  );

  // Three shells around one body. Desktop default: a centred modal Dialog
  // (focus trap, overlay, Escape from Kobalte). Mobile: a plain
  // full-screen page under the floating pills — no modal, no portal, so
  // the pills stay live and nothing counts as an "outside" interaction.
  // Desktop with the side panel showing: an inline pane portaled into
  // the panel's host element, non-modal like the page.

  // Non-modal shells: focus the editor whenever the target changes (the
  // dialog does this via onOpenAutoFocus, but it can't swap targets while
  // open) and hand focus back once nothing is open. A followed (not
  // entered) item skips the focus so keyboard nav stays on the list.
  const nonModal = () => isMobile() || panelMode();
  createEffect(() => {
    if (!nonModal() || !open()) return;
    // Re-run per target, not just per open: a row click while the pane
    // shows another item lands the caret in the new title. Also re-runs
    // when the same item is re-entered (`entered` notifies).
    props.itemId();
    newItemTarget();
    // A capture has no row to follow: it is always entered, whatever the
    // owner's entered flag (keyed on an open item id, which a new item
    // doesn't have yet) says.
    if (!isNew() && props.entered?.() === false) return;
    // A click into the pane entered it with focus already where the
    // user put it; don't yank the caret to the title.
    if (shellRef?.contains(document.activeElement)) return;
    focusOnOpen();
  });
  createEffect(() => {
    if (!nonModal() || !open()) return;
    onCleanup(() => props.onClosed?.());
  });

  return (
    <Switch
      fallback={
        <Show when={open()}>
          <section
            class="task-dialog task-page"
            role="region"
            aria-label={m().common.close}
            data-shortcuts-inert=""
            onKeyDown={onShellKeyDown}
          >
            {body()}
          </section>
        </Show>
      }
    >
      <Match when={panelHost()}>
        {(host) => (
          <Show when={open()}>
            <Portal mount={host()}>
              <section
                ref={shellRef}
                class="task-dialog task-panel"
                role="region"
                aria-label={m().common.close}
                data-shortcuts-inert=""
                onKeyDown={onShellKeyDown}
                onFocusIn={() => {
                  if (props.entered?.() === false) props.onFocused?.();
                }}
              >
                {body()}
              </section>
            </Portal>
          </Show>
        )}
      </Match>
      <Match when={!isMobile()}>
      <Dialog
        open={open()}
        onOpenChange={(o) => {
          if (!o) close();
        }}
        modal
      >
        <Dialog.Portal>
          <Dialog.Overlay class="dialog-overlay" />
          <div class="dialog-positioner">
            <Dialog.Content
              class="task-dialog task-modal"
              onKeyDown={onShellKeyDown}
              onCloseAutoFocus={(e) => {
                // Kobalte would restore focus to whatever opened the dialog
                // (a row's open icon, the note badge, …). Take over and send
                // focus to the list so keyboard nav resumes there.
                e.preventDefault();
                props.onClosed?.();
              }}
              onOpenAutoFocus={(e) => {
                // Kobalte would focus the first tabbable (the close button);
                // take over and land the caret.
                e.preventDefault();
                focusOnOpen();
              }}
            >
              {body()}
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog>
      </Match>
    </Switch>
  );
}

/** The five pickable workflow states, in ladder order (the bin is not a
 *  state — it's reached from the header menu, not from here). */
const LIFECYCLE_CHOICES: readonly WorkflowState[] = [...OPEN_STATES, "done"];

/** Lifecycle status badge beside the list picker: shows the item's current
 *  workflow state and opens a menu of all five to move it in one commit.
 *  Backed by `setLifecycle` for open items and by the new-item target
 *  buffer in capture mode. */
function LifecycleBadge(props: {
  value: () => WorkflowState;
  onChange: (state: WorkflowState) => void;
}) {
  const { m } = useAppI18n();
  return (
    <DropdownMenu>
      <DropdownMenu.Trigger
        class="badge task-dialog-lifecycle"
        aria-label={m().workspace.changeStatus}
        title={m().workspace.changeStatus}
      >
        <span class="task-dialog-lifecycle-value">
          {laneLabel(m(), props.value())}
        </span>
        <span
          class="task-dialog-list-caret"
          aria-hidden="true"
          innerHTML={caretSortSvg}
        />
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="dropdown-menu-content task-dialog-lifecycle-menu">
          <DropdownMenu.RadioGroup
            value={props.value()}
            onChange={(v) => props.onChange(v as WorkflowState)}
          >
            <For each={LIFECYCLE_CHOICES}>
              {(state) => (
                <DropdownMenu.RadioItem
                  value={state}
                  class="dropdown-menu-item task-dialog-lifecycle-item"
                  // Radio items keep the menu open by default (built for
                  // toggling); picking a state is a one-shot move, so close.
                  closeOnSelect
                >
                  <span>{laneLabel(m(), state)}</span>
                  <DropdownMenu.ItemIndicator
                    class="task-dialog-lifecycle-check"
                    aria-hidden="true"
                    innerHTML={checkSvg}
                  />
                </DropdownMenu.RadioItem>
              )}
            </For>
          </DropdownMenu.RadioGroup>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu>
  );
}

/** Pin-to-Focus toggle shown beside the deadline badge: outline pin when
 *  unpinned, filled when pinned. Backed by live Focus state for open items
 *  and by the `newFocus` buffer in new-item capture mode. */
function PinToggle(props: {
  pinned: () => boolean;
  onToggle: () => void;
}) {
  const { m } = useAppI18n();
  const label = () => (props.pinned() ? m().focus.remove : m().focus.add);
  return (
    <button
      type="button"
      class="badge task-dialog-pin-toggle"
      aria-pressed={props.pinned()}
      aria-label={label()}
      title={label()}
      onClick={props.onToggle}
      innerHTML={props.pinned() ? drawingPinFilledSvg : drawingPinSvg}
    />
  );
}
