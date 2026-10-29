import { describe, expect, test } from "bun:test";
import { avatarColour, avatarInitial } from "../src/Avatar.tsx";

describe("avatarColour", () => {
  test("is deterministic and keyed on the first 8 hex chars", () => {
    const a = avatarColour("0000000a-1111-2222-3333-444444444444");
    expect(avatarColour("0000000a-ffff-0000-0000-000000000000")).toBe(a);
    expect(a).toMatch(/^#[0-9A-F]{6}$/);
  });
  test("walks the palette as the hash steps", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 10; i++) seen.add(avatarColour(i.toString(16).padStart(8, "0")));
    expect(seen.size).toBe(10);
  });
  test("never yields undefined for a non-hex id", () => {
    expect(avatarColour("not-a-uuid")).toMatch(/^#[0-9A-F]{6}$/);
    expect(avatarColour("")).toMatch(/^#[0-9A-F]{6}$/);
  });
});

describe("avatarInitial", () => {
  test("upper-cases the first code point", () => {
    expect(avatarInitial("daniel@example.com")).toBe("D");
    expect(avatarInitial("  émile")).toBe("É");
    expect(avatarInitial(null)).toBe("?");
  });
});
