import { describe, expect, test } from "bun:test";

import {
  clipFromHtml,
  ITEM_CLIP_TYPE,
  readClip,
  closedLogLine,
  renderClip,
  toClipItem,
  type ClipItem,
} from "../src/itemClip.ts";
import type { ItemView } from "../src/sync/store.ts";

const item = (over: Partial<ItemView>): ItemView => ({
  id: "i1",
  docId: "doc-1",
  text: "Buy milk",
  notes: "",
  listId: "inbox",
  state: "backlog",
  lifecycleAt: 0,
  createdAt: 0,
  ...over,
});

/** A minimal DataTransfer stand-in: the reader uses `getData` and `types`. */
const transfer = (types: Record<string, string>): DataTransfer =>
  ({
    types: Object.keys(types),
    getData: (t: string) => types[t] ?? "",
  }) as unknown as DataTransfer;

/** Where the `item()` fixture is filed. */
const origin = { docId: "doc-1", listId: "inbox" };

describe("toClipItem", () => {
  test("carries only the clone-defining fields, dropping empties", () => {
    expect(toClipItem(item({}))).toEqual({
      text: "Buy milk",
      state: "backlog",
      origin,
    });
    expect(
      toClipItem(
        item({
          notes: "2 litres",
          deadline: "2026-10-01",
          when: "2026-09-30T09:00",
          duration: 30,
          state: "in_progress",
          startedAt: 5,
        }),
      ),
    ).toEqual({
      text: "Buy milk",
      notes: "2 litres",
      deadline: "2026-10-01",
      when: "2026-09-30T09:00",
      duration: 30,
      state: "in_progress",
      origin,
    });
  });

  test("carries the list the item is filed in as its origin", () => {
    expect(toClipItem(item({ docId: "doc-2", listId: "l9" })).origin).toEqual({
      docId: "doc-2",
      listId: "l9",
    });
  });

  test("duration is dropped without a `when`", () => {
    expect(toClipItem(item({ duration: 30 }))).toEqual({
      text: "Buy milk",
      state: "backlog",
      origin,
    });
  });

  test("carries the place whole", () => {
    const place = {
      label: "Luigi's",
      lat: -33.8688,
      lon: 151.2093,
      address: "1 George St",
      ref: "osm:node/1",
    };
    expect(toClipItem(item({ place }))).toEqual({
      text: "Buy milk",
      state: "backlog",
      place,
      origin,
    });
  });
});

describe("renderClip", () => {
  const items: ClipItem[] = [
    { text: "Buy milk", state: "backlog" },
    { text: 'Fix <b>"quotes"</b> & amps', notes: "n", state: "todo" },
  ];

  test("plain text is bare titles, one per line", () => {
    expect(renderClip(items).text).toBe('Buy milk\nFix <b>"quotes"</b> & amps');
  });

  test("html is a list with the payload escaped into an attribute", () => {
    const { html, json } = renderClip(items);
    expect(html.startsWith('<ul data-monoplan="')).toBe(true);
    expect(html).toContain("<li>Buy milk</li>");
    expect(html).toContain("<li>Fix &lt;b&gt;&quot;quotes&quot;&lt;/b&gt; &amp; amps</li>");
    expect(html).not.toContain("<b>");
    expect(JSON.parse(json)).toEqual({ v: 1, items });
  });

  test("the html payload round-trips", () => {
    expect(clipFromHtml(renderClip(items).html)).toEqual(items);
  });

  test("a sanitiser re-serialising the attribute still round-trips", () => {
    const { html } = renderClip(items);
    // Numeric entities for the quotes, as some serialisers emit.
    const rewritten = html.replace(/&quot;/g, "&#34;");
    expect(clipFromHtml(rewritten)).toEqual(items);
  });
});

describe("readClip", () => {
  const items: ClipItem[] = [{ text: "A", state: "review", when: "2026-10-02" }];

  test("prefers the custom type", () => {
    const { json, html } = renderClip(items);
    const other = renderClip([{ text: "B", state: "backlog" }]).html;
    expect(readClip(transfer({ [ITEM_CLIP_TYPE]: json, "text/html": other }))).toEqual(
      items,
    );
    expect(readClip(transfer({ "text/html": html }))).toEqual(items);
  });

  test("foreign html and plain text give no payload", () => {
    expect(readClip(transfer({ "text/html": "<ul><li>A</li></ul>" }))).toBeNull();
    expect(readClip(transfer({ "text/plain": "A\nB" }))).toBeNull();
    expect(readClip(null)).toBeNull();
  });

  test("rejects malformed or foreign-version payloads", () => {
    expect(readClip(transfer({ [ITEM_CLIP_TYPE]: "{not json" }))).toBeNull();
    expect(
      readClip(transfer({ [ITEM_CLIP_TYPE]: JSON.stringify({ v: 2, items: [] }) })),
    ).toBeNull();
    expect(
      readClip(
        transfer({ [ITEM_CLIP_TYPE]: JSON.stringify({ v: 1, items: [{ text: 3 }] }) }),
      ),
    ).toBeNull();
  });

  test("normalises loose field values", () => {
    const raw = JSON.stringify({
      v: 1,
      items: [
        { text: "A", state: "bogus", notes: "", duration: 15 },
        { text: "B", state: "done", when: "2026-10-02", duration: 1.5 },
        { text: "C", state: "cancelled" },
      ],
    });
    expect(readClip(transfer({ [ITEM_CLIP_TYPE]: raw }))).toEqual([
      { text: "A", state: "backlog" },
      { text: "B", state: "done", when: "2026-10-02" },
      { text: "C", state: "cancelled" },
    ]);
  });

  test("origin round-trips, and a malformed origin is dropped", () => {
    const raw = JSON.stringify({
      v: 1,
      items: [
        { text: "A", state: "backlog", origin: { docId: "d", listId: "l", x: 1 } },
        { text: "B", state: "backlog", origin: { docId: "d" } },
        { text: "C", state: "backlog", origin: { docId: "", listId: "l" } },
        { text: "D", state: "backlog", origin: "inbox" },
        { text: "E", state: "backlog", origin: null },
      ],
    });
    expect(readClip(transfer({ [ITEM_CLIP_TYPE]: raw }))).toEqual([
      { text: "A", state: "backlog", origin: { docId: "d", listId: "l" } },
      { text: "B", state: "backlog" },
      { text: "C", state: "backlog" },
      { text: "D", state: "backlog" },
      { text: "E", state: "backlog" },
    ]);
  });

  test("places round-trip, and a malformed place is dropped", () => {
    const good = { label: "Gym", lat: 51.5, lon: -0.12, address: "London" };
    const labelOnly = { label: "Home" };
    const raw = JSON.stringify({
      v: 1,
      items: [
        { text: "A", state: "backlog", place: good },
        { text: "B", state: "backlog", place: labelOnly },
        { text: "C", state: "backlog", place: { label: "" } },
        { text: "D", state: "backlog", place: { label: "Half", lat: 1 } },
        { text: "E", state: "backlog", place: { label: "Far", lat: 91, lon: 0 } },
        { text: "F", state: "backlog", place: "Luigi's" },
        { text: "G", state: "backlog", place: { label: "Extra", bogus: 1, ref: "" } },
      ],
    });
    expect(readClip(transfer({ [ITEM_CLIP_TYPE]: raw }))).toEqual([
      { text: "A", state: "backlog", place: good },
      { text: "B", state: "backlog", place: labelOnly },
      { text: "C", state: "backlog" },
      { text: "D", state: "backlog" },
      { text: "E", state: "backlog" },
      { text: "F", state: "backlog" },
      { text: "G", state: "backlog", place: { label: "Extra" } },
    ]);
  });
});

describe("closedLogLine", () => {
  // Local-time constructor, so the expectation holds in any timezone.
  const at = new Date(2026, 2, 7, 12, 34).getTime();

  test("done item is a ticked task line stamped in local ISO time", () => {
    expect(
      closedLogLine({ text: "Replace minio with rustfs", state: "done", lifecycleAt: at }),
    ).toBe("- [x] (2026-03-07 12:34) Replace minio with rustfs");
  });

  test("cancelled item is marked [-]", () => {
    expect(closedLogLine({ text: "Old idea", state: "cancelled", lifecycleAt: at })).toBe(
      "- [-] (2026-03-07 12:34) Old idea",
    );
  });

  test("renderClip uses the lines for plain text only", () => {
    const items: ClipItem[] = [{ text: "A", state: "done" }];
    const p = renderClip(items, ["- [x] (2026-03-07 12:34) A"]);
    expect(p.text).toBe("- [x] (2026-03-07 12:34) A");
    expect(p.html).toContain("<li>A</li>");
  });
});
