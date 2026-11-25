// The 1–4 digit shortcuts' decision logic: which fixed nav view a keydown
// targets, and when it must be a no-op. The overlay / editable-surface
// guards are centralized in `onGlobalKey` (overlay.ts) and are not
// re-tested here; the modifier guard belongs to the mapping itself.

import { describe, expect, test } from "bun:test";

import { digitNavTarget } from "../src/navShortcuts.ts";

const key = (
  k: string,
  mods: Partial<
    Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "altKey" | "shiftKey">
  > = {},
) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("digitNavTarget", () => {
  test("1–4 map to Upcoming / Focus / Done / Inbox in sidebar order", () => {
    expect(digitNavTarget(key("1"))).toEqual({ kind: "upcoming" });
    expect(digitNavTarget(key("2"))).toEqual({ kind: "focus" });
    expect(digitNavTarget(key("3"))).toEqual({ kind: "done" });
    expect(digitNavTarget(key("4"))).toEqual({ kind: "list", id: "inbox" });
  });

  test("Bin has no digit", () => {
    expect(digitNavTarget(key("5"))).toBeNull();
  });

  test("any modifier disqualifies the key", () => {
    // Cmd+1 etc. belong to the browser (tab switching); never hijack them.
    for (const mods of [
      { metaKey: true },
      { ctrlKey: true },
      { altKey: true },
      { shiftKey: true },
    ] as const) {
      expect(digitNavTarget(key("1", mods))).toBeNull();
      expect(digitNavTarget(key("4", mods))).toBeNull();
    }
  });

  test("non-digit and out-of-range keys are ignored", () => {
    for (const k of ["0", "6", "9", "a", "Enter", "[", " "]) {
      expect(digitNavTarget(key(k))).toBeNull();
    }
  });
});
