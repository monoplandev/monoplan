// The combined Inbox's workspace filter (`spec/inbox-plan.md` "One
// Inbox"): the doc ids picked, or none for "All workspaces", the
// default. Several can be picked at once. Device-local UI state, so
// localStorage, like `agendaMode.ts`, and a signal so the nav count,
// the header and the rows follow a change. Which workspaces are shown,
// given the open ones, is decided in Rust (`inboxScope` in the wasm,
// `monoplan_core::inbox::scope`): a filter naming no open workspace
// reads as all of them, and is kept so a workspace that comes back is
// filtered again.

import { createSignal } from "solid-js";

const KEY = "monoplan.inbox.scope";

function load(): string[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    // Unparseable or no storage: all workspaces.
    return [];
  }
}

const [filter, setFilter] = createSignal<readonly string[]>(load());

/** The doc ids the Inbox is filtered to; empty is every workspace. */
export const inboxFilter = filter;

export function setInboxFilter(docIds: readonly string[]): void {
  setFilter([...docIds]);
  try {
    if (docIds.length === 0) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, JSON.stringify(docIds));
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
}
