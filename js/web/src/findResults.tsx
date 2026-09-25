// Shared machinery for the Find surfaces: the desktop palette
// (FindPalette.tsx) and the mobile switcher sheet (FindSheet.tsx). Both
// are thin views over the local SearchEngine (spec/search.md); this
// module owns what they have in common — the debounced query, the
// result-set policy (built-in view synthesis, the empty-query default
// menu, lists floated above items) and the row's leading-glyph anatomy —
// so each surface is left holding only its own interaction model.

import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import type { DocApp } from "./sync/store.ts";
import { matchesName, type SearchResult } from "./search.ts";
import { useAppI18n } from "./i18n.tsx";
import archiveSvg from "./icons/archive.svg?raw";
import calendarSvg from "./icons/calendar.svg?raw";
import checkSvg from "./icons/check.svg?raw";
import crumpledPaperSvg from "./icons/crumpled-paper.svg?raw";
import drawingPinSvg from "./icons/drawing-pin.svg?raw";
import fileSvg from "./icons/file.svg?raw";

// The built-in views (Focus / Upcoming / Done / Bin / Inbox) aren't
// `ListMeta` rows, so the search engine never indexes them
// (spec/search.md "Palette-level entries"). The surfaces synthesize them:
// present in the default (empty-query) menu, and name-matched into query
// results so "foc" finds Focus the way it finds a user list. Bin follows
// the sidebar's rule and only exists while it holds items.
export type ViewResultId = "focus" | "inbox" | "upcoming" | "done" | "bin";
export interface ViewResult {
  kind: "view";
  id: ViewResultId;
  title: string;
}
export type FindResult = SearchResult | ViewResult;

/** A fixed nav view (Focus / Upcoming / Done / Bin) as opposed to Inbox, which
 *  sits with the lists. Lets a surface draw the sidebar's group break. */
export function isFixedView(item: FindResult): boolean {
  return item.kind === "view" && item.id !== "inbox";
}

// Fixed nav icons, mirrored so a built-in reads the same here as in the
// sidebar.
const VIEW_ICONS: Record<ViewResultId, string> = {
  focus: drawingPinSvg,
  inbox: archiveSvg,
  upcoming: calendarSvg,
  done: checkSvg,
  bin: crumpledPaperSvg,
};

/** Query state + result set for a Find surface. `setInput` feeds the
 *  debounced filter; `reset` clears both at once so a reopen never
 *  flashes the previous query's results. */
export function createFindState(app: DocApp) {
  const { m } = useAppI18n();
  const [input, setInput] = createSignal("");
  const [filter, setFilter] = createSignal("");

  // Debounced search. Sub-frame human latency tolerance — keeps us off
  // the tokenize/postings hot path on every keystroke without an
  // observable input lag.
  createEffect(() => {
    const value = input().trim();
    const timer = window.setTimeout(() => setFilter(value), 100);
    onCleanup(() => window.clearTimeout(timer));
  });

  // Localized labels re-derive when the language changes; sidebar order:
  // the fixed views first (Bin only while non-empty, as in the nav), then
  // Inbox heading the lists group (it's the reserved primary list, filed
  // with the user's lists in the nav).
  const builtinViews = (): ViewResult[] => [
    { kind: "view", id: "focus", title: m().nav.focus },
    { kind: "view", id: "upcoming", title: m().nav.upcoming },
    { kind: "view", id: "done", title: m().nav.done },
    ...(app.state.binCount > 0
      ? [{ kind: "view", id: "bin", title: m().nav.bin } as ViewResult]
      : []),
    { kind: "view", id: "inbox", title: m().nav.inbox },
  ];

  // Re-run on every doc version bump too, so a peer or local mutation
  // while the surface is open updates the visible result set.
  const items = createMemo((): FindResult[] => {
    const q = filter();
    app.version();
    if (!q) {
      // Default menu: the built-in views then every active list, in nav
      // order — the surface doubles as a jump-to-view switcher before the
      // user types anything. Archived lists stay reachable by query only.
      const lists: SearchResult[] = [];
      for (const id of app.state.listsOrder) {
        const l = app.state.listsById[id];
        if (!l || l.archivedAt != null) continue;
        lists.push({ id: l.id, kind: "list", title: l.name, score: 0 });
      }
      return [...builtinViews(), ...lists];
    }
    // Built-ins match by name like a list result would (shared tokenizer
    // semantics via matchesName) and sit above everything — they're the
    // pinned nav entries.
    const views = builtinViews().filter((v) => matchesName(v.title, q));
    const results = app.search.query(q, 50);
    // Float list results above items, preserving each group's relevance
    // order (Array.sort is stable). The surface owns this presentation
    // tweak — the engine's ranking is left untouched.
    const sorted = results
      .slice()
      .sort((a, b) => (a.kind === "list" ? 0 : 1) - (b.kind === "list" ? 0 : 1));
    return [...views, ...sorted];
  });

  const reset = () => {
    setInput("");
    setFilter("");
  };

  return { input, setInput, reset, items };
}

/** Display name of the list an item lives in, for the right-hand
 *  column. The reserved `inbox` list isn't a `ListMeta` row — it always
 *  renders the localized built-in label. Lists themselves get no label.
 *  Returns "" when there's nothing to show. */
export function findResultListLabel(
  app: DocApp,
  inboxLabel: string,
  item: FindResult,
): string {
  if (item.kind !== "item") return "";
  const listId = item.listId;
  if (!listId) return "";
  if (listId === "inbox") return inboxLabel;
  return app.state.listsById[listId]?.name ?? "";
}

/** Whether a result denotes, or lives in, an archived list
 *  (`spec/data-model.md` "Archived lists"). Archived lists stay indexed
 *  and keep labelling their items, so both surfaces flag the row: a
 *  list result that is itself archived, or an item whose owning list
 *  is. Built-in views and the reserved `inbox` can't be archived. */
export function findResultArchived(app: DocApp, item: FindResult): boolean {
  if (item.kind === "view") return false;
  const listId = item.kind === "list" ? item.id : item.listId;
  if (!listId) return false;
  return app.state.listsById[listId]?.archivedAt != null;
}

/** Lifecycle of an item result ("" for other kinds) — union-safe access
 *  for the binned strike-through and the done checkbox mirror. */
export function findResultLifecycle(item: FindResult): string {
  return item.kind === "item" ? item.lifecycle ?? "" : "";
}

/** A result row's body: leading glyph slot, title, owning-list column.
 *  Purely presentational — the surface supplies the wrapping element and
 *  whatever selection / current-view state it has. The slot is always
 *  rendered so titles stay aligned across mixed result kinds: a checkbox
 *  mirror for items, the list's icon for lists, the fixed nav icon for
 *  built-in views. */
export function FindResultBody(props: { app: DocApp; item: FindResult }) {
  const { m } = useAppI18n();
  const lifecycle = () => findResultLifecycle(props.item);
  // Chosen icon for a list result (a literal emoji grapheme), or
  // undefined when unset — falls back to the default file glyph,
  // mirroring the nav.
  const listIcon = (): string | undefined =>
    props.item.kind === "list"
      ? props.app.state.listsById[props.item.id]?.icon
      : undefined;
  const viewIcon = (): string | undefined =>
    props.item.kind === "view" ? VIEW_ICONS[props.item.id] : undefined;
  const listLabel = () => findResultListLabel(props.app, m().nav.inbox, props.item);
  const archived = () => findResultArchived(props.app, props.item);
  return (
    <>
      <Show
        when={props.item.kind !== "item"}
        fallback={
          <span
            class="task-check palette__item-check"
            data-kind={props.item.kind}
            data-checked={
              lifecycle() === "done" || lifecycle() === "cancelled" ? "" : undefined
            }
            data-cancelled={lifecycle() === "cancelled" ? "" : undefined}
            aria-hidden="true"
          />
        }
      >
        <Show
          when={viewIcon()}
          fallback={
            <Show
              when={listIcon()}
              fallback={
                <span
                  class="palette__item-icon"
                  innerHTML={fileSvg}
                  aria-hidden="true"
                />
              }
            >
              {(icon) => (
                <span
                  class="palette__item-icon palette__item-icon-emoji"
                  aria-hidden="true"
                >
                  {icon()}
                </span>
              )}
            </Show>
          }
        >
          {(svg) => (
            <span class="palette__item-icon" innerHTML={svg()} aria-hidden="true" />
          )}
        </Show>
      </Show>
      <span class="palette__item-name">{props.item.title}</span>
      {/* Archived marker sits between the title and the owning-list
          column: "Task … Archived Work" for an item, "Work … Archived"
          for the list itself. Plain .badge, like the row badges. */}
      <Show when={archived()}>
        <span class="badge">{m().nav.archived}</span>
      </Show>
      <Show when={listLabel()}>
        {(label) => <span class="palette__item-list">{label()}</span>}
      </Show>
    </>
  );
}
