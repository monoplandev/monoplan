// `applySelectionKeyAction` is the one answer to the selection keys for
// both the listbox controller and the agenda. It returns the key to
// scroll to; the agenda's order crosses day boundaries, so ↑ / ↓ must run
// straight through the flat order.

import { describe, expect, test } from "bun:test";
import { applySelectionKeyAction } from "../src/dnd/core/keyboard.ts";
import { DndSelection } from "../src/dnd/core/selection.ts";

const multi = { multi: true };

describe("applySelectionKeyAction", () => {
  test("navigate from nothing lands on the edge, then walks and clamps", () => {
    const order = ["a", "b", "c"];
    const sel = new DndSelection(order);
    expect(
      applySelectionKeyAction(sel, { type: "navigate", direction: "down" }, order, multi),
    ).toBe("a");
    expect(
      applySelectionKeyAction(sel, { type: "navigate", direction: "down" }, order, multi),
    ).toBe("b");
    applySelectionKeyAction(sel, { type: "navigate", direction: "down" }, order, multi);
    expect(
      applySelectionKeyAction(sel, { type: "navigate", direction: "down" }, order, multi),
    ).toBe("c");
    expect(sel.getSelectedKeys()).toEqual(["c"]);
    expect(
      applySelectionKeyAction(sel, { type: "navigate", direction: "up" }, order, multi),
    ).toBe("b");
  });

  test("extend grows a block; single-select collapses it", () => {
    const order = ["a", "b", "c"];
    const sel = new DndSelection(order);
    sel.selectOnly("a");
    expect(
      applySelectionKeyAction(sel, { type: "extend", direction: "down" }, order, multi),
    ).toBe("b");
    expect(sel.getSelectedKeys()).toEqual(["a", "b"]);
    expect(
      applySelectionKeyAction(sel, { type: "extend-to-last" }, order, multi),
    ).toBe("c");
    expect(sel.getSelectedKeys()).toEqual(["a", "b", "c"]);

    const single = new DndSelection(order);
    single.selectOnly("a");
    applySelectionKeyAction(single, { type: "extend", direction: "down" }, order, {
      multi: false,
    });
    expect(single.getSelectedKeys()).toEqual(["b"]);
    applySelectionKeyAction(single, { type: "select-all" }, order, { multi: false });
    expect(single.getSelectedKeys()).toEqual(["b"]);
  });

  test("first / last / select-all / clear", () => {
    const order = ["a", "b", "c"];
    const sel = new DndSelection(order);
    expect(applySelectionKeyAction(sel, { type: "select-only-last" }, order, multi)).toBe("c");
    expect(applySelectionKeyAction(sel, { type: "select-only-first" }, order, multi)).toBe("a");
    expect(applySelectionKeyAction(sel, { type: "select-all" }, order, multi)).toBeNull();
    expect(sel.getSelectedKeys()).toEqual(["a", "b", "c"]);
    expect(applySelectionKeyAction(sel, { type: "clear" }, order, multi)).toBeNull();
    expect(sel.hasSelection()).toBe(false);
  });

  test("an empty order is inert", () => {
    const sel = new DndSelection([]);
    expect(applySelectionKeyAction(sel, { type: "navigate", direction: "down" }, [], multi)).toBeNull();
    expect(applySelectionKeyAction(sel, { type: "select-only-first" }, [], multi)).toBeNull();
    expect(applySelectionKeyAction(sel, { type: "extend-to-last" }, [], multi)).toBeNull();
    expect(sel.hasSelection()).toBe(false);
  });
});
