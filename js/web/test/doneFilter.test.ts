import { describe, expect, test } from "bun:test";

import {
  doneFilterAdmits,
  doneListKey,
  effectiveDoneFilter,
  offeredDoneLists,
  shownDoneLists,
  toggled,
  type DoneListOption,
} from "../src/doneFilter.ts";

const list = (docId: string, listId: string): DoneListOption => ({
  key: doneListKey(docId, listId),
  docId,
  listId,
  name: listId,
});
const LISTS = [list("home", "inbox"), list("home", "l1"), list("team", "inbox"), list("team", "l2")];
const DOCS = ["home", "team"];
const item = (docId: string, listId: string) => ({ docId, listId });

describe("done filter", () => {
  test("an empty filter admits everything", () => {
    const admits = doneFilterAdmits(effectiveDoneFilter({ docs: [], lists: [] }, DOCS, LISTS));
    expect(admits(item("home", "l1"))).toBe(true);
    expect(admits(item("team", "inbox"))).toBe(true);
  });

  test("workspaces and lists are ANDed; inbox is per doc", () => {
    const f = effectiveDoneFilter(
      { docs: ["team"], lists: [doneListKey("team", "inbox")] },
      DOCS,
      LISTS,
    );
    const admits = doneFilterAdmits(f);
    expect(admits(item("team", "inbox"))).toBe(true);
    expect(admits(item("team", "l2"))).toBe(false);
    expect(admits(item("home", "inbox"))).toBe(false);
  });

  test("several lists across workspaces", () => {
    const f = effectiveDoneFilter(
      { docs: [], lists: [doneListKey("home", "l1"), doneListKey("team", "l2")] },
      DOCS,
      LISTS,
    );
    const admits = doneFilterAdmits(f);
    expect(admits(item("home", "l1"))).toBe(true);
    expect(admits(item("team", "l2"))).toBe(true);
    expect(admits(item("home", "inbox"))).toBe(false);
  });

  test("closed docs, deleted lists and lists of unchecked workspaces drop out", () => {
    const f = effectiveDoneFilter(
      { docs: ["home", "gone"], lists: [doneListKey("team", "l2"), doneListKey("home", "dead")] },
      DOCS,
      LISTS,
    );
    expect(f).toEqual({ docs: ["home"], lists: [] });
    expect(offeredDoneLists(f.docs, LISTS).map((l) => l.key)).toEqual([
      doneListKey("home", "inbox"),
      doneListKey("home", "l1"),
    ]);
    expect(doneFilterAdmits(f)(item("home", "l1"))).toBe(true);
  });

  test("archived lists show only when searched for or checked", () => {
    const old = { ...list("home", "old"), archived: true };
    const offered = [...LISTS, old];
    const matches = (name: string, q: string) => name.includes(q);
    const keys = (q: string, checked: string[] = []) =>
      shownDoneLists(offered, q, (k) => checked.includes(k), matches).map((l) => l.key);
    expect(keys("")).toEqual(LISTS.map((l) => l.key));
    expect(keys("ol")).toEqual([old.key]);
    expect(keys("l1")).toEqual([doneListKey("home", "l1")]);
    expect(keys("", [old.key])).toEqual(offered.map((l) => l.key));
  });

  test("toggled flips membership", () => {
    expect(toggled(["a"], "b")).toEqual(["a", "b"]);
    expect(toggled(["a", "b"], "a")).toEqual(["b"]);
  });
});
