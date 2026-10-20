import { describe, expect, test } from "bun:test";
import {
  captureInviteFromLocation,
  clearPendingInvite,
  pendingInvite,
  type Address,
  type KeyStore,
} from "../src/invite.ts";
import { inviteHash, inviteUrl, parseHash, parseInviteHash } from "../src/url.ts";

const SECRET = "0".repeat(63) + "a";

function memStore(): KeyStore & { map: Map<string, string> } {
  const map = new Map<string, string>();
  return {
    map,
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => void map.set(k, v),
    removeItem: (k) => void map.delete(k),
  };
}

function address(hash: string): Address & { replaced: string[] } {
  const replaced: string[] = [];
  return {
    hash,
    pathname: "/",
    search: "?x=1",
    replaced,
    replace: (url) => replaced.push(url),
  };
}

describe("invite links", () => {
  test("the invite token is its own grammar, not a route", () => {
    expect(parseInviteHash(inviteHash(SECRET))).toBe(SECRET);
    expect(parseInviteHash(`invite_${SECRET}`)).toBe(SECRET);
    expect(parseHash(inviteHash(SECRET))).toBeNull();
    // Wrong length, wrong alphabet, wrong prefix.
    expect(parseInviteHash(`#invite_${SECRET.slice(1)}`)).toBeNull();
    expect(parseInviteHash(`#invite_${"g".repeat(64)}`)).toBeNull();
    expect(parseInviteHash(`#item_${SECRET}`)).toBeNull();
    expect(inviteUrl(SECRET, "https://m.example/")).toBe(`https://m.example/#invite_${SECRET}`);
  });

  test("capture parks the secret and strips the fragment, keeping the query", () => {
    const store = memStore();
    const addr = address(inviteHash(SECRET));
    expect(captureInviteFromLocation(addr, store)).toBe(true);
    expect(pendingInvite(store)).toBe(SECRET);
    expect(addr.replaced).toEqual(["/?x=1"]);
    clearPendingInvite(store);
    expect(pendingInvite(store)).toBeNull();
  });

  test("an ordinary hash is left alone", () => {
    const store = memStore();
    const addr = address("#inbox");
    expect(captureInviteFromLocation(addr, store)).toBe(false);
    expect(pendingInvite(store)).toBeNull();
    expect(addr.replaced).toEqual([]);
  });

  test("a storage that throws falls back to memory for this page load", () => {
    const throwing: KeyStore = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
      removeItem: () => {
        throw new Error("blocked");
      },
    };
    const addr = address(inviteHash(SECRET));
    expect(captureInviteFromLocation(addr, throwing)).toBe(true);
    expect(pendingInvite(throwing)).toBe(SECRET);
    clearPendingInvite(throwing);
    expect(pendingInvite(throwing)).toBeNull();
  });
});
