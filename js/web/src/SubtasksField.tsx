// The task surface's subtasks (`spec/subtasks.md`): text nodes with
// done / not done, owned by the item. A column of rows, each a checkbox
// and a text field, over a trailing "Add subtask" field that keeps the
// caret after Enter so a list can be typed straight down. A field is a
// textarea that grows with its text, so a long subtask wraps onto as
// many lines as it needs; the text itself stays one line (Enter
// commits, a pasted newline becomes a space).
// Subtasks show here and nowhere else; a list row carries only a count.
//
// Keys in a row: Enter commits and moves down, Backspace in an emptied
// row removes it (the only way to remove one), Up / Down walk the rows
// (within a wrapped row they move between its lines first), Alt+Up /
// Alt+Down reorder, Escape puts the text back.
//
// The pointer reorders by the grip at a row's end: the row follows the
// pointer, the rows it passes step aside, and the release is one move.

import { batch, createEffect, createSignal, For, onCleanup, onMount } from "solid-js";
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
  let addRef: HTMLTextAreaElement | undefined;
  // Rows are keyed by id, not by the node objects (the store replaces
  // the whole array on every change), so a row's input survives a tick
  // or an edit elsewhere in the list with its caret.
  const ids = (): string[] => props.subtasks().map((s) => s.id);
  const subOf = (id: string): Subtask | undefined => props.subtasks().find((s) => s.id === id);

  const inputOf = (id: string): HTMLTextAreaElement | null =>
    rootRef?.querySelector<HTMLTextAreaElement>(`textarea[data-subtask="${id}"]`) ?? null;
  // Focus the row at `index`, or the add field past either end, with
  // the caret at the start or the end of its text.
  const focusAt = (index: number, caret: "start" | "end" = "end"): void => {
    const id = ids()[index];
    const el = id ? inputOf(id) : addRef;
    if (!el) return;
    el.focus();
    const at = caret === "start" ? 0 : el.value.length;
    el.setSelectionRange(at, at);
  };

  // Every field (the add field too) is sized to its wrapped text, again
  // whenever the list's width changes (the panel resizes).
  onMount(() => {
    if (!rootRef) return;
    const ro = new ResizeObserver(() => {
      rootRef?.querySelectorAll<HTMLTextAreaElement>("textarea.subtask-text").forEach(fit);
    });
    ro.observe(rootRef);
    onCleanup(() => ro.disconnect());
  });

  const commit = (id: string, el: HTMLTextAreaElement): void => {
    const sub = subOf(id);
    if (!sub) return;
    const text = oneLine(el.value);
    // An emptied row is not a delete on its own (Backspace is): the
    // text comes back.
    if (!text) setField(el, sub.text);
    else if (text !== sub.text) props.onText(id, text);
  };

  const onRowKeyDown = (e: KeyboardEvent, id: string): void => {
    const el = e.currentTarget as HTMLTextAreaElement;
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
    } else if (e.key === "ArrowUp" && at > 0 && atEdge(el, "up")) {
      e.preventDefault();
      focusAt(at - 1);
    } else if (e.key === "ArrowDown" && atEdge(el, "down")) {
      e.preventDefault();
      focusAt(at + 1, "start");
    } else if (e.key === "Escape") {
      const sub = subOf(id);
      if (sub && el.value !== sub.text) {
        // Only the edit is cancelled; a second Escape is the surface's.
        e.stopPropagation();
        setField(el, sub.text);
      }
    }
  };

  const add = (): void => {
    if (!addRef) return;
    const text = oneLine(addRef.value);
    if (!text) return;
    setField(addRef, "");
    props.onAdd(text);
  };

  // A grip drag in flight: the row at `from` is `dy` px off its slot
  // and would land at `to`. `heights` are the rows' heights at the
  // start (a wrapped row is taller); the rows it passes step aside by
  // the dragged row's height.
  const [drag, setDrag] = createSignal<{
    id: string;
    from: number;
    to: number;
    dy: number;
    heights: readonly number[];
  } | null>(null);
  let endDrag: (() => void) | undefined;
  onCleanup(() => endDrag?.());

  const onGripDown = (e: PointerEvent, id: string): void => {
    if (e.button !== 0 || drag()) return;
    const grip = e.currentTarget as HTMLElement;
    const from = ids().indexOf(id);
    if (from < 0 || !rootRef) return;
    // No focus change and no text selection: an edit in progress in
    // another row keeps its caret through the drag.
    e.preventDefault();
    const heights = Array.from(
      rootRef.querySelectorAll<HTMLElement>(".subtask:not(.subtask-add)"),
      (row) => row.getBoundingClientRect().height,
    );
    const startY = e.clientY;
    const count = ids().length;
    if (heights.length !== count) return;
    const pointerId = e.pointerId;
    grip.setPointerCapture(pointerId);
    setDrag({ id, from, to: from, dy: 0, heights });

    const finish = (drop: boolean): void => {
      const d = drag();
      endDrag?.();
      if (!d) return;
      // Reordering the DOM drops the caret from a moved row's input.
      const active = document.activeElement;
      const focused =
        active instanceof HTMLTextAreaElement ? active.dataset.subtask : undefined;
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
      const { dy, to } = dragTarget(from, ev.clientY - startY, heights);
      setDrag({ id, from, to, dy, heights });
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
    const h = d.heights[d.from] ?? 0;
    if (at > d.from && at <= d.to) return `translateY(${-h}px)`;
    if (at < d.from && at >= d.to) return `translateY(${h}px)`;
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
            {/* The leading box is one line tall, so the checkbox sits on
                a wrapped row's first line. */}
            <span class="subtask-lead">
              <input
                type="checkbox"
                class="task-check"
                checked={subOf(id)?.done === true}
                onChange={(e) => props.onDone(id, e.currentTarget.checked)}
              />
            </span>
            <textarea
              class="subtask-text"
              rows={1}
              data-subtask={id}
              ref={(el) => {
                // The text arriving (an edit here or elsewhere) resizes.
                createEffect(() => setField(el, subOf(id)?.text ?? "", true));
              }}
              aria-label={m().workspace.subtask}
              onInput={(e) => onFieldInput(e.currentTarget)}
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
        <span class="subtask-lead">
          <span class="subtask-add-mark" aria-hidden="true" innerHTML={plusSvg} />
        </span>
        <textarea
          class="subtask-text"
          rows={1}
          ref={addRef}
          onInput={(e) => onFieldInput(e.currentTarget)}
          placeholder={m().workspace.addSubtask}
          aria-label={m().workspace.addSubtask}
          onBlur={add}
          onKeyDown={(e) => {
            const el = e.currentTarget;
            if (e.key === "Enter") {
              e.preventDefault();
              add();
            } else if (
              (e.key === "Backspace" && el.value === "") ||
              (e.key === "ArrowUp" && atEdge(el, "up"))
            ) {
              if (ids().length === 0) return;
              e.preventDefault();
              focusAt(ids().length - 1);
            } else if (e.key === "Escape" && el.value !== "") {
              e.stopPropagation();
              setField(el, "");
            }
          }}
        />
      </div>
    </div>
  );
}

/** A subtask's text as stored: one line, trimmed. A wrapped field's
 *  newlines (only a paste or a drop brings one) become spaces. */
function oneLine(text: string): string {
  return text.replace(/\s*[\r\n]+\s*/g, " ").trim();
}

/** Grow (or shrink) a field to its wrapped text. */
function fit(el: HTMLTextAreaElement): void {
  el.style.height = "auto";
  el.style.height = `${el.scrollHeight}px`;
}

/** Put `text` in a field and fit it. `keepEdit` leaves a focused
 *  field's in-progress text alone (a remote write must not clobber
 *  what is being typed). */
function setField(el: HTMLTextAreaElement, text: string, keepEdit = false): void {
  if (keepEdit && document.activeElement === el) return;
  if (el.value !== text) el.value = text;
  // A field not yet in the document has no layout to measure.
  if (el.isConnected) fit(el);
  else queueMicrotask(() => fit(el));
}

/** Typing or a paste: newlines out at once (the length is kept, so the
 *  caret stays put), then fit. */
function onFieldInput(el: HTMLTextAreaElement): void {
  if (el.value.includes("\n")) {
    const { selectionStart, selectionEnd } = el;
    el.value = el.value.replace(/\n/g, " ");
    el.setSelectionRange(selectionStart, selectionEnd);
  }
  fit(el);
}

/** Whether Up / Down should leave the field for the row beside it: a
 *  field of one visual line always does, a wrapped one only from the
 *  very start (Up) or end (Down) of its text, so the arrows walk its
 *  lines first (the platform takes the caret to an end on the first /
 *  last line). */
function atEdge(el: HTMLTextAreaElement, dir: "up" | "down"): boolean {
  if (el.selectionStart !== el.selectionEnd) return false;
  const cs = getComputedStyle(el);
  const pad = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
  const line = parseFloat(cs.lineHeight);
  if (!Number.isFinite(line) || el.scrollHeight - pad < line * 1.5) return true;
  return dir === "up" ? el.selectionStart === 0 : el.selectionEnd === el.value.length;
}

/** Where a grip drag stands: the row that started at `from`, of rows
 *  `heights` tall, with the pointer `delta` px from where it went
 *  down. `dy` is the row's offset, held inside the list; `to` the
 *  index it would land at, changing as it crosses half of the next
 *  row over. */
export function dragTarget(
  from: number,
  delta: number,
  heights: readonly number[],
): { dy: number; to: number } {
  const count = heights.length;
  if (count === 0 || from < 0 || from >= count || heights.some((h) => !(h > 0))) {
    return { dy: 0, to: from };
  }
  const sum = (a: number, b: number): number =>
    heights.slice(a, b).reduce((t, h) => t + h, 0);
  const dy = Math.min(Math.max(delta, -sum(0, from)), sum(from + 1, count));
  let to = from;
  if (dy > 0) {
    while (to + 1 < count && dy >= sum(from + 1, to + 1) + heights[to + 1]! / 2) to++;
  } else {
    while (to > 0 && -dy >= sum(to, from) + heights[to - 1]! / 2) to--;
  }
  return { dy, to };
}

/** The count a list row carries for an item's subtasks, `done/total`,
 *  or null when it has none. */
export function subtaskCount(
  subtasks: readonly Subtask[] | undefined,
): { done: number; total: number } | null {
  if (!subtasks || subtasks.length === 0) return null;
  return { done: subtasks.filter((s) => s.done).length, total: subtasks.length };
}
