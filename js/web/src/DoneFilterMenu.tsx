// The Done view's header filter: a funnel-lines trigger opening a menu of
// two submenus, Workspaces and Lists, each a set of checkboxes. The state
// and its semantics live in `doneFilter.ts`; this is the surface.
//
// The Lists submenu leads with a text box that narrows the rows. Archived
// lists are left out until the box has a query (or they are checked), and
// carry an Archived badge when shown (`shownDoneLists`). A text
// field inside a Kobalte menu fights the menu's own keyboard model
// (typeahead on printable keys, focus following the pointer), so:
//   - keys typed in the box stop there, except the arrows (which step into
//     the rows) and Escape;
//   - while the box has focus, hovering a row does not pull focus off it
//     (CSS `:hover` carries the highlight instead);
//   - a printable key or Backspace on a focused row goes back to the box,
//     so typing always narrows rather than type-selecting.

import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { Tooltip } from "@kobalte/core/tooltip";
import { createMemo, createSignal, For, Show } from "solid-js";
import { shownDoneLists, type DoneListOption } from "./doneFilter.ts";
import { useAppI18n } from "./i18n.tsx";
import checkSvg from "./icons/check.svg?raw";
import filterSvg from "./icons/filter.svg?raw";
import usersSvg from "./icons/users.svg?raw";
import { createPopoverTooltipGuard } from "./popoverTooltip.ts";
import { matchesName } from "./search.ts";

export function DoneFilterMenu(props: {
  /** Every open doc, Home first, with its shown name. `shared` is any
   *  non-Home doc, marked with the people glyph as in the nav and the
   *  move palette. The Workspaces submenu is only offered when there is
   *  more than one. */
  workspaces: { docId: string; label: string; shared: boolean }[];
  /** The lists on offer (those of the checked workspaces). */
  lists: DoneListOption[];
  docChecked: (docId: string) => boolean;
  toggleDoc: (docId: string) => void;
  listChecked: (key: string) => boolean;
  toggleList: (key: string) => void;
  /** How many checks are in effect, per facet (0 ≡ unfiltered). */
  docCount: number;
  listCount: number;
  clear: () => void;
}) {
  const { m } = useAppI18n();
  const guard = createPopoverTooltipGuard();
  const [query, setQuery] = createSignal("");
  let inputEl: HTMLInputElement | undefined;
  const active = () => props.docCount + props.listCount > 0;
  const shownLists = createMemo(() =>
    shownDoneLists(props.lists, query().trim(), props.listChecked, matchesName),
  );
  return (
    <DropdownMenu {...guard.popover} placement="bottom-end" gutter={6}>
      <Tooltip {...guard.tooltip} openDelay={200} closeDelay={0} placement="bottom">
        <Tooltip.Trigger
          as={DropdownMenu.Trigger}
          class="icon-button done-filter-trigger"
          tabIndex={-1}
          aria-label={m().workspace.filter}
          data-filtered={active() ? "" : undefined}
          innerHTML={filterSvg}
        />
        <Tooltip.Portal>
          <Tooltip.Content class="tooltip-content">
            {m().workspace.filter}
            <Tooltip.Arrow />
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip>
      <DropdownMenu.Portal>
        <DropdownMenu.Content {...guard.content} class="dropdown-menu-content">
          <Show when={props.workspaces.length > 1}>
            <DropdownMenu.Sub gutter={4}>
              <DropdownMenu.SubTrigger class="dropdown-menu-item">
                <span>{m().workspace.filterWorkspaces}</span>
                <SubTriggerTail count={props.docCount} />
              </DropdownMenu.SubTrigger>
              <DropdownMenu.Portal>
                <DropdownMenu.SubContent class="dropdown-menu-content done-filter-sub">
                  <For each={props.workspaces}>
                    {(w) => (
                      <DropdownMenu.CheckboxItem
                        class="dropdown-menu-item done-filter-item"
                        closeOnSelect={false}
                        checked={props.docChecked(w.docId)}
                        onChange={() => props.toggleDoc(w.docId)}
                      >
                        <CheckBox />
                        <span class="done-filter-item-name">{w.label}</span>
                        <Show when={w.shared}>
                          <SharedGlyph />
                        </Show>
                      </DropdownMenu.CheckboxItem>
                    )}
                  </For>
                </DropdownMenu.SubContent>
              </DropdownMenu.Portal>
            </DropdownMenu.Sub>
          </Show>
          <DropdownMenu.Sub
            gutter={4}
            onOpenChange={(open) => {
              if (!open) setQuery("");
            }}
          >
            <DropdownMenu.SubTrigger class="dropdown-menu-item">
              <span>{m().workspace.filterLists}</span>
              <SubTriggerTail count={props.listCount} />
            </DropdownMenu.SubTrigger>
            <DropdownMenu.Portal>
              <DropdownMenu.SubContent class="dropdown-menu-content done-filter-sub">
                <input
                  // The submenu mounts on open: land in the filter box,
                  // after Kobalte's own focus pass (first row on a
                  // keyboard open, nothing on a hover open).
                  ref={(el) => {
                    inputEl = el;
                    requestAnimationFrame(() => el.focus());
                  }}
                  class="done-filter-input"
                  type="text"
                  autocomplete="off"
                  spellcheck={false}
                  placeholder={m().workspace.filterListsPlaceholder}
                  aria-label={m().workspace.filterListsPlaceholder}
                  value={query()}
                  onInput={(e) => setQuery(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Escape") return;
                    e.stopPropagation();
                  }}
                />
                <div class="done-filter-list">
                  <For each={shownLists()}>
                    {(l) => (
                      <DropdownMenu.CheckboxItem
                        class="dropdown-menu-item done-filter-item"
                        closeOnSelect={false}
                        checked={props.listChecked(l.key)}
                        onChange={() => props.toggleList(l.key)}
                        onPointerMove={(e: PointerEvent) => {
                          if (document.activeElement === inputEl) e.preventDefault();
                        }}
                        onKeyDown={(e: KeyboardEvent) => {
                          if (e.ctrlKey || e.metaKey || e.altKey) return;
                          if (e.key === "Backspace" || (e.key.length === 1 && e.key !== " ")) {
                            e.stopPropagation();
                            inputEl?.focus();
                          }
                        }}
                      >
                        <CheckBox />
                        <span class="done-filter-item-name">{l.name}</span>
                        <Show when={l.archived}>
                          <span class="badge">{m().nav.archived}</span>
                        </Show>
                        <Show when={l.workspace}>
                          <span class="done-filter-item-doc" title={l.workspace}>
                            <SharedGlyph />
                            <span class="done-filter-item-doc-name">{l.workspace}</span>
                          </span>
                        </Show>
                      </DropdownMenu.CheckboxItem>
                    )}
                  </For>
                  <Show when={shownLists().length === 0}>
                    <div class="done-filter-empty">{m().workspace.filterNoLists}</div>
                  </Show>
                </div>
              </DropdownMenu.SubContent>
            </DropdownMenu.Portal>
          </DropdownMenu.Sub>
          <Show when={active()}>
            <DropdownMenu.Separator class="dropdown-menu-separator" />
            <DropdownMenu.Item class="dropdown-menu-item" onSelect={() => props.clear()}>
              {m().workspace.clearFilters}
            </DropdownMenu.Item>
          </Show>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu>
  );
}

/** Right edge of a submenu trigger: how many checks the facet holds, then
 *  the arrow. */
function SubTriggerTail(props: { count: number }) {
  return (
    <span class="done-filter-tail">
      <Show when={props.count > 0}>
        <span class="menu-shortcut">{props.count}</span>
      </Show>
      <span class="menu-sub-arrow" aria-hidden="true">
        ›
      </span>
    </span>
  );
}

/** The people glyph marking a shared workspace, or a list in one. */
function SharedGlyph() {
  return <span class="done-filter-shared" aria-hidden="true" innerHTML={usersSvg} />;
}

/** The row's checkbox: an always-present box, ticked through the item
 *  indicator, so unchecked rows keep their alignment. */
function CheckBox() {
  return (
    <span class="done-filter-box" aria-hidden="true">
      <DropdownMenu.ItemIndicator class="done-filter-tick" innerHTML={checkSvg} />
    </span>
  );
}
