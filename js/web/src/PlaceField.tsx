// The task dialog's place control, in its own ruled band under the
// deadline with the dates' anatomy (a pin glyph inset at the left edge,
// an inset ✕ that removes the place). The results card carries the same
// removal as a Remove row pinned under the list while a place is set:
// the only route on the mobile page, which hides the ✕ (as the date and
// time inputs' pickers do), so starting an edit with a place in hand
// opens the card for it.
//
// Two faces. With a place set and no edit under way, the band shows the
// label as a link to the map (`placeUrl`: the pin when the place has
// coordinates, the map's search for the label when it has none), and the
// blank run of the band beside it starts an edit. While editing, the
// band is a text input holding the label. Enter or blur with a changed
// label writes a label-only place: the coordinates described the old
// label, so a new label always drops them. Emptying it clears.
//
// Suggestions come in tiers (`spec/place-plan.md` "Reuse"). First, the
// text as typed, a label-only place with no lookup behind it: it reads
// as selected while the caret is in the input, and it is what Enter
// writes, so there is always a row Enter takes. Then the known places, the ones already on items as core
// ranks them: focusing the blank field lists the most recent few, and
// typing narrows them from the first character with no network. Under
// them, once three characters are in, a debounced lookup against Photon
// (OpenStreetMap data, `geocode.ts` for the constraints) adds places the
// user has never used, minus any that repeat a known row. Picking any row
// writes that place whole. Arrow down moves from the input into the
// results past the typed row (it is already the selection).

import { Popover } from "@kobalte/core/popover";
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import { placeUrl, searchPlaces } from "./geocode.ts";
import mapPinSvg from "./icons/map-pin.svg?raw";
import clockSvg from "./icons/clock.svg?raw";
import spinnerSvg from "./icons/spinner.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { refreshLocationBias } from "./locationBias.ts";
import { trackOverlay } from "./overlay.ts";
import { dropKnown, filterKnown, type PlaceSuggestion } from "./placeSuggest.ts";
import type { Place } from "./sync/store.ts";

type Status = "idle" | "searching" | "empty" | "error";

/** Quiet time after the last keystroke before a lookup fires. */
const DEBOUNCE_MS = 350;
/** Photon autocompletes from three characters. */
const MIN_CHARS = 3;
/** Known places shown at once: the recents on a blank field, or the
 *  narrowed matches while typing. */
const KNOWN_SHOWN = 6;
/** The typed row's element id, for the input's `aria-activedescendant`. */
const TYPED_ROW_ID = "place-popover-typed";

export function PlaceField(props: {
  place: () => Place | null;
  muted: () => boolean;
  onChange: (place: Place | null) => void;
  /** The places already on items, newest use first, read when the
   *  popover opens (not reactive: a scan in core). */
  knownPlaces: () => PlaceSuggestion[];
}) {
  const { m } = useAppI18n();
  let inputRef: HTMLInputElement | undefined;
  let contentRef: HTMLDivElement | undefined;
  const [draft, setDraft] = createSignal(props.place()?.label ?? "");
  // The input is up while there is nothing to link to, or on request;
  // otherwise the band shows the label as a link.
  const [editing, setEditing] = createSignal(props.place() === null);
  const [open, setOpen] = createSignal(false);
  // While the caret is in the input the typed row is the selection:
  // Enter writes the text as typed, and the row shows it.
  const [inputFocused, setInputFocused] = createSignal(false);
  // The results are portaled out of the task surface, so a focused result
  // sits outside its `data-shortcuts-inert` region. In the side pane
  // (no modal overlay) the workspace's shortcuts would otherwise fire on
  // keystrokes there: Enter's "open the selected item" preventDefault
  // swallowed the button's own click.
  trackOverlay(open);
  const [status, setStatus] = createSignal<Status>("idle");
  const [remote, setRemote] = createSignal<Place[]>([]);
  // Loaded once per open; `null` until then so a stale list from a
  // previous open never shows.
  const [known, setKnown] = createSignal<PlaceSuggestion[] | null>(null);
  let debounce: ReturnType<typeof setTimeout> | undefined;
  // Latest wins: each lookup takes a ticket, and a response whose ticket
  // is no longer current is dropped. Superseded requests are left to
  // finish (the server has done the work either way) rather than aborted,
  // so the network panel shows nothing cancelled; only closing the
  // results aborts whatever is still in flight.
  let ticket = 0;
  let closer = new AbortController();

  // The stored label is the input's truth whenever it changes underneath
  // (another device, a pick, a clear, the dialog moving to another item:
  // the field stays mounted across items); a draft mid-edit is only ever
  // one blur away from being written anyway. The face follows too: a
  // clear leaves nothing to link to, so the input comes back, and a place
  // arriving shows as the link, unless the caret is in the input, where
  // the user keeps the edit they are in.
  createEffect(
    on(
      () => props.place(),
      (p) => {
        setDraft(p?.label ?? "");
        if (!p) setEditing(true);
        else if (!inputFocused()) setEditing(false);
      },
      { defer: true },
    ),
  );

  const ensureKnown = (): PlaceSuggestion[] => {
    const have = known();
    if (have) return have;
    const loaded = props.knownPlaces();
    setKnown(loaded);
    return loaded;
  };

  // The typed text as a place of its own, offered whenever it would be a
  // change: a label-only place, no lookup, no known coordinates.
  const typedRow = createMemo((): Place | null => {
    const t = draft().trim();
    return t && t !== props.place()?.label ? { label: t } : null;
  });
  const localRows = createMemo(() => filterKnown(known() ?? [], draft(), KNOWN_SHOWN));
  const remoteRows = createMemo(() => dropKnown(remote(), localRows()));

  /** The place the band links to: the stored one, unless an edit is up. */
  const linked = (): Place | null => (editing() ? null : props.place());

  const startEdit = () => {
    setEditing(true);
    // The input mounts on the flag; focus it once it is there.
    queueMicrotask(() => inputRef?.focus());
  };

  // Leaving the input: with a place to show, back to the link face.
  const settle = () => {
    if (props.place()) setEditing(false);
  };

  const commit = () => {
    clearTimeout(debounce);
    const t = draft().trim();
    const cur = props.place();
    if (!t) {
      if (cur) props.onChange(null);
      return;
    }
    // An unchanged label writes nothing (a focus and blur is not an
    // edit). A changed one is the text as typed: the old coordinates
    // described the old label.
    if (t !== cur?.label) props.onChange({ label: t });
  };

  const stopRemote = () => {
    clearTimeout(debounce);
    ticket++;
    closer.abort();
    closer = new AbortController();
    setRemote([]);
    setStatus("idle");
  };

  const closeResults = () => {
    stopRemote();
    setOpen(false);
    setKnown(null);
  };
  onCleanup(closeResults);

  // Typing opens the results at once (the typed row is always there for
  // a change, and the known places narrow with it), and schedules a
  // lookup once the text is long enough and differs from the stored
  // label (reopening a dialog is not a search). Each keystroke restarts
  // the clock and retires any response still to come.
  const onTyped = () => {
    ensureKnown();
    const q = draft().trim();
    if (q.length < MIN_CHARS || q === props.place()?.label) {
      stopRemote();
      setOpen(typedRow() !== null || localRows().length > 0);
      return;
    }
    clearTimeout(debounce);
    ticket++;
    // The lookup is pending from this keystroke, not from when the timer
    // fires: showing "Searching…" now keeps the card from opening blank
    // (or holding the last query's "No results") through the debounce.
    setStatus("searching");
    setOpen(true);
    debounce = setTimeout(() => void search(), DEBOUNCE_MS);
  };

  // A blank field, focused, offers the most recent places. An edit of a
  // set place opens the card for its Remove row (with whatever known
  // places the label narrows to above it). Focus is also the only point
  // the app asks the platform where the user is: the permission prompt
  // lands beside the field the fix serves, never at boot
  // (`locationBias.ts`). Fire-and-forget; a fix arriving mid-edit ranks
  // the next lookup.
  const onFocused = () => {
    void refreshLocationBias();
    ensureKnown();
    if (draft().trim() === "") {
      if (localRows().length > 0) setOpen(true);
    } else if (props.place()) {
      setOpen(true);
    }
  };

  // Focus inside the popover (arrow down into the results) is not a
  // departure from the field.
  const insidePopover = (target: EventTarget | null): boolean =>
    target instanceof Node && contentRef !== undefined && contentRef.contains(target);

  const resultButtons = (): HTMLButtonElement[] =>
    contentRef ? Array.from(contentRef.querySelectorAll("button.place-popover-result")) : [];
  const focusResult = (index: number) => {
    const buttons = resultButtons();
    if (buttons.length === 0) return;
    buttons[Math.max(0, Math.min(index, buttons.length - 1))]?.focus();
  };
  const onResultKeyDown = (e: KeyboardEvent, p: Place) => {
    const index = resultButtons().indexOf(e.currentTarget as HTMLButtonElement);
    if (e.key === "Enter") {
      // Pick explicitly rather than ride the button's synthesized click,
      // so the key never reaches the dialog's Enter handling.
      e.preventDefault();
      e.stopPropagation();
      pick(p);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      focusResult(index + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      // Back into the input, where the typed row (if any) is selected.
      if (index <= (typedRow() ? 1 : 0)) inputRef?.focus();
      else focusResult(index - 1);
    } else if (e.key === "Escape") {
      e.stopPropagation();
      closeResults();
      inputRef?.focus();
    }
  };

  const search = async () => {
    const q = draft().trim();
    if (!q) return;
    const mine = ++ticket;
    const signal = closer.signal;
    setStatus("searching");
    setOpen(true);
    try {
      const found = await searchPlaces(q, { signal });
      if (mine !== ticket) return;
      setRemote(found);
      setStatus(found.length ? "idle" : "empty");
    } catch {
      if (mine !== ticket || signal.aborted) return;
      setStatus("error");
    }
  };

  // A pick from the results: focus was in the popover (or the input), so
  // nothing blurs to settle the band; do it here.
  const pick = (p: Place) => {
    props.onChange(p);
    setDraft(p.label);
    closeResults();
    setEditing(false);
  };

  const clear = () => {
    closeResults();
    setDraft("");
    props.onChange(null);
    setEditing(true);
  };

  const anyRows = () =>
    typedRow() !== null || localRows().length > 0 || remoteRows().length > 0;

  const typedActive = () => inputFocused() && typedRow() !== null;

  const resultRow = (p: Place, kind: "typed" | "known" | "lookup") => (
    <li
      role="option"
      id={kind === "typed" ? TYPED_ROW_ID : undefined}
      aria-selected={kind === "typed" ? typedActive() : undefined}
    >
      <button
        type="button"
        class="place-popover-result"
        data-known={kind === "known" ? "" : undefined}
        data-typed={kind === "typed" ? "" : undefined}
        data-active={kind === "typed" && typedActive() ? "" : undefined}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => pick(p)}
        onKeyDown={(e) => onResultKeyDown(e, p)}
      >
        <span class="place-popover-result-label">
          {p.label}
          <Show when={kind === "known"}>
            <span
              class="place-popover-result-known"
              title={m().place.known}
              aria-label={m().place.known}
              innerHTML={clockSvg}
            />
          </Show>
        </span>
        <Show when={kind === "typed"}>
          <span class="place-popover-result-address">{m().place.asTyped}</span>
        </Show>
        <Show when={kind !== "typed" && p.address}>
          {(a) => <span class="place-popover-result-address">{a()}</span>}
        </Show>
      </button>
    </li>
  );

  return (
    <div class="task-dialog-place">
      <div class="task-dialog-dates-row">
        <Popover
          open={open()}
          onOpenChange={(v) => {
            if (!v) closeResults();
          }}
          placement="bottom-start"
          gutter={4}
          // A typeahead, not a picker: a fixed 340px card (see
          // `.place-popover`), and never over the input. With no room
          // below it flips above the input instead of sliding across it
          // (so no `overlap`, unlike the date fields).
        >
          <Popover.Anchor as="span" class="task-dialog-when-anchor">
            <span
              class="task-dialog-when-icon"
              aria-hidden="true"
              innerHTML={mapPinSvg}
            />
            {/* The link face shows only with a place in hand (a removal
                nulls the place a tick before the edit flag follows), the
                input otherwise. */}
            <Show
              when={linked()}
              fallback={
                <input
                  ref={inputRef}
                  type="text"
                  class="task-dialog-when-input task-dialog-place-input"
                  value={draft()}
                  placeholder={m().place.placeholder}
                  aria-label={m().place.label}
                  aria-activedescendant={
                    open() && inputFocused() && typedRow() ? TYPED_ROW_ID : undefined
                  }
                  autocomplete="off"
                  spellcheck={false}
                  data-muted={props.muted() ? "" : undefined}
                  onFocus={() => {
                    setInputFocused(true);
                    onFocused();
                  }}
                  onInput={(e) => {
                    setDraft(e.currentTarget.value);
                    onTyped();
                  }}
                  onBlur={(e) => {
                    setInputFocused(false);
                    if (insidePopover(e.relatedTarget)) return;
                    commit();
                    closeResults();
                    settle();
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      commit();
                      closeResults();
                      settle();
                    } else if (e.key === "Escape" && open()) {
                      e.stopPropagation();
                      closeResults();
                    } else if (e.key === "ArrowDown" && open()) {
                      // Past the typed row: it is the selection already.
                      const first = typedRow() ? 1 : 0;
                      if (resultButtons().length > first) {
                        e.preventDefault();
                        focusResult(first);
                      }
                    }
                  }}
                />
              }
            >
              {(p) => (
                // The link face: the label opens the map, the blank run
                // beside it (a button with no face of its own) starts an
                // edit. Two controls, so the row is a span, not a button.
                <span
                  class="task-dialog-when-input task-dialog-place-display"
                  data-muted={props.muted() ? "" : undefined}
                >
                  <a
                    class="task-dialog-place-link"
                    href={placeUrl(p())}
                    target="_blank"
                    rel="noopener noreferrer"
                    title={p().address ?? m().place.openMap}
                  >
                    {p().label}
                  </a>
                  <button
                    type="button"
                    class="task-dialog-place-edit"
                    aria-label={m().place.edit}
                    onClick={startEdit}
                  />
                </span>
              )}
            </Show>
            <Show when={props.place()}>
              <button
                type="button"
                class="icon-button task-dialog-place-action"
                aria-label={m().place.remove}
                onMouseDown={(e) => e.preventDefault()}
                onClick={clear}
              >
                ✕
              </button>
            </Show>
          </Popover.Anchor>
          <Popover.Portal>
            <Popover.Content
              ref={contentRef}
              class="deadline-dialog when-popover place-popover"
              // The input keeps focus while the results show: this is a
              // typeahead, not a dialog. Moving focus into the popover on
              // open would blur the input, which commits and closes.
              onOpenAutoFocus={(e) => e.preventDefault()}
              onInteractOutside={(e) => {
                if (
                  inputRef &&
                  e.target instanceof Node &&
                  inputRef.contains(e.target)
                ) {
                  e.preventDefault();
                }
              }}
              // Closing never pulls focus back: the input still has it in
              // the typeahead case, and a click elsewhere should land
              // where it was aimed. Escape from a result refocuses
              // explicitly.
              onCloseAutoFocus={(e) => e.preventDefault()}
            >
              <div
                onFocusOut={(e) => {
                  const to = e.relatedTarget;
                  if (to === inputRef || insidePopover(to)) return;
                  closeResults();
                  settle();
                }}
              >
                <Show when={anyRows()}>
                  <ul class="place-popover-list" role="listbox">
                    {/* Keyed: a fresh row per typed value, so the label
                        follows every keystroke rather than the first. */}
                    <Show when={typedRow()} keyed>
                      {(p) => resultRow(p, "typed")}
                    </Show>
                    <For each={localRows()}>{(s) => resultRow(s.place, "known")}</For>
                    <Show when={localRows().length > 0 && remoteRows().length > 0}>
                      <li role="separator" class="place-popover-separator" />
                    </Show>
                    <For each={remoteRows()}>{(p) => resultRow(p, "lookup")}</For>
                  </ul>
                </Show>
                <Show when={status() === "searching"}>
                  <div class="place-popover-note place-popover-searching">
                    <span class="place-popover-spinner" innerHTML={spinnerSvg} />
                    {m().place.searching}
                  </div>
                </Show>
                {/* The lookup's own notes only matter when no known place
                    is showing (the typed row is always there during a
                    lookup, and says nothing about the lookup). */}
                <Show when={status() === "empty" && localRows().length === 0}>
                  <div class="place-popover-note">{m().place.noResults}</div>
                </Show>
                <Show when={status() === "error" && localRows().length === 0}>
                  <div class="place-popover-note">{m().place.error}</div>
                </Show>
                <Show when={remoteRows().length > 0}>
                  <div class="place-popover-credit">{m().place.credit}</div>
                </Show>
                {/* Removal, pinned under everything else while a place is
                    set. Never focused (the cancelled mousedown keeps the
                    caret in the input, as for a result row) and outside
                    the arrow-key run through the results. */}
                <Show when={props.place()}>
                  <div class="place-popover-footer">
                    <button
                      type="button"
                      class="place-popover-remove"
                      tabIndex={-1}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={clear}
                    >
                      {m().place.remove}
                    </button>
                  </div>
                </Show>
              </div>
            </Popover.Content>
          </Popover.Portal>
        </Popover>
      </div>
    </div>
  );
}
