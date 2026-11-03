// The app's settings on the page (`hostSettings.ts`): the
// document-start global is what the module reads at import, a live
// push merges over it and reaches the listeners, and nothing is there
// outside the app. The module keeps state at import time, so each case
// imports a fresh copy with the global set first.

import { describe, expect, test } from "bun:test";

let n = 0;
async function fresh(global: unknown) {
  (globalThis as Record<string, unknown>).window = global === undefined ? undefined : { __monoplanSettings: global };
  // A query string defeats the module cache.
  return import(`../src/hostSettings.ts?case=${n++}`);
}

describe("hostSettings", () => {
  test("absent outside the app", async () => {
    const m = await fresh(undefined);
    expect(m.hasHostSettings()).toBe(false);
    expect(m.hostSetting("typedDates")).toBeUndefined();
  });

  test("reads the document-start global", async () => {
    const m = await fresh({ typedDates: false });
    expect(m.hasHostSettings()).toBe(true);
    expect(m.hostSetting("typedDates")).toBe(false);
  });

  test("a live push merges and notifies", async () => {
    const m = await fresh({ typedDates: false });
    const seen: unknown[] = [];
    const off = m.onHostSettings((s: unknown) => seen.push(s));
    m.applyHostSettings({ typedDates: true });
    expect(m.hostSetting("typedDates")).toBe(true);
    expect(seen).toEqual([{ typedDates: true }]);
    off();
    m.applyHostSettings({ typedDates: false });
    expect(seen.length).toBe(1);
  });

  test("a junk global counts as absent", async () => {
    const m = await fresh("nope");
    expect(m.hasHostSettings()).toBe(false);
  });
});
