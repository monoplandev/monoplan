// The task dialog's place control: a text input in its own ruled band
// under the deadline, with the dates' anatomy (a pin glyph inset at the
// left edge, an inset ✕ that removes the place). Unlike the date inputs it
// is editable: the text is the place's label, and a typed label is a
// complete place on its own. Enter or blur with a changed label writes a
// label-only place (any coordinates belonged to the old label, so they
// go); emptying it clears.
//
// Suggestions come in two tiers (`spec/place-plan.md` "Reuse"). Known
// places, the ones already on items as core ranks them, show first:
// focusing the blank field lists the most recent few, and typing narrows
// them from the first character with no network. Under them, once three
// characters are in, a debounced lookup against Photon (OpenStreetMap
// data, `geocode.ts` for the constraints) adds places the user has never
// used, minus any that repeat a known row. Picking either writes the
// whole place; Enter on text that names a known place reuses it,
// coordinates included, rather than writing a label-only one. Arrow
// down moves from the input into the results. A place with coordinates
// shows its address muted under the input with a link to the map.

import { Popover } from "@kobalte/core/popover";
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import { osmMapUrl, searchPlaces } from "./geocode.ts";
import mapPinSvg from "./icons/map-pin.svg?raw";
import clockSvg from "./icons/clock.svg?raw";
import externalLinkSvg from "./icons/external-link.svg?raw";
import spinnerSvg from "./icons/spinner.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { trackOverlay } from "./overlay.ts";
import {
  dropKnown,
  exactKnown,
  filterKnown,
  samePlace,
  type PlaceSuggestion,
} from "./placeSuggest.ts";
import type { Place } from "./sync/store.ts";

type Status = "idle" | "searching" | "empty" | "error";

/** Quiet time after the last keystroke before a lookup fires. */
const DEBOUNCE_MS = 350;
/** Photon autocompletes from three characters. */
const MIN_CHARS = 3;
/** Known places shown at once: the recents on a blank field, or the
 *  narrowed matches while typing. */
const KNOWN_SHOWN = 6;

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
  const [open, setOpen] = createSignal(false);
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
  // (another device, a pick, a clear); a draft mid-edit is only ever one
  // blur away from being written anyway.
  createEffect(
    on(
      () => props.place()?.label ?? "",
      (label) => setDraft(label),
    ),
  );

  const ensureKnown = (): PlaceSuggestion[] => {
    const have = known();
    if (have) return have;
    const loaded = props.knownPlaces();
    setKnown(loaded);
    return loaded;
  };

  const localRows = createMemo(() => filterKnown(known() ?? [], draft(), KNOWN_SHOWN));
  const remoteRows = createMemo(() => dropKnown(remote(), localRows()));

  const commit = () => {
    clearTimeout(debounce);
    const t = draft().trim();
    const cur = props.place();
    if (!t) {
      if (cur) props.onChange(null);
      return;
    }
    // An unchanged label writes nothing (a focus and blur is not an
    // edit). A typed label that names a known place takes that place
    // whole: "gym" reuses the geocoded Gym. Only a whole-label match
    // counts.
    if (t === cur?.label) return;
    const match = exactKnown(ensureKnown(), t);
    if (match && !samePlace(match, cur)) props.onChange(match);
    else if (!match) props.onChange({ label: t });
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

  // Typing narrows the known places at once, and schedules a lookup once
  // the text is long enough and differs from the stored label (reopening
  // a dialog is not a search). Each keystroke restarts the clock and
  // retires any response still to come.
  const onTyped = () => {
    ensureKnown();
    const q = draft().trim();
    if (q.length < MIN_CHARS || q === props.place()?.label) {
      stopRemote();
      setOpen(localRows().length > 0);
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

  // A blank field, focused, offers the most recent places.
  const onFocused = () => {
    if (draft().trim() !== "") return;
    ensureKnown();
    if (localRows().length > 0) setOpen(true);
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
      if (index <= 0) inputRef?.focus();
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

  const pick = (p: Place) => {
    props.onChange(p);
    setDraft(p.label);
    closeResults();
  };

  const clear = () => {
    closeResults();
    setDraft("");
    props.onChange(null);
  };

  const coords = () => {
    const p = props.place();
    return p && p.lat != null && p.lon != null ? { lat: p.lat, lon: p.lon } : null;
  };

  const anyRows = () => localRows().length > 0 || remoteRows().length > 0;

  const resultRow = (p: Place, isKnown: boolean) => (
    <li role="option">
      <button
        type="button"
        class="place-popover-result"
        data-known={isKnown ? "" : undefined}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => pick(p)}
        onKeyDown={(e) => onResultKeyDown(e, p)}
      >
        <span class="place-popover-result-label">
          {p.label}
          <Show when={isKnown}>
            <span
              class="place-popover-result-known"
              title={m().place.known}
              aria-label={m().place.known}
              innerHTML={clockSvg}
            />
          </Show>
        </span>
        <Show when={p.address}>
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
            <input
              ref={inputRef}
              type="text"
              class="task-dialog-when-input task-dialog-place-input"
              value={draft()}
              placeholder={m().place.placeholder}
              aria-label={m().place.label}
              autocomplete="off"
              spellcheck={false}
              data-muted={props.muted() ? "" : undefined}
              onFocus={onFocused}
              onInput={(e) => {
                setDraft(e.currentTarget.value);
                onTyped();
              }}
              onBlur={(e) => {
                if (insidePopover(e.relatedTarget)) return;
                commit();
                closeResults();
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commit();
                  closeResults();
                } else if (e.key === "Escape" && open()) {
                  e.stopPropagation();
                  closeResults();
                } else if (e.key === "ArrowDown" && open() && anyRows()) {
                  e.preventDefault();
                  focusResult(0);
                }
              }}
            />
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
                }}
              >
                <Show when={anyRows()}>
                  <ul class="place-popover-list" role="listbox">
                    <For each={localRows()}>{(s) => resultRow(s.place, true)}</For>
                    <Show when={localRows().length > 0 && remoteRows().length > 0}>
                      <li role="separator" class="place-popover-separator" />
                    </Show>
                    <For each={remoteRows()}>{(p) => resultRow(p, false)}</For>
                  </ul>
                </Show>
                <Show when={status() === "searching"}>
                  <div class="place-popover-note place-popover-searching">
                    <span class="place-popover-spinner" innerHTML={spinnerSvg} />
                    {m().place.searching}
                  </div>
                </Show>
                {/* The lookup's own notes only matter when nothing of the
                    user's is already showing. */}
                <Show when={status() === "empty" && !anyRows()}>
                  <div class="place-popover-note">{m().place.noResults}</div>
                </Show>
                <Show when={status() === "error" && !anyRows()}>
                  <div class="place-popover-note">{m().place.error}</div>
                </Show>
                <Show when={remoteRows().length > 0}>
                  <div class="place-popover-credit">{m().place.credit}</div>
                </Show>
              </div>
            </Popover.Content>
          </Popover.Portal>
        </Popover>
      </div>
      <Show when={coords()}>
        {(c) => (
          <div class="task-dialog-place-address" data-muted={props.muted() ? "" : undefined}>
            <span class="task-dialog-place-address-text">
              {props.place()?.address ?? `${c().lat}, ${c().lon}`}
            </span>
            <a
              class="task-dialog-place-map"
              href={osmMapUrl(c().lat, c().lon)}
              target="_blank"
              rel="noopener noreferrer"
            >
              {m().place.openMap}
              <span class="task-dialog-place-map-icon" innerHTML={externalLinkSvg} />
            </a>
          </div>
        )}
      </Show>
    </div>
  );
}
