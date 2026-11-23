// The zone picker's list and matching (`zones.ts`), and a fixed `when`
// through the web store (`spec/calendar-plan.md` "Fixed instants"). The
// conversions are the core's; what is checked here is that the web layer
// hands it the right values. Nothing depends on the machine's zone:
// expectations are built from the core's own answers.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, Replica, whenInZone } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import { clipFromHtml, renderClip, toClipItem } from "../src/itemClip.ts";
import { createSyncedApp } from "../src/sync/store.ts";
import {
  fixedWhen,
  localWhen,
  storedWhen,
  wallInZone,
  zoneCity,
  zoneIds,
  zoneWall,
  zoneOption,
  zoneOptions,
  zoneSuggestions,
} from "../src/zones.ts";

const DOC_ID = "00000000-0000-0000-0000-000000000000";

function app() {
  return createSyncedApp(
    new Replica(
      Doc.create(),
      DOC_ID,
      Dek.generate(),
      0n,
      new MemReplicaStorage() as unknown as ReplicaStorage,
    ),
  );
}

const WHEN = "2026-07-13T14:00";

describe("zone list", () => {
  test("a city is the id's last segment", () => {
    expect(zoneCity("Europe/Paris")).toBe("Paris");
    expect(zoneCity("America/Argentina/Buenos_Aires")).toBe("Buenos Aires");
    expect(zoneCity("UTC")).toBe("UTC");
  });

  test("lists UTC and no id twice", () => {
    const ids = zoneIds();
    expect(ids).toContain("UTC");
    expect(new Set(ids).size).toBe(ids.length);
  });

  test("an option carries the offset on the day asked about", () => {
    expect(zoneOption("Europe/Paris", "en", "2026-07-13").offset).toBe("GMT+2");
    expect(zoneOption("Europe/Paris", "en", "2026-01-13").offset).toBe("GMT+1");
    expect(zoneOption("Europe/Paris", "en", "2026-01-13").name).toContain("European");
  });

  test("a zone the browser does not know still has its city", () => {
    expect(zoneOption("Mars/Olympus_Mons", "en", "2026-07-13")).toEqual({
      id: "Mars/Olympus_Mons",
      city: "Olympus Mons",
      name: "",
      offset: "",
    });
  });
});

describe("zone suggestions", () => {
  // A fixed list: which ids a browser lists (and under which alias)
  // varies by engine.
  const opt = (id: string, name: string, offset: string) => ({
    id,
    city: zoneCity(id),
    name,
    offset,
  });
  const options = [
    opt("America/Argentina/Buenos_Aires", "Argentina Standard Time", "GMT-3"),
    opt("America/North_Dakota/Center", "Central Time", "GMT-6"),
    opt("America/New_York", "Eastern Time", "GMT-5"),
    opt("America/Paramaribo", "Suriname Time", "GMT-3"),
    opt("Europe/Paris", "Central European Time", "GMT+1"),
    opt("America/Sao_Paulo", "Brasilia Standard Time", "GMT-3"),
    opt("Asia/Tokyo", "Japan Standard Time", "GMT+9"),
  ];
  const ids = (q: string, first?: string) =>
    zoneSuggestions(q, options, first).map((o) => o.id);

  test("blank is every zone, the device's first", () => {
    const all = ids("", "Europe/Paris");
    expect(all.length).toBe(options.length);
    expect(all[0]).toBe("Europe/Paris");
    expect(all.filter((id) => id === "Europe/Paris").length).toBe(1);
    expect(ids("", "Mars/Olympus_Mons")).toEqual(options.map((o) => o.id));
  });

  test("a city prefix matches, in list order", () => {
    expect(ids("par")).toEqual(["America/Paramaribo", "Europe/Paris"]);
    expect(ids("new y")).toEqual(["America/New_York"]);
    expect(ids("aires")).toEqual(["America/Argentina/Buenos_Aires"]);
  });

  test("accents and case are ignored", () => {
    expect(ids("SÃO PAULO")).toEqual(["America/Sao_Paulo"]);
  });

  test("matches the zone's name, region and offset", () => {
    expect(ids("japan")).toEqual(["Asia/Tokyo"]);
    expect(ids("europe par")).toEqual(["Europe/Paris"]);
    expect(ids("gmt+1")).toEqual(["Europe/Paris"]);
    expect(ids("utc+9")).toEqual(["Asia/Tokyo"]);
  });

  test("a city match outranks a name match", () => {
    // "Central European Time" comes first in the list, but Center is
    // the city.
    expect(ids("cent")).toEqual(["America/North_Dakota/Center", "Europe/Paris"]);
  });

  test("every typed word must match", () => {
    expect(ids("paris tokyo")).toEqual([]);
    expect(ids("zzzz")).toEqual([]);
  });

  test("the browser's own list is searchable", () => {
    const all = zoneOptions("en", "2026-01-13");
    expect(all.length).toBe(zoneIds().length);
    expect(zoneSuggestions("tokyo", all).map((o) => o.id)).toContain("Asia/Tokyo");
  });
});

describe("zone conversions", () => {
  test("stored form names the zone's wall time", () => {
    const there = wallInZone(WHEN, "Asia/Tokyo");
    expect(there).toBe(whenInZone(WHEN, "Asia/Tokyo") ?? null);
    expect(storedWhen(WHEN, "Asia/Tokyo")).toBe(`${there}[Asia/Tokyo]`);
    expect(storedWhen(WHEN, null)).toBe(WHEN);
    // Not convertible: the floating value, never a suffix the core refuses.
    expect(storedWhen(WHEN, "Mars/Olympus_Mons")).toBe(WHEN);
    expect(storedWhen("2026-07-13", "Asia/Tokyo")).toBe("2026-07-13");
  });

  test("the fields' clock and the register round-trip", () => {
    const local = "2026-07-13T14:00";
    const wall = zoneWall(local, "Asia/Tokyo");
    expect(wall).toBe(wallInZone(local, "Asia/Tokyo")!);
    expect(fixedWhen(wall, "Asia/Tokyo")).toBe(`${wall}[Asia/Tokyo]`);
    expect(localWhen(wall, "Asia/Tokyo")).toBe(local);
    // No zone, a day, or a zone the tzdb lacks: plain values throughout.
    expect(zoneWall(local, null)).toBe(local);
    expect(fixedWhen(local, null)).toBe(local);
    expect(localWhen(local, null)).toBe(local);
    expect(fixedWhen("2026-07-13", "Asia/Tokyo")).toBe("2026-07-13");
    expect(zoneWall(local, "Mars/Olympus_Mons")).toBe(local);
    expect(fixedWhen(local, "Mars/Olympus_Mons")).toBe(local);
  });
});

describe("store when zone", () => {
  test("fixing a time keeps its digits in the zone", () => {
    const a = app();
    const id = a.addItem("inbox", "call");
    a.setItemWhen(id, WHEN);
    expect(a.getItem(id)?.whenZone).toBeUndefined();

    a.setItemWhenZone(id, "Asia/Tokyo");
    const it = a.getItem(id)!;
    expect(it.whenZone).toBe("Asia/Tokyo");
    expect(wallInZone(it.when!, "Asia/Tokyo")).toBe(WHEN);
    // The fields show the zone's clock: the digits did not move.
    expect(zoneWall(it.when!, it.whenZone)).toBe(WHEN);

    // An edit of those fields is written on the zone's clock.
    a.setItemWhen(id, fixedWhen("2026-07-14T09:30", "Asia/Tokyo"));
    expect(a.getItem(id)?.whenZone).toBe("Asia/Tokyo");
    expect(zoneWall(a.getItem(id)!.when!, "Asia/Tokyo")).toBe("2026-07-14T09:30");

    // A local-time write (a drag in the agenda) moves the instant and
    // keeps the zone.
    a.setItemWhen(id, "2026-07-14T09:30");
    expect(a.getItem(id)?.whenZone).toBe("Asia/Tokyo");
    expect(a.getItem(id)?.when).toBe("2026-07-14T09:30");

    // Floating again, at the time the device shows.
    a.setItemWhenZone(id, null);
    expect(a.getItem(id)?.whenZone).toBeUndefined();
    expect(a.getItem(id)?.when).toBe("2026-07-14T09:30");
  });

  test("an all-day date takes no zone and drops one", () => {
    const a = app();
    const id = a.addItem("inbox", "trip");
    a.setItemWhen(id, "2026-07-13");
    a.setItemWhenZone(id, "Asia/Tokyo");
    expect(a.getItem(id)?.whenZone).toBeUndefined();

    a.setItemWhen(id, WHEN);
    a.setItemWhenZone(id, "Asia/Tokyo");
    a.setItemWhen(id, "2026-07-13");
    expect(a.getItem(id)?.whenZone).toBeUndefined();
  });

  test("a no-op zone write is not an undo step", () => {
    const a = app();
    const id = a.addItem("inbox", "call");
    a.setItemWhen(id, WHEN);
    a.setItemWhenZone(id, "Asia/Tokyo");
    a.setItemWhenZone(id, "Asia/Tokyo");
    a.setItemWhenZone(id, "Asia/Tokyo");
    a.undo();
    expect(a.getItem(id)?.whenZone).toBeUndefined();
    expect(a.getItem(id)?.when).toBe(WHEN);
  });

  test("a copy written in the stored form lands on the same instant", () => {
    const a = app();
    const src = a.addItem("inbox", "call");
    a.setItemWhen(src, WHEN);
    a.setItemWhenZone(src, "Asia/Tokyo");
    const from = a.getItem(src)!;

    const clip = clipFromHtml(renderClip([toClipItem(from)]).html);
    expect(clip?.[0]?.whenZone).toBe("Asia/Tokyo");

    const copy = a.addItem("inbox", "call");
    a.setItemWhen(copy, storedWhen(from.when!, from.whenZone));
    expect(a.getItem(copy)?.when).toBe(from.when);
    expect(a.getItem(copy)?.whenZone).toBe("Asia/Tokyo");
  });
});
