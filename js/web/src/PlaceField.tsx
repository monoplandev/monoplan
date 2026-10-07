// The task dialog's place control: a text input in its own ruled band
// under the deadline, with the dates' anatomy (a pin glyph inset at the
// left edge, an inset ✕ that removes the place). Unlike the date inputs it
// is editable: the text is the place's label, and a typed label is a
// complete place on its own. Enter or blur with a changed label writes a
// label-only place (any coordinates belonged to the old label, so they
// go); emptying it clears. Typing runs a debounced lookup against Photon
// (OpenStreetMap data, `geocode.ts` for the constraints) once three
// characters are in: a popover of results opens under the input, and
// picking one writes the full place, coordinates and address included.
// Arrow down moves from the input into the results. A place with
// coordinates shows its address muted under the input with a link to the
// map.

import { Popover } from "@kobalte/core/popover";
import { createEffect, createSignal, For, on, onCleanup, Show } from "solid-js";
import { osmMapUrl, searchPlaces } from "./geocode.ts";
import mapPinSvg from "./icons/map-pin.svg?raw";
import externalLinkSvg from "./icons/external-link.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import type { Place } from "./sync/store.ts";

type Status = "idle" | "searching" | "empty" | "error";

/** Quiet time after the last keystroke before a lookup fires. */
const DEBOUNCE_MS = 350;
/** Photon autocompletes from three characters. */
const MIN_CHARS = 3;

export function PlaceField(props: {
  place: () => Place | null;
  muted: () => boolean;
  onChange: (place: Place | null) => void;
}) {
  const { m } = useAppI18n();
  let inputRef: HTMLInputElement | undefined;
  let contentRef: HTMLDivElement | undefined;
  let listRef: HTMLUListElement | undefined;
  const [draft, setDraft] = createSignal(props.place()?.label ?? "");
  const [open, setOpen] = createSignal(false);
  const [status, setStatus] = createSignal<Status>("idle");
  const [results, setResults] = createSignal<Place[]>([]);
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

  const commit = () => {
    clearTimeout(debounce);
    const t = draft().trim();
    const cur = props.place();
    if (!t) {
      if (cur) props.onChange(null);
      return;
    }
    if (t !== cur?.label) props.onChange({ label: t });
  };

  const closeResults = () => {
    clearTimeout(debounce);
    ticket++;
    closer.abort();
    closer = new AbortController();
    setOpen(false);
    setResults([]);
    setStatus("idle");
  };
  onCleanup(closeResults);

  // Typing schedules a lookup once the text is long enough and differs
  // from the stored label (reopening a dialog is not a search). Each
  // keystroke restarts the clock and retires any response still to come.
  const scheduleSearch = () => {
    clearTimeout(debounce);
    const q = draft().trim();
    if (q.length < MIN_CHARS || q === props.place()?.label) {
      closeResults();
      return;
    }
    ticket++;
    debounce = setTimeout(() => void search(), DEBOUNCE_MS);
  };

  // Focus inside the popover (arrow down into the results) is not a
  // departure from the field.
  const insidePopover = (target: EventTarget | null): boolean =>
    target instanceof Node && contentRef !== undefined && contentRef.contains(target);

  const resultButtons = (): HTMLButtonElement[] =>
    listRef ? Array.from(listRef.querySelectorAll("button")) : [];
  const focusResult = (index: number) => {
    const buttons = resultButtons();
    if (buttons.length === 0) return;
    buttons[Math.max(0, Math.min(index, buttons.length - 1))]?.focus();
  };
  const onResultKeyDown = (e: KeyboardEvent, index: number) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      focusResult(index + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (index === 0) inputRef?.focus();
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
      setResults(found);
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
          // A typeahead, not a picker: as wide as the input, and never
          // over it. With no room below it flips above the input instead
          // of sliding across it (so no `overlap`, unlike the date fields).
          sameWidth
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
              onInput={(e) => {
                setDraft(e.currentTarget.value);
                scheduleSearch();
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
                } else if (e.key === "ArrowDown" && open() && results().length > 0) {
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
                <Show when={status() === "searching"}>
                  <div class="place-popover-note">{m().place.searching}</div>
                </Show>
                <Show when={status() === "empty"}>
                  <div class="place-popover-note">{m().place.noResults}</div>
                </Show>
                <Show when={status() === "error"}>
                  <div class="place-popover-note">{m().place.error}</div>
                </Show>
                <Show when={results().length > 0}>
                  <ul class="place-popover-list" role="listbox" ref={listRef}>
                    <For each={results()}>
                      {(p, i) => (
                        <li role="option">
                          <button
                            type="button"
                            class="place-popover-result"
                            onMouseDown={(e) => e.preventDefault()}
                            onClick={() => pick(p)}
                            onKeyDown={(e) => onResultKeyDown(e, i())}
                          >
                            <span class="place-popover-result-label">{p.label}</span>
                            <Show when={p.address}>
                              {(a) => (
                                <span class="place-popover-result-address">{a()}</span>
                              )}
                            </Show>
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
                <div class="place-popover-credit">{m().place.credit}</div>
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
