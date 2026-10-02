import { describe, expect, test } from "bun:test";
import {
  itemHash,
  parseHash,
  parseInternalUrl,
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
    // Leading `#` is optional.
    expect(parseHash(`item_${ID}`)).toEqual({ kind: "item", id: ID });
  });

  test("rejects malformed and unknown tokens", () => {
    expect(parseHash("")).toBeNull();
    expect(parseHash("#")).toBeNull();
    expect(parseHash("#home")).toBeNull();
    expect(parseHash("#item_")).toBeNull();
    expect(parseHash("#item_abc")).toBeNull();
    expect(parseHash(`#item_${ID.toUpperCase()}`)).toBeNull();
    expect(parseHash(`#item_${ID}x`)).toBeNull();
    expect(parseHash(`#item_${ID}:${OTHER}`)).toBeNull();
    expect(parseHash("#list_")).toBeNull();
    expect(parseHash("#list_main")).toBeNull();
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
