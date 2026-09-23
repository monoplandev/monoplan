import { describe, expect, test } from "bun:test";

import {
  clipFromHtml,
  ITEM_CLIP_TYPE,
  readClip,
  renderClip,
  toClipItem,
  type ClipItem,
} from "../src/itemClip.ts";
import type { ItemView } from "../src/sync/store.ts";

const item = (over: Partial<ItemView>): ItemView => ({
  id: "i1",
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

describe("toClipItem", () => {
  test("carries only the clone-defining fields, dropping empties", () => {
    expect(toClipItem(item({}))).toEqual({ text: "Buy milk", state: "backlog" });
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
    });
  });

  test("duration is dropped without a `when`", () => {
    expect(toClipItem(item({ duration: 30 }))).toEqual({
      text: "Buy milk",
      state: "backlog",
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
      ],
    });
    expect(readClip(transfer({ [ITEM_CLIP_TYPE]: raw }))).toEqual([
      { text: "A", state: "backlog" },
      { text: "B", state: "done", when: "2026-10-02" },
    ]);
  });
});
