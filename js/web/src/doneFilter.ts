// The Done view's filter: which workspaces and which lists its closed
// items are drawn from. Device-local like the lane prefs (a filter is
// "what am I looking at right now", not a setting to sync), kept in
// localStorage so it survives a reload.
//
// Two facets, ANDed. An empty facet filters nothing, so the default (both
// empty) is every open doc's closed items. Stored ids are kept verbatim
// and narrowed against what exists at read time (`effectiveDoneFilter`):
// a doc that opens late, or a list that syncs in after boot, comes back
// checked rather than having been pruned on an early read.

export interface DoneFilter {
  /** Checked workspaces, by doc id. Empty ≡ all. */
  docs: string[];
  /** Checked lists, by `doneListKey`. Empty ≡ all. */
  lists: string[];
}

/** One list offered by the filter. `inbox` / `events` exist in every
 *  doc, so a list is identified by doc and id together. */
export interface DoneListOption {
  key: string;
  docId: string;
  listId: string;
  name: string;
  /** Name of the list's shared workspace; absent for Personal. */
  workspace?: string;
}

export const EMPTY_DONE_FILTER: DoneFilter = { docs: [], lists: [] };

export const doneListKey = (docId: string, listId: string): string => `${docId}/${listId}`;

const DONE_FILTER_KEY = "monoplan:done-filter";

const stringList = (v: unknown): string[] =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];

export function loadDoneFilter(): DoneFilter {
  try {
    const raw = localStorage.getItem(DONE_FILTER_KEY);
    const parsed = raw ? (JSON.parse(raw) as { docs?: unknown; lists?: unknown } | null) : null;
    if (!parsed || typeof parsed !== "object") return EMPTY_DONE_FILTER;
    return { docs: stringList(parsed.docs), lists: stringList(parsed.lists) };
  } catch {
    return EMPTY_DONE_FILTER;
  }
}

export function saveDoneFilter(filter: DoneFilter): void {
  try {
    if (filter.docs.length === 0 && filter.lists.length === 0) {
      localStorage.removeItem(DONE_FILTER_KEY);
    } else {
      localStorage.setItem(DONE_FILTER_KEY, JSON.stringify(filter));
    }
  } catch {
    // Private mode / quota: the filter just lasts the session.
  }
}

/** `id` flipped in `ids`. */
export const toggled = (ids: readonly string[], id: string): string[] =>
  ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];

/** The checked workspaces that are actually open. */
export function effectiveDoneDocs(filter: DoneFilter, openDocIds: readonly string[]): string[] {
  return filter.docs.filter((id) => openDocIds.includes(id));
}

/** The lists the filter offers: every list of the checked workspaces, or
 *  of all of them when none is checked. */
export function offeredDoneLists(
  docs: readonly string[],
  all: readonly DoneListOption[],
): DoneListOption[] {
  return docs.length === 0 ? [...all] : all.filter((l) => docs.includes(l.docId));
}

/** The filter narrowed to what exists: open docs, and checked lists that
 *  are on offer (so a list of an unchecked workspace doesn't empty the
 *  view from behind a submenu that no longer shows it). */
export function effectiveDoneFilter(
  filter: DoneFilter,
  openDocIds: readonly string[],
  allLists: readonly DoneListOption[],
): DoneFilter {
  const docs = effectiveDoneDocs(filter, openDocIds);
  const offered = new Set(offeredDoneLists(docs, allLists).map((l) => l.key));
  return { docs, lists: filter.lists.filter((k) => offered.has(k)) };
}

/** Predicate for an *effective* filter. */
export function doneFilterAdmits(
  filter: DoneFilter,
): (it: { docId: string; listId: string }) => boolean {
  const docs = filter.docs.length > 0 ? new Set(filter.docs) : null;
  const lists = filter.lists.length > 0 ? new Set(filter.lists) : null;
  return (it) =>
    (docs === null || docs.has(it.docId)) &&
    (lists === null || lists.has(doneListKey(it.docId, it.listId)));
}
