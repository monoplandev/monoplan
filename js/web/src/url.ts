// Fragment URLs for items, lists and the built-in views (`spec/urls.md`).
//
// The whole address lives in the hash so the server never sees an item
// or list id (ids only ever exist inside encrypted update blobs) and no
// server route is needed. The grammar is a single self-describing
// token, safe to paste into a shell or a chat:
//
//   "#item_" id | "#upcoming_" id | "#list_" id | "#inbox" | "#inbox_" doc
//   | "#focus" | "#upcoming" | "#done" | "#bin" | "#menu"
//   id  = [0-9a-f]{32}
//   doc = [0-9a-f-]{32,36}      -- a server doc id (uuid, hyphenated)
//
// `#upcoming_<id>` is the calendar anchored on an item: the same item
// as `#item_<id>`, opened over the agenda instead of its home list, for
// a jump from an agenda-shaped surface (the macOS tray's Today rows).
// The address bar canonicalises it to `#item_<id>` once applied.
//
// One more token is not a route: "#invite_" secret (secret = [0-9a-f]{64})
// is a sharing link (`spec/sharing-plan.md` §Invites). `parseHash` does
// not return it; `parseInviteHash` does, and the app consumes and strips
// it before the router ever runs (`invite.ts`), so the secret is never
// mirrored back into the address bar or history.
//
// An item URL names only the item: the list and doc are looked up
// client-side across every open doc, so a link keeps working after the
// item moves. A user list's id is unique across docs too, so `#list_`
// carries no doc either; only another doc's reserved Inbox needs one
// (`#inbox_<doc>`), since every doc has an `inbox` (`spec/sharing-plan.md`).
//
// `#menu` is the phone's menu page (`FindSheet`), a page of its own in
// the mobile shell so Back (and the swipe gesture) leave it the way a
// tap on a row does. Desktop has no such page: there the token is
// ignored and the address bar rewritten to the view on screen.

import type { ViewKey } from "./prefs.ts";

export type Route =
  | { kind: "view"; view: ViewKey }
  /** `on` names the view to open the item over instead of its home:
   *  the agenda, for `#upcoming_<id>`. Absent for a plain item link. */
  | { kind: "item"; id: string; on?: "upcoming" }
  | { kind: "menu" };

/** The menu page's hash (mobile shell only). */
export const MENU_HASH = "#menu";

const ID_RE = /^[0-9a-f]{32}$/;
const DOC_RE = /^[0-9a-f-]{32,36}$/;

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
    case "menu":
      return { kind: "menu" };
  }
  if (token.startsWith("item_")) {
    const id = token.slice("item_".length);
    return ID_RE.test(id) ? { kind: "item", id } : null;
  }
  // The agenda anchored on an item. A day anchor (`upcoming_2026-07-13`,
  // reserved in `spec/calendar-plan.md`) is not an id and stays ignored.
  if (token.startsWith("upcoming_")) {
    const id = token.slice("upcoming_".length);
    return ID_RE.test(id) ? { kind: "item", id, on: "upcoming" } : null;
  }
  if (token.startsWith("list_")) {
    const id = token.slice("list_".length);
    return ID_RE.test(id) ? { kind: "view", view: { kind: "list", id } } : null;
  }
  if (token.startsWith("inbox_")) {
    const docId = token.slice("inbox_".length);
    return DOC_RE.test(docId)
      ? { kind: "view", view: { kind: "list", id: "inbox", docId } }
      : null;
  }
  return null;
}

const INVITE_SECRET_RE = /^[0-9a-f]{64}$/;

/** The 64-char hex secret of a sharing link hash (`#invite_<secret>`),
 *  or null for anything else. */
export function parseInviteHash(hash: string): string | null {
  const token = hash.startsWith("#") ? hash.slice(1) : hash;
  if (!token.startsWith("invite_")) return null;
  const secret = token.slice("invite_".length);
  return INVITE_SECRET_RE.test(secret) ? secret : null;
}

export function inviteHash(secretHex: string): string {
  return `#invite_${secretHex}`;
}

/** The sharing link to hand out: the app's base plus the secret in the
 *  fragment, which the browser never sends to the server. */
export function inviteUrl(secretHex: string, base: string = appBase()): string {
  return base + inviteHash(secretHex);
}

/** Canonical hash (leading `#` included) for a view. A list view with
 *  no `docId` is a Home list; another doc's Inbox names its doc. */
export function viewHash(view: ViewKey): string {
  if (view.kind !== "list") return `#${view.kind}`;
  if (view.id !== "inbox") return `#list_${view.id}`;
  return view.docId === undefined ? "#inbox" : `#inbox_${view.docId}`;
}

/** Canonical hash for an item. */
export function itemHash(id: string): string {
  return `#item_${id}`;
}

/** Hash for an item opened over the calendar rather than its home list
 *  (`#upcoming_<id>`). What the macOS tray's Today rows send. */
export function agendaItemHash(id: string): string {
  return `#upcoming_${id}`;
}

/** True when `hash` names `id` in either item form, so the address bar
 *  mirror can tell a context form of the open item from a stale hash. */
export function namesItem(hash: string, id: string): boolean {
  const r = parseHash(hash);
  return r?.kind === "item" && r.id === id;
}

/** The hash the address bar should show for a given workspace state: the
 *  menu page (mobile) wins over everything, then an open item over the
 *  view behind it. */
export function stateHash(view: ViewKey, openItemId: string | null, menu = false): string {
  if (menu) return MENU_HASH;
  return openItemId !== null ? itemHash(openItemId) : viewHash(view);
}

/** Origin + path of the running app, the base every shareable URL is
 *  built on. The hash is the only part that varies. In the browser that
 *  is the page's own address; the macOS app's window overrides it with
 *  the server's (`setLinkBase`), since a `monoplan://app/mirror.html`
 *  address opens nowhere but that window. */
export function appBase(): string {
  return linkBase ?? `${location.origin}${location.pathname}`;
}

let linkBase: string | null = null;

/** Make every shareable link (and the internal-link test) use `base`
 *  instead of the page's address. `base` is an origin + path, e.g.
 *  `https://alpha.monoplan.app/`; null restores the page's own. */
export function setLinkBase(base: string | null): void {
  linkBase = base;
}

/** The link base for a server the web client is served from: its
 *  origin with the root path, whatever the configured URL's trailing
 *  slash or path. */
export function serverLinkBase(serverUrl: string): string {
  return `${new URL(serverUrl).origin}/`;
}

export function itemUrl(id: string): string {
  return appBase() + itemHash(id);
}

export function listUrl(id: string, docId?: string): string {
  return appBase() + viewHash(docId === undefined ? { kind: "list", id } : { kind: "list", id, docId });
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
