// Deadline coverage across the two JS layers the feature touches:
//  - the workspace store hydrates `deadline` from the initial ItemAdded
//    burst and patches it on live itemDeadlineChanged events;
//  - `formatDeadlineBadge` produces the spec's overdue/today/tomorrow/weekday/
//    compact-date labels from local date parts.

import { expect, test, describe } from "bun:test";

import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";
import {
  addDaysToStamp,
  formatDeadlineBadge,
  localDateStamp,
  todayStamp,
} from "../src/format.tsx";

const DOC_ID = "00000000-0000-0000-0000-000000000000";

function engineFrom(doc: Doc): SyncEngine {
  return new SyncEngine(
    doc,
    DOC_ID,
    Dek.generate(),
    0n,
    new MemEngineStorage() as unknown as EngineStorage,
  );
}

describe("store deadline", () => {
  test("hydrates deadline from the attach burst", () => {
    const doc = Doc.create();
    const id = doc.addItem("inbox", "with a deadline");
    doc.setItemDeadline(id, "2026-07-15");

    const app = createSyncedApp(engineFrom(doc));
    expect(app.state.itemsById[id]?.deadline).toBe("2026-07-15");
  });

  test("patches deadline on set, then clears it", () => {
    const doc = Doc.create();
    const id = doc.addItem("inbox", "task");
    const app = createSyncedApp(engineFrom(doc));
    expect(app.state.itemsById[id]?.deadline).toBeUndefined();

    app.setItemDeadline(id, "2026-12-01");
    expect(app.state.itemsById[id]?.deadline).toBe("2026-12-01");

    app.setItemDeadline(id, null);
    expect(app.state.itemsById[id]?.deadline).toBeUndefined();
  });
});

describe("date-part helpers", () => {
  test("localDateStamp / todayStamp use local calendar parts", () => {
    // 2026-03-09, well clear of any midnight edge — same day locally.
    const d = new Date(2026, 2, 9, 13, 30);
    expect(localDateStamp(d)).toBe("2026-03-09");
    expect(todayStamp(d.getTime())).toBe("2026-03-09");
  });

  test("addDaysToStamp crosses a month boundary", () => {
    expect(addDaysToStamp("2026-01-31", 1)).toBe("2026-02-01");
    expect(addDaysToStamp("2026-03-01", -1)).toBe("2026-02-28");
  });
});

describe("formatDeadlineBadge", () => {
  const labels = { overdue: "Overdue", today: "Today", tomorrow: "Tomorrow" };
  const today = "2026-07-07"; // a Tuesday

  test("before today is overdue", () => {
    expect(formatDeadlineBadge("2026-07-06", today, labels, "en-US")).toEqual({
      label: "Overdue",
      urgency: "overdue",
    });
    expect(formatDeadlineBadge("2020-01-01", today, labels, "en-US")?.urgency).toBe(
      "overdue",
    );
  });

  test("pastAsDate shows the date instead of Overdue", () => {
    expect(
      formatDeadlineBadge("2026-07-06", today, labels, "en-US", { pastAsDate: true }),
    ).toEqual({ label: "Jul 6", urgency: "overdue" });
    expect(
      formatDeadlineBadge("2020-01-01", today, labels, "en-US", { pastAsDate: true })?.label,
    ).toBe("Jan 1, 2020");
    // Only the past branch changes; today/tomorrow keep their labels.
    expect(
      formatDeadlineBadge(today, today, labels, "en-US", { pastAsDate: true })?.label,
    ).toBe("Today");
  });

  test("today and tomorrow", () => {
    expect(formatDeadlineBadge(today, today, labels, "en-US")).toEqual({
      label: "Today",
      urgency: "today",
    });
    expect(formatDeadlineBadge("2026-07-08", today, labels, "en-US")).toEqual({
      label: "Tomorrow",
      urgency: "future",
    });
  });

  test("within the next 7 days shows a weekday", () => {
    // 2026-07-10 is a Friday.
    const r = formatDeadlineBadge("2026-07-10", today, labels, "en-US");
    expect(r?.urgency).toBe("future");
    expect(r?.label).toBe("Fri");
  });

  test("further out shows a compact date", () => {
    const r = formatDeadlineBadge("2026-07-20", today, labels, "en-US");
    expect(r?.urgency).toBe("future");
    expect(r?.label).toBe("Jul 20");
  });

  test("null on a malformed stamp", () => {
    expect(formatDeadlineBadge("nope", today, labels, "en-US")).toBeNull();
  });
});
