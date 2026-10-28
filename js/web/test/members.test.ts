// Member directory (`src/sync/members.ts`) and the avatar initial
// (`src/Avatar.tsx`): the pieces that turn a stamped `createdBy` account
// id into a person on a shared doc's rows.

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";
import type { Member } from "../src/api.ts";
import { avatarInitial } from "../src/Avatar.tsx";
import { createMemberDirectory } from "../src/sync/members.ts";

const DOC = "00000000-0000-0000-0000-00000000000b";

function member(account_id: string, email: string): Member {
  return { account_id, email, role: "member", added_at: 0 };
}

const tick = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("member directory", () => {
  test("resolves an account id to its email once the doc's list loads", async () => {
    await createRoot(async (dispose) => {
      const calls: string[] = [];
      const dir = createMemberDirectory(async (docId) => {
        calls.push(docId);
        return [member("a1", "ann@example.com"), member("b2", "bob@example.com")];
      });
      expect(dir.emailOf(DOC, "a1")).toBeUndefined();
      dir.ensure(DOC);
      dir.ensure(DOC); // in flight: no second fetch
      await tick();
      expect(dir.emailOf(DOC, "a1")).toBe("ann@example.com");
      expect(dir.emailOf(DOC, "b2")).toBe("bob@example.com");
      expect(dir.emailOf(DOC, "zz")).toBeUndefined();
      dir.ensure(DOC); // loaded: no refetch
      await tick();
      expect(calls).toEqual([DOC]);
      dispose();
    });
  });

  test("a failed load is forgotten so the next ensure retries", async () => {
    await createRoot(async (dispose) => {
      let fail = true;
      let calls = 0;
      const dir = createMemberDirectory(async () => {
        calls++;
        if (fail) throw new Error("offline");
        return [member("a1", "ann@example.com")];
      });
      dir.ensure(DOC);
      await tick();
      expect(dir.emailOf(DOC, "a1")).toBeUndefined();
      fail = false;
      dir.ensure(DOC);
      await tick();
      expect(dir.emailOf(DOC, "a1")).toBe("ann@example.com");
      expect(calls).toBe(2);
      dispose();
    });
  });

  test("refresh replaces a doc's list and leaves other docs alone", async () => {
    await createRoot(async (dispose) => {
      const OTHER = "00000000-0000-0000-0000-00000000000c";
      let list = [member("a1", "ann@example.com")];
      const dir = createMemberDirectory(async (docId) =>
        docId === OTHER ? [member("c3", "cat@example.com")] : list,
      );
      await dir.refresh(DOC);
      await dir.refresh(OTHER);
      list = [member("a1", "ann@new.example.com")];
      await dir.refresh(DOC);
      expect(dir.emailOf(DOC, "a1")).toBe("ann@new.example.com");
      expect(dir.emailOf(OTHER, "c3")).toBe("cat@example.com");
      dispose();
    });
  });
});

describe("avatarInitial", () => {
  test("first letter of the email, upper-cased", () => {
    expect(avatarInitial("daniel@example.com")).toBe("D");
    expect(avatarInitial("  émile@example.com")).toBe("É");
  });
  test("whole first code point, not half a surrogate pair", () => {
    expect(avatarInitial("😀x@example.com")).toBe("😀");
  });
  test("unknown person renders a question mark", () => {
    expect(avatarInitial(null)).toBe("?");
    expect(avatarInitial("")).toBe("?");
    expect(avatarInitial("   ")).toBe("?");
  });
});
