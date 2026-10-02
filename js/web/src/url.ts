// Fragment URLs for items, lists and the built-in views (`spec/urls.md`).
//
// The whole address lives in the hash so the server never sees an item
// or list id (ids only ever exist inside encrypted op blobs) and no
// server route is needed. The grammar is a single self-describing
// token, safe to paste into a shell or a chat:
//
//   "#item_" id | "#list_" id | "#inbox" | "#focus" | "#upcoming"
//   | "#done" | "#bin"
//   id = [0-9a-f]{32}
//
// An item URL names only the item: the list and (future) doc are looked
// up client-side, so a link keeps working after the item moves.

import type { ViewKey } from "./prefs.ts";

export type Route =
  | { kind: "view"; view: ViewKey }
  | { kind: "item"; id: string };

const ID_RE = /^[0-9a-f]{32}$/;

/** Parse a `location.hash` (with or without the leading `#`). Returns
 *  null for an empty or unrecognised hash; callers leave their state
 *  alone in that case. */
export function parseHash(hash: string): Route | null {
  const token = hash.startsWith("#") ? hash.slice(1) : hash;
  if (!token) return null;
  switch (token) {
    case "inbox":
    case "list_inbox":
      return { kind: "view", view: { kind: "list", id: "inbox" } };
    // The reserved `events` list has no view of its own: the calendar
    // is its home (`spec/events-plan.md`).
    case "list_events":
      return { kind: "view", view: { kind: "upcoming" } };
    case "focus":
    case "upcoming":
    case "done":
    case "bin":
      return { kind: "view", view: { kind: token } };
  }
  if (token.startsWith("item_")) {
    const id = token.slice("item_".length);
    return ID_RE.test(id) ? { kind: "item", id } : null;
  }
  if (token.startsWith("list_")) {
    const id = token.slice("list_".length);
    return ID_RE.test(id) ? { kind: "view", view: { kind: "list", id } } : null;
  }
  return null;
}

/** Canonical hash (leading `#` included) for a view. */
export function viewHash(view: ViewKey): string {
  if (view.kind !== "list") return `#${view.kind}`;
  return view.id === "inbox" ? "#inbox" : `#list_${view.id}`;
}

/** Canonical hash for an item. */
export function itemHash(id: string): string {
  return `#item_${id}`;
}

/** The hash the address bar should show for a given workspace state: an
 *  open item wins over the view behind it. */
export function stateHash(view: ViewKey, openItemId: string | null): string {
  return openItemId !== null ? itemHash(openItemId) : viewHash(view);
}

/** Origin + path of the running app, the base every shareable URL is
 *  built on. The hash is the only part that varies. */
export function appBase(): string {
  return `${location.origin}${location.pathname}`;
}

export function itemUrl(id: string): string {
  return appBase() + itemHash(id);
}

export function listUrl(id: string): string {
  return appBase() + viewHash({ kind: "list", id });
}

/** If `href` is one of this app's own URLs (same origin and path) with a
 *  recognised hash, the route it names; else null. Used by the note
 *  linkifier to turn pasted item links into in-app jumps. */
export function parseInternalUrl(href: string, base: string = appBase()): Route | null {
  let u: URL;
  try {
    u = new URL(href);
  } catch {
    return null;
  }
  if (`${u.origin}${u.pathname}` !== base) return null;
  return parseHash(u.hash);
}
