import { describe, expect, test } from "bun:test";
import {
  agendaItemHash,
  appBase,
  itemHash,
  namesItem,
  itemUrl,
  listUrl,
  parseHash,
  parseInternalUrl,
  serverLinkBase,
  setLinkBase,
  stateHash,
  viewHash,
} from "../src/url.ts";

const ID = "0192f5c1a3b74e6c9d2f8a1b3c4d5e6f";
const OTHER = "0192f5c1a3b74e6c9d2f8a1b3c4d5e70";

describe("hash routes", () => {
  test("parses every token", () => {
    expect(parseHash("#inbox")).toEqual({ kind: "view", view: { kind: "list", id: "inbox" } });
    expect(parseHash("#list_inbox")).toEqual({ kind: "view", view: { kind: "list", id: "inbox" } });
    // The reserved `events` list's home is the calendar.
    expect(parseHash("#list_events")).toEqual({ kind: "view", view: { kind: "upcoming" } });
    expect(parseHash("#focus")).toEqual({ kind: "view", view: { kind: "focus" } });
    expect(parseHash("#upcoming")).toEqual({ kind: "view", view: { kind: "upcoming" } });
    expect(parseHash("#done")).toEqual({ kind: "view", view: { kind: "done" } });
    expect(parseHash("#bin")).toEqual({ kind: "view", view: { kind: "bin" } });
    expect(parseHash(`#list_${ID}`)).toEqual({ kind: "view", view: { kind: "list", id: ID } });
    expect(parseHash(`#item_${ID}`)).toEqual({ kind: "item", id: ID });
    // The calendar anchored on an item: the same item, opened over the agenda.
    expect(parseHash(`#upcoming_${ID}`)).toEqual({ kind: "item", id: ID, on: "upcoming" });
    expect(parseHash(agendaItemHash(ID))).toEqual({ kind: "item", id: ID, on: "upcoming" });
    // Leading `#` is optional.
    expect(parseHash(`item_${ID}`)).toEqual({ kind: "item", id: ID });
  });

  test("rejects malformed and unknown tokens", () => {
    expect(parseHash("")).toBeNull();
    expect(parseHash("#")).toBeNull();
    expect(parseHash("#home")).toBeNull();
    expect(parseHash("#menu")).toBeNull();
    expect(parseHash("#item_")).toBeNull();
    expect(parseHash("#item_abc")).toBeNull();
    expect(parseHash(`#item_${ID.toUpperCase()}`)).toBeNull();
    expect(parseHash(`#item_${ID}x`)).toBeNull();
    expect(parseHash(`#item_${ID}:${OTHER}`)).toBeNull();
    expect(parseHash("#list_")).toBeNull();
    expect(parseHash("#list_main")).toBeNull();
    expect(parseHash("#upcoming_")).toBeNull();
    // The day anchor is reserved, not parsed (spec/calendar-plan.md).
    expect(parseHash("#upcoming_2026-07-13")).toBeNull();
    expect(parseHash(`#upcoming_${ID}x`)).toBeNull();
    expect(parseHash("#inbox_")).toBeNull();
    expect(parseHash("#inbox_not-a-doc")).toBeNull();
  });

  test("another doc's inbox names its doc; a Home list never does", () => {
    const doc = "01a15717-7622-7323-8997-af3e145454d2";
    expect(parseHash(`#inbox_${doc}`)).toEqual({
      kind: "view",
      view: { kind: "list", id: "inbox", docId: doc },
    });
    expect(viewHash({ kind: "list", id: "inbox", docId: doc })).toBe(`#inbox_${doc}`);
    // A user list's id is unique across docs, so its hash carries none.
    expect(viewHash({ kind: "list", id: ID, docId: doc })).toBe(`#list_${ID}`);
  });

  test("round-trips canonical forms", () => {
    for (const h of ["#inbox", "#focus", "#upcoming", "#done", "#bin", `#list_${ID}`]) {
      const r = parseHash(h);
      expect(r?.kind).toBe("view");
      if (r?.kind === "view") expect(viewHash(r.view)).toBe(h);
    }
    const r = parseHash(itemHash(ID));
    expect(r).toEqual({ kind: "item", id: ID });
  });

  test("inbox canonicalises to the short form", () => {
    expect(viewHash({ kind: "list", id: "inbox" })).toBe("#inbox");
  });

  test("open item wins over the view", () => {
    expect(stateHash({ kind: "focus" }, null)).toBe("#focus");
    expect(stateHash({ kind: "focus" }, ID)).toBe(`#item_${ID}`);
  });

  test("the agenda form names the same item as the canonical one", () => {
    expect(namesItem(`#item_${ID}`, ID)).toBe(true);
    expect(namesItem(`#upcoming_${ID}`, ID)).toBe(true);
    expect(namesItem(`#upcoming_${OTHER}`, ID)).toBe(false);
    expect(namesItem("#upcoming", ID)).toBe(false);
    expect(namesItem("", ID)).toBe(false);
  });
});

describe("internal links", () => {
  const base = "https://app.example/";
  test("recognises own-origin item and list links", () => {
    expect(parseInternalUrl(`${base}#item_${ID}`, base)).toEqual({ kind: "item", id: ID });
    expect(parseInternalUrl(`${base}#list_${ID}`, base)).toEqual({
      kind: "view",
      view: { kind: "list", id: ID },
    });
    expect(parseInternalUrl(`${base}#focus`, base)).toEqual({ kind: "view", view: { kind: "focus" } });
  });

  test("ignores other hosts, other paths, bare app links and junk", () => {
    expect(parseInternalUrl(`https://other.example/#item_${ID}`, base)).toBeNull();
    expect(parseInternalUrl(`${base}docs#item_${ID}`, base)).toBeNull();
    expect(parseInternalUrl(base, base)).toBeNull();
    expect(parseInternalUrl("not a url", base)).toBeNull();
  });
});

describe("link base", () => {
  test("serverLinkBase is the server's origin with the root path", () => {
    expect(serverLinkBase("https://alpha.monoplan.app")).toBe("https://alpha.monoplan.app/");
    expect(serverLinkBase("https://alpha.monoplan.app/")).toBe("https://alpha.monoplan.app/");
    expect(serverLinkBase("http://127.0.0.1:8000/api/")).toBe("http://127.0.0.1:8000/");
  });

  test("setLinkBase redirects shareable links and the internal-link test", () => {
    const base = "https://alpha.monoplan.app/";
    setLinkBase(base);
    try {
      expect(appBase()).toBe(base);
      expect(itemUrl(ID)).toBe(`${base}#item_${ID}`);
      expect(listUrl(ID)).toBe(`${base}#list_${ID}`);
      expect(listUrl("inbox", "0192f5c1-a3b7-4e6c-9d2f-8a1b3c4d5e6f")).toBe(
        `${base}#inbox_0192f5c1-a3b7-4e6c-9d2f-8a1b3c4d5e6f`,
      );
      expect(parseInternalUrl(`${base}#item_${ID}`)).toEqual({ kind: "item", id: ID });
      expect(parseInternalUrl(`monoplan://app/mirror.html#item_${ID}`)).toBeNull();
    } finally {
      setLinkBase(null);
    }
  });
});
