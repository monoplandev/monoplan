// The task surface's subtasks (`spec/subtasks.md`): text nodes with
// done / not done, owned by the item. A column of rows, each a checkbox
// and a one-line text field, over a trailing "Add subtask" field that
// keeps the caret after Enter so a list can be typed straight down.
// Subtasks show here and nowhere else; a list row carries only a count.
//
// Keys in a row: Enter commits and moves down, Backspace in an emptied
// row removes it (the only way to remove one), Up / Down walk the rows,
// Alt+Up / Alt+Down reorder, Escape puts the text back.
//
// The pointer reorders by the grip at a row's end: the row follows the
// pointer, the rows it passes step aside, and the release is one move.

import { batch, createSignal, For, onCleanup } from "solid-js";
import { useAppI18n } from "./i18n.tsx";
import dragHandleSvg from "./icons/drag-handle.svg?raw";
import plusSvg from "./icons/plus.svg?raw";
import type { Subtask } from "./sync/store.ts";

export function SubtasksField(props: {
  subtasks: () => readonly Subtask[];
  onAdd: (text: string) => void;
  onText: (id: string, text: string) => void;
  onDone: (id: string, done: boolean) => void;
  /** Move the subtask so it ends at `index`. */
  onMove: (id: string, index: number) => void;
  onRemove: (id: string) => void;
}) {
  const { m } = useAppI18n();
  let rootRef: HTMLDivElement | undefined;
  let addRef: HTMLInputElement | undefined;
  // Rows are keyed by id, not by the node objects (the store replaces
  // the whole array on every change), so a row's input survives a tick
  // or an edit elsewhere in the list with its caret.
  const ids = (): string[] => props.subtasks().map((s) => s.id);
  const subOf = (id: string): Subtask | undefined => props.subtasks().find((s) => s.id === id);

  const inputOf = (id: string): HTMLInputElement | null =>
    rootRef?.querySelector<HTMLInputElement>(`input[data-subtask="${id}"]`) ?? null;
  // Focus the row at `index`, or the add field past either end.
  const focusAt = (index: number): void => {
    const id = ids()[index];
    const el = id ? inputOf(id) : addRef;
    el?.focus();
  };

  const commit = (id: string, el: HTMLInputElement): void => {
    const sub = subOf(id);
    if (!sub) return;
    const text = el.value.trim();
    // An emptied row is not a delete on its own (Backspace is): the
    // text comes back.
    if (!text) el.value = sub.text;
    else if (text !== sub.text) props.onText(id, text);
  };

  const onRowKeyDown = (e: KeyboardEvent, id: string): void => {
    const el = e.currentTarget as HTMLInputElement;
    const at = ids().indexOf(id);
    if (e.key === "Enter") {
      e.preventDefault();
      commit(id, el);
      focusAt(at + 1);
    } else if (e.key === "Backspace" && el.value === "") {
      e.preventDefault();
      props.onRemove(id);
      // The row above takes the caret; the add field when it was first.
      queueMicrotask(() => focusAt(at > 0 ? at - 1 : ids().length));
    } else if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      const to = at + (e.key === "ArrowUp" ? -1 : 1);
      if (to < 0 || to >= ids().length) return;
      commit(id, el);
      props.onMove(id, to);
      // Moving the node in the DOM drops its focus.
      queueMicrotask(() => inputOf(id)?.focus());
    } else if (e.key === "ArrowUp" && at > 0) {
      e.preventDefault();
      focusAt(at - 1);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      focusAt(at + 1);
    } else if (e.key === "Escape") {
      const sub = subOf(id);
      if (sub && el.value !== sub.text) {
        // Only the edit is cancelled; a second Escape is the surface's.
        e.stopPropagation();
        el.value = sub.text;
      }
    }
  };

  const add = (): void => {
    if (!addRef) return;
    const text = addRef.value.trim();
    if (!text) return;
    addRef.value = "";
    props.onAdd(text);
  };

  // A grip drag in flight: the row at `from` is `dy` px off its slot
  // and would land at `to`. Rows are one line, so one height serves.
  const [drag, setDrag] = createSignal<{
    id: string;
    from: number;
    to: number;
    dy: number;
    rowHeight: number;
  } | null>(null);
  let endDrag: (() => void) | undefined;
  onCleanup(() => endDrag?.());

  const onGripDown = (e: PointerEvent, id: string): void => {
    if (e.button !== 0 || drag()) return;
    const grip = e.currentTarget as HTMLElement;
    const row = grip.closest<HTMLElement>(".subtask");
    const from = ids().indexOf(id);
    if (!row || from < 0) return;
    // No focus change and no text selection: an edit in progress in
    // another row keeps its caret through the drag.
    e.preventDefault();
    const rowHeight = row.getBoundingClientRect().height;
    const startY = e.clientY;
    const count = ids().length;
    const pointerId = e.pointerId;
    grip.setPointerCapture(pointerId);
    setDrag({ id, from, to: from, dy: 0, rowHeight });

    const finish = (drop: boolean): void => {
      const d = drag();
      endDrag?.();
      if (!d) return;
      // Reordering the DOM drops the caret from a moved row's input.
      const active = document.activeElement;
      const focused =
        active instanceof HTMLInputElement ? active.dataset.subtask : undefined;
      batch(() => {
        setDrag(null);
        // The list changing under the drag (a remote edit) voids it.
        if (drop && d.to !== d.from && ids()[d.from] === id && ids().length === count) {
          props.onMove(id, d.to);
        }
      });
      if (focused) queueMicrotask(() => inputOf(focused)?.focus());
    };
    const onMove = (ev: PointerEvent): void => {
      const { dy, to } = dragTarget(from, ev.clientY - startY, rowHeight, count);
      setDrag({ id, from, to, dy, rowHeight });
    };
    const onUp = (): void => finish(true);
    const onCancel = (): void => finish(false);
    const onKey = (ev: KeyboardEvent): void => {
      if (ev.key !== "Escape") return;
      // Escape ends the drag, not the surface.
      ev.preventDefault();
      ev.stopPropagation();
      finish(false);
    };
    grip.addEventListener("pointermove", onMove);
    grip.addEventListener("pointerup", onUp);
    grip.addEventListener("pointercancel", onCancel);
    grip.addEventListener("lostpointercapture", onCancel);
    document.addEventListener("keydown", onKey, true);
    endDrag = () => {
      endDrag = undefined;
      grip.removeEventListener("pointermove", onMove);
      grip.removeEventListener("pointerup", onUp);
      grip.removeEventListener("pointercancel", onCancel);
      grip.removeEventListener("lostpointercapture", onCancel);
      document.removeEventListener("keydown", onKey, true);
      if (grip.hasPointerCapture(pointerId)) grip.releasePointerCapture(pointerId);
    };
  };

  // The dragged row follows the pointer; rows between its slot and its
  // target step one row towards the slot it left.
  const rowShift = (id: string): string | undefined => {
    const d = drag();
    if (!d) return undefined;
    if (d.id === id) return `translateY(${d.dy}px)`;
    const at = ids().indexOf(id);
    if (at > d.from && at <= d.to) return `translateY(${-d.rowHeight}px)`;
    if (at < d.from && at >= d.to) return `translateY(${d.rowHeight}px)`;
    return undefined;
  };

  return (
    <div
      class="subtasks"
      ref={rootRef}
      role="group"
      aria-label={m().workspace.subtasks}
      data-dragging={drag() ? "" : undefined}
    >
      <For each={ids()}>
        {(id) => (
          <div
            class="subtask"
            data-done={subOf(id)?.done ? "" : undefined}
            data-dragged={drag()?.id === id ? "" : undefined}
            style={{ transform: rowShift(id) }}
          >
            <input
              type="checkbox"
              class="task-check"
              checked={subOf(id)?.done === true}
              onChange={(e) => props.onDone(id, e.currentTarget.checked)}
            />
            <input
              type="text"
              class="subtask-text"
              data-subtask={id}
              value={subOf(id)?.text ?? ""}
              aria-label={m().workspace.subtask}
              onBlur={(e) => commit(id, e.currentTarget)}
              onKeyDown={(e) => onRowKeyDown(e, id)}
            />
            {/* Pointer only: the keyboard reorders with Alt+Up / Down. */}
            <span
              class="subtask-grip"
              aria-hidden="true"
              innerHTML={dragHandleSvg}
              onPointerDown={(e) => onGripDown(e, id)}
            />
          </div>
        )}
      </For>
      <div class="subtask subtask-add">
        <span class="subtask-add-mark" aria-hidden="true" innerHTML={plusSvg} />
        <input
          type="text"
          class="subtask-text"
          ref={addRef}
          placeholder={m().workspace.addSubtask}
          aria-label={m().workspace.addSubtask}
          onBlur={add}
          onKeyDown={(e) => {
            const el = e.currentTarget;
            if (e.key === "Enter") {
              e.preventDefault();
              add();
            } else if ((e.key === "Backspace" || e.key === "ArrowUp") && el.value === "") {
              if (ids().length === 0) return;
              e.preventDefault();
              focusAt(ids().length - 1);
            } else if (e.key === "Escape" && el.value !== "") {
              e.stopPropagation();
              el.value = "";
            }
          }}
        />
      </div>
    </div>
  );
}

/** Where a grip drag stands: the row that started at `from` of `count`
 *  rows, each `rowHeight` tall, with the pointer `delta` px from where
 *  it went down. `dy` is the row's offset, held inside the list; `to`
 *  the index it would land at, changing as it crosses half a row. */
export function dragTarget(
  from: number,
  delta: number,
  rowHeight: number,
  count: number,
): { dy: number; to: number } {
  if (rowHeight <= 0 || count <= 0) return { dy: 0, to: from };
  const dy = Math.min(Math.max(delta, -from * rowHeight), (count - 1 - from) * rowHeight);
  return { dy, to: from + Math.round(dy / rowHeight) };
}

/** The count a list row carries for an item's subtasks, `done/total`,
 *  or null when it has none. */
export function subtaskCount(
  subtasks: readonly Subtask[] | undefined,
): { done: number; total: number } | null {
  if (!subtasks || subtasks.length === 0) return null;
  return { done: subtasks.filter((s) => s.done).length, total: subtasks.length };
}
