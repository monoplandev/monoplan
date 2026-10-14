import type { DndSelection } from "./selection";
import type { Key } from "./types";

export type DndKeyAction =
  | { type: "select-only-first" }
  | { type: "select-only-last" }
  | { type: "navigate"; direction: "up" | "down" }
  | { type: "extend"; direction: "up" | "down" }
  | { type: "extend-to-first" }
  | { type: "extend-to-last" }
  | { type: "mod-click" } // cmd+click or ctrl+click — handled by mouse, not keyboard
  | { type: "move-selection"; direction: "up" | "down" }
  | { type: "select-all" }
  | { type: "clear" }
  | { type: "ignore" };

const isMac =
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad|iPod/.test(navigator.userAgent);

export function mapDndKeyEvent(e: KeyboardEvent): DndKeyAction {
  const mod = isMac ? e.metaKey : e.ctrlKey;

  // Escape — clear selection
  if (e.key === "Escape") {
    return { type: "clear" };
  }

  // Mod + A — select all
  if (mod && e.key === "a") {
    return { type: "select-all" };
  }

  // Alt/Option + Arrow — jump to first/last
  if (e.altKey && !e.shiftKey && e.key === "ArrowUp") {
    return { type: "select-only-first" };
  }
  if (e.altKey && !e.shiftKey && e.key === "ArrowDown") {
    return { type: "select-only-last" };
  }

  // Shift + Mod + Arrow — extend to first/last
  if (e.shiftKey && mod && e.key === "ArrowUp") {
    return { type: "extend-to-first" };
  }
  if (e.shiftKey && mod && e.key === "ArrowDown") {
    return { type: "extend-to-last" };
  }

  // Shift + Arrow — extend selection
  if (e.shiftKey && !mod && e.key === "ArrowUp") {
    return { type: "extend", direction: "up" };
  }
  if (e.shiftKey && !mod && e.key === "ArrowDown") {
    return { type: "extend", direction: "down" };
  }

  // Mod + Arrow — move selection (reorder)
  if (mod && !e.shiftKey && e.key === "ArrowUp") {
    return { type: "move-selection", direction: "up" };
  }
  if (mod && !e.shiftKey && e.key === "ArrowDown") {
    return { type: "move-selection", direction: "down" };
  }

  // Plain Arrow — navigate
  if (!e.shiftKey && !mod && !e.altKey && e.key === "ArrowUp") {
    return { type: "navigate", direction: "up" };
  }
  if (!e.shiftKey && !mod && !e.altKey && e.key === "ArrowDown") {
    return { type: "navigate", direction: "down" };
  }

  return { type: "ignore" };
}

/**
 * Apply a selection-only key action to `sel` over `order`, the way the
 * listbox controller does, and return the key the caller should scroll
 * into view (null when nothing moved). Shared with hosts that run a
 * `DndSelection` without the `Dnd` component (the agenda), so both
 * surfaces answer the same keys the same way. `move-selection` reorders
 * items and is left to the caller; `mod-click` and `ignore` are no-ops.
 */
export function applySelectionKeyAction(
  sel: DndSelection,
  action: DndKeyAction,
  order: readonly Key[],
  opts: { multi: boolean },
): Key | null {
  const edge = (dir: "up" | "down"): Key | undefined =>
    dir === "down" ? order[0] : order[order.length - 1];
  switch (action.type) {
    case "select-only-first":
      if (order.length === 0) return null;
      sel.selectOnly(sel.first());
      return sel.first();
    case "select-only-last":
      if (order.length === 0) return null;
      sel.selectOnly(sel.last());
      return sel.last();
    case "navigate": {
      if (!sel.hasSelection()) {
        const key = edge(action.direction);
        if (key === undefined) return null;
        sel.selectOnly(key);
        return key;
      }
      const ref =
        action.direction === "down"
          ? sel.getSelectionBottom()
          : sel.getSelectionTop();
      if (ref === null) return null;
      const target =
        action.direction === "down" ? sel.next(ref) : sel.prev(ref);
      sel.selectOnly(target);
      return target;
    }
    case "extend": {
      const active = sel.getActiveBlock();
      if (active === null) {
        const key = edge(action.direction);
        if (key === undefined) return null;
        sel.selectOnly(key);
        return key;
      }
      const target =
        action.direction === "down" ? sel.next(active.to) : sel.prev(active.to);
      if (opts.multi) sel.extendActive(target);
      else sel.selectOnly(target);
      return target;
    }
    case "extend-to-first":
      if (order.length === 0) return null;
      if (opts.multi) sel.extendActive(sel.first());
      else sel.selectOnly(sel.first());
      return sel.first();
    case "extend-to-last":
      if (order.length === 0) return null;
      if (opts.multi) sel.extendActive(sel.last());
      else sel.selectOnly(sel.last());
      return sel.last();
    case "select-all":
      if (opts.multi) sel.selectAll();
      return null;
    case "clear":
      sel.clear();
      return null;
    default:
      return null;
  }
}
