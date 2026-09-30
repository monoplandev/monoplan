import {
  createMemo,
  createSignal,
  onMount,
  onCleanup,
  createEffect,
  on,
  For,
  Show,
  type JSX,
} from "solid-js";
import { DndSource } from "../core/source";
import { DndSelection } from "../core/selection";
import {
  DndController,
  type DndControllerConfig,
  type DndRenderItem,
} from "../core/dnd-controller";
import type { Key, DndOp } from "../core/types";

const BORDER_RADIUS_PX = 4;

export interface DndImperative {
  getSelection(): { blocks: any[]; active: any | null };
  scrollToKey(key: Key): void;
  /** Move keyboard focus to the listbox so arrow/Enter/Space act on it.
   *  Never scrolls: Safari's focus-time reveal is an instant jump that
   *  cancels any smooth scroll the caller started (the board's lane hop),
   *  so callers own bringing the listbox into view. */
  focus(): void;
  /** Preview a foreign drag (from another Dnd) hovering this list as a
   *  drop target: placeholder + nudge + vertical autoscroll. Pass the
   *  pointer's viewport coordinates. */
  setForeignHover(clientX: number, clientY: number): void;
  /** Clear any foreign-drag preview on this list. */
  clearForeignHover(): void;
  /** Insertion slot the foreign-drag preview last showed (0..count, where
   *  count means append past the last item), or null when no foreign drag
   *  is over this list. Read at drop time so the drop lands where previewed. */
  getForeignHoverIndex(): number | null;
}

export interface DndProps<T> {
  items: T[];
  setItems?: (next: T[]) => void;
  onReorder?: (op: DndOp<T>) => void;
  /** Controlled expanded item key. Leave undefined for uncontrolled mode. */
  expandedKey?: Key | null;
  /** Fired whenever the controller requests an expansion state change. */
  onExpandedChange?: (key: Key | null) => void;
  /** Receives an imperative handle for getSelection / scrollToKey / focus. */
  ref?: (handle: DndImperative) => void;
  getKey: (item: T) => Key;
  /** Optional consumer-owned selection model. */
  selection?: DndSelection;
  itemHeight?: number;
  /** Px shaved off the bottom of the drop placeholder so it matches gapped
   *  cards (the board); defaults to 0 for the flush list view. Also shrinks
   *  the drag-overlay clones so they conform to the visible row. */
  placeholderGap?: number;
  /** Border radius (px) applied to drag-overlay clones so they conform to
   *  the host's row visuals (e.g. the board card's 8px). Defaults to the
   *  selection radius. */
  overlayRadius?: number;
  expandable?: boolean;
  overscan?: number;
  confineAutoscroll?: boolean;
  autoscrollBuffer?: number;
  dragStackCount?: number;
  nudge?: boolean;
  /** When false, this list refuses to reorder itself. */
  reorder?: boolean;
  roundedSelect?: boolean;
  autofocus?: boolean;
  /** When false, shift/cmd-click and shift+arrow keys collapse to single-select. */
  multi?: boolean;
  /** When false, plain Arrow keys do nothing. Modifier-key combos (Shift,
   *  Cmd/Ctrl, Alt) keep working for extend/move/jump. Default true. */
  arrowNavigate?: boolean;
  /** When true, a click on the listbox that lands on no row clears the
   *  selection; clicks elsewhere only unfocus the list. */
  clearOnBlankClick?: boolean;
  /** When true, a click anywhere outside the Dnd element clears selection
   *  (implies `clearOnBlankClick`). */
  clearOnClickOutside?: boolean;
  /** When true, host fills its parent's height. */
  fillHeight?: boolean;
  dragType?: "native" | "overlay";
  /** Optional native-drop-data hook (drag-type="native" only). */
  getNativeDropData?: (
    keys: Key[],
    items: T[],
  ) => Array<{ type: string; data: string }>;
  class?: string;
  style?: JSX.CSSProperties | string;
  children: (item: () => T, expanded: () => boolean) => JSX.Element;
}

export function Dnd<T>(props: DndProps<T>): JSX.Element {
  let hostEl!: HTMLDivElement;
  let parentEl!: HTMLDivElement;
  let listboxEl!: HTMLDivElement;

  // Per-key DOM refs used by the controller for overlay clones, expansion
  // observer, hit-testing, and so on.
  const itemEls = new Map<Key, HTMLDivElement>();
  const innerEls = new Map<Key, HTMLDivElement>();

  // version() bumps on every controller onChange; reactive getters depend
  // on it so render state stays consistent without per-property signals.
  const [version, setVersion] = createSignal(0);

  // Singleton expansion state lifted into Solid so per-row `expanded()` and
  // host-level `data-expanded` derive directly from a primitive signal —
  // not from the broad version() bump via per-item render-state lookup.
  const [uncontrolledExpandedKey, setUncontrolledExpandedKey] =
    createSignal<Key | null>(null);

  const keyIndex = createMemo(() => {
    const m = new Map<Key, T>();
    for (const item of props.items) m.set(props.getKey(item), item);
    return m;
  });

  // Key of the final item in logical order. Exposed as `data-last` on the
  // item wrapper so CSS can drop the trailing divider: DOM order is
  // insertion-stable (see renderKeys) and the window is virtualized, so
  // `:last-child` does not identify the last row.
  const lastKey = createMemo<Key | null>(() => {
    const items = props.items;
    return items.length > 0 ? props.getKey(items[items.length - 1]) : null;
  });

  let controller: DndController | null = null;
  let source: DndSource<T> | null = null;
  let syncingControlledExpanded = false;

  const isExpansionControlled = () => props.expandedKey !== undefined;
  const expandedKey = createMemo<Key | null>(() =>
    isExpansionControlled() ? (props.expandedKey ?? null) : uncontrolledExpandedKey(),
  );

  const cfg = (): DndControllerConfig => ({
    itemHeight: props.itemHeight ?? 32,
    placeholderGap: props.placeholderGap ?? 0,
    overlayRadius: props.overlayRadius ?? BORDER_RADIUS_PX,
    overscan: props.overscan ?? 2,
    dragType: props.dragType ?? "overlay",
    roundedSelect: props.roundedSelect ?? true,
    expansionEnabled: props.expandable ?? false,
    confineAutoscroll: props.confineAutoscroll ?? true,
    autoscrollBuffer: props.autoscrollBuffer ?? 32,
    dragStackCount: props.dragStackCount ?? 3,
    nudge: props.nudge ?? true,
    reorder: props.reorder !== false,
    multi: props.multi !== false,
    arrowNavigate: props.arrowNavigate !== false,
    clearOnBlankClick: props.clearOnBlankClick ?? false,
    clearOnClickOutside: props.clearOnClickOutside ?? false,
    fillHeight: props.fillHeight ?? false,
  });

  onMount(() => {
    controller = new DndController({
      config: cfg(),
      host: { host: hostEl, parent: parentEl, listbox: listboxEl },
      onChange: () => setVersion((v) => v + 1),
      getItemElement: (key) => itemEls.get(key) ?? null,
      getItemInnerElement: (key) => innerEls.get(key) ?? null,
    });

    source = new DndSource<T>({
      getKey: props.getKey,
      getOrder: () => props.items.map(props.getKey),
      getItem: (key) => keyIndex().get(key),
    });

    source.onChange((op) => {
      if (op.type === "move" && props.setItems) {
        const keySet = new Set(op.keys);
        const moved = op.keys
          .map((k) => keyIndex().get(k))
          .filter((i): i is T => i !== undefined);
        const filtered = props.items.filter(
          (i) => !keySet.has(props.getKey(i)),
        );
        let next: T[];
        if (op.beforeKey === null) {
          next = [...filtered, ...moved];
        } else {
          const idx = filtered.findIndex(
            (i) => props.getKey(i) === op.beforeKey,
          );
          if (idx === -1) next = [...filtered, ...moved];
          else {
            next = [...filtered];
            next.splice(idx, 0, ...moved);
          }
        }
        props.setItems(next);
      }
      props.onReorder?.(op);
    });

    controller.setSource(source);
    if (props.selection) controller.setSelection(props.selection);
    controller.setRenderer({
      // Solid owns per-item rendering through JSX/<For>; the renderer's mount
      // is intentionally a no-op so no per-item Solid roots are created — the
      // whole point of this architecture.
      mount: () => () => {},
      // Push singleton expansion state into Solid as a primitive signal so
      // only the matching row reacts on flip — instead of every row reading
      // through the per-item state map on every version() bump.
      setExpanded: (k) => {
        if (!isExpansionControlled()) {
          setUncontrolledExpandedKey(k);
        }
        if (!syncingControlledExpanded) {
          props.onExpandedChange?.(k);
        }
      },
      getNativeDropData: props.getNativeDropData,
    });

    // Wire DOM events
    parentEl.addEventListener("scroll", controller.onScroll, { passive: true });
    listboxEl.addEventListener("keydown", controller.onKeyDown);
    listboxEl.addEventListener("mousedown", controller.onMouseDown);
    listboxEl.addEventListener("click", controller.onClick);
    listboxEl.addEventListener("dblclick", controller.onDblClick);
    listboxEl.addEventListener("touchstart", controller.onTouchStart, {
      passive: false,
    });
    listboxEl.addEventListener("touchmove", controller.onTouchMove, {
      passive: false,
    });
    listboxEl.addEventListener("touchend", controller.onTouchEnd);

    if (props.autofocus) {
      queueMicrotask(() => {
        // Don't steal focus from an editable element that another mount
        // path (e.g. rename-on-dblclick) just focused via microtask.
        const ae = document.activeElement;
        if (
          ae instanceof HTMLElement &&
          (ae.isContentEditable ||
            ae.tagName === "INPUT" ||
            ae.tagName === "TEXTAREA")
        ) {
          return;
        }
        listboxEl.focus();
      });
    }

    props.ref?.({
      getSelection: () => controller!.getSelection(),
      scrollToKey: (k) => controller!.scrollToKey(k),
      focus: () => listboxEl.focus({ preventScroll: true }),
      setForeignHover: (x, y) => controller!.setForeignHover(x, y),
      clearForeignHover: () => controller!.clearForeignHover(),
      getForeignHoverIndex: () => controller!.getForeignHoverIndex(),
    });

    setVersion((v) => v + 1);
  });

  // External item changes — re-sync source order
  createEffect(
    on(
      () => props.items,
      () => {
        if (source) source.syncOrder();
      },
      { defer: true },
    ),
  );

  // Live-track config changes (excluding ones that need re-init like dragType)
  createEffect(() => {
    if (!controller) return;
    controller.setConfig(cfg());
  });

  createEffect(() => {
    if (!controller || !isExpansionControlled()) return;
    const next = props.expandedKey ?? null;
    if (controller.getExpanded() !== next) {
      syncingControlledExpanded = true;
      controller.setExpanded(next);
      syncingControlledExpanded = false;
    }
  });

  onCleanup(() => {
    if (controller) {
      parentEl.removeEventListener("scroll", controller.onScroll);
      listboxEl.removeEventListener("keydown", controller.onKeyDown);
      listboxEl.removeEventListener("mousedown", controller.onMouseDown);
      listboxEl.removeEventListener("click", controller.onClick);
      listboxEl.removeEventListener("dblclick", controller.onDblClick);
      listboxEl.removeEventListener("touchstart", controller.onTouchStart);
      listboxEl.removeEventListener("touchmove", controller.onTouchMove);
      listboxEl.removeEventListener("touchend", controller.onTouchEnd);
      controller.destroy();
      controller = null;
    }
  });

  // Reactive render state.
  const renderState = createMemo(() => {
    version();
    return (
      controller?.getRenderState() ?? {
        isDragging: false,
        listboxHeight: 0,
        items: [] as DndRenderItem[],
        keepAlive: [] as DndRenderItem[],
      }
    );
  });

  // <For> over the union of visible + dragged keys. Using a single keyed list
  // means dragged items are kept mounted (not unmounted-then-remounted) across
  // drag start/end, so consumer per-item state — input edits, hover, focus —
  // survives the drag. This is the architectural advantage over the old
  // per-item-Solid-root design.
  //
  // We pass an array of KEYS (primitives) — not item-state objects — because
  // Solid's <For> keys rows by reference (===). With primitives, "5" === "5"
  // matches across renders so rows are reused; with fresh objects, every
  // render fully unmounts and remounts every row, losing state.
  //
  // Critically, the order returned here must stay STABLE across reorders.
  // Solid's <For> reorders DOM via insertBefore when the key array order
  // changes — and moving a DOM node within its parent resets running CSS
  // transitions (Chrome/Safari). Keeping DOM order at insertion-order
  // means top/height transitions on `.dnd-item` actually fire when items
  // logically reorder; visual order is driven entirely by absolute `top`.
  const stableKeyOrder: Key[] = [];
  const renderKeys = createMemo<Key[]>(() => {
    const s = renderState();
    const wanted = new Set<Key>();
    for (const it of s.items) wanted.add(it.key);
    for (const it of s.keepAlive) wanted.add(it.key);

    // Drop keys that left the visible/keep-alive set; keep insertion order
    // for everyone else (no DOM reorder → transitions survive).
    for (let i = stableKeyOrder.length - 1; i >= 0; i--) {
      if (!wanted.has(stableKeyOrder[i])) stableKeyOrder.splice(i, 1);
    }
    const have = new Set(stableKeyOrder);
    for (const it of s.items) {
      if (!have.has(it.key)) {
        stableKeyOrder.push(it.key);
        have.add(it.key);
      }
    }
    for (const it of s.keepAlive) {
      if (!have.has(it.key)) {
        stableKeyOrder.push(it.key);
        have.add(it.key);
      }
    }
    // Return a fresh array so the memo dependents recompute on adds/removes
    // — but the contained primitives match by `===` so <For> reuses rows.
    return stableKeyOrder.slice();
  });

  // Look up the per-item state by key so <For> rows derive their props
  // reactively without re-creating row JSX on every state change.
  const stateByKey = createMemo(() => {
    const s = renderState();
    const map = new Map<Key, DndRenderItem>();
    for (const it of s.items) map.set(it.key, it);
    for (const it of s.keepAlive) map.set(it.key, it);
    return map;
  });

  const ariaMulti = createMemo<"true" | "false">(() =>
    props.multi !== false ? "true" : "false",
  );

  const hostStyle = createMemo<JSX.CSSProperties>(() => {
    const fill = props.fillHeight ?? false;
    return {
      position: "relative",
      display: "block",
      ...(fill ? { height: "100%" } : {}),
    };
  });

  const parentStyle = createMemo<JSX.CSSProperties>(() => {
    const fill = props.fillHeight ?? false;
    return {
      position: "relative",
      "overflow-y": "auto",
      "z-index": 2,
      ...(fill ? { height: "100%" } : {}),
    };
  });

  const listboxStyle = createMemo<JSX.CSSProperties>(() => {
    const fill = props.fillHeight ?? false;
    return {
      position: "relative",
      outline: "none",
      height: `${renderState().listboxHeight}px`,
      ...(fill ? { "min-height": "100%" } : {}),
      ...(renderState().isDragging ? { overflow: "hidden" } : {}),
    };
  });

  const styleSheet = createMemo(() => {
    const base = `
      .dnd-item {
        position: absolute;
        left: 0;
        right: 0;
        overflow: hidden;
        transition: top 0.15s ease, height 0.15s ease;
      }
      .dnd-item-inner {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
      }
      .dnd-item-inner[data-expanded] {
        bottom: auto;
      }
      [data-selected] { background: var(--dnd-select-bg, transparent); z-index: 1; }
    `;
    if (!(props.roundedSelect ?? true)) return base;
    return `
      ${base}
      [data-sel-first] { border-top-left-radius: ${BORDER_RADIUS_PX}px; border-top-right-radius: ${BORDER_RADIUS_PX}px; }
      [data-sel-last] { border-bottom-left-radius: ${BORDER_RADIUS_PX}px; border-bottom-right-radius: ${BORDER_RADIUS_PX}px; }
    `;
  });

  // After the host renders item DOM, give the controller a chance to
  // (re)attach the expansion observer to the inner element.
  createEffect(() => {
    renderKeys();
    stateByKey();
    queueMicrotask(() => controller?.syncExpansionObserver());
  });

  // Imperative <style> element — emitting via JSX `<style>{expr}</style>` is
  // brittle (Solid's reactive child wrapper inserts marker comments into the
  // <style>'s textContent, which the browser parses as invalid CSS and
  // discards the whole sheet). Without the CSS, `.dnd-item` loses its
  // top/height transitions and `.dnd-item-inner[data-expanded]` never flips
  // to `bottom:auto`, so the ResizeObserver only sees the outer's collapsed
  // height and the expanded item never grows.
  let styleEl!: HTMLStyleElement;
  createEffect(() => {
    if (styleEl) styleEl.textContent = styleSheet();
  });

  return (
    <div
      ref={hostEl}
      class={props.class}
      data-expanded={expandedKey() != null ? "" : undefined}
      style={
        typeof props.style === "string"
          ? props.style
          : { ...hostStyle(), ...((props.style as JSX.CSSProperties) ?? {}) }
      }
    >
      <style ref={styleEl} />
      <div
        ref={parentEl}
        class="dnd-parent"
        tabindex="-1"
        style={parentStyle()}
      >
        <div
          ref={listboxEl}
          role="listbox"
          aria-multiselectable={ariaMulti()}
          tabindex="-1"
          style={listboxStyle()}
        >
          <For each={renderKeys()}>
            {(key) => {
              const state = createMemo(() => stateByKey().get(key));
              const item = createMemo(() => keyIndex().get(key));
              const expanded = createMemo(() => expandedKey() === key);
              const selected = createMemo(() => state()?.selected ?? false);
              const selFirst = createMemo(() => state()?.selFirst ?? false);
              const selLast = createMemo(() => state()?.selLast ?? false);
              const hidden = createMemo(() => state()?.hidden ?? false);
              const last = createMemo(() => lastKey() === key);
              const top = createMemo(() => state()?.top ?? 0);
              const height = createMemo(
                () => state()?.height ?? props.itemHeight ?? 32,
              );

              const isNative = createMemo(
                () => (props.dragType ?? "overlay") === "native",
              );

              return (
                <div
                  ref={(el) => {
                    itemEls.set(key, el);
                  }}
                  class="dnd-item"
                  role="option"
                  data-key={String(key)}
                  data-selected={selected() ? "" : undefined}
                  data-sel-first={selFirst() ? "" : undefined}
                  data-sel-last={selLast() ? "" : undefined}
                  data-last={last() ? "" : undefined}
                  aria-selected={selected() ? "true" : "false"}
                  draggable={isNative()}
                  onDragStart={(e) => {
                    if (isNative()) controller?.onNativeDragStart(e, key);
                  }}
                  onDragEnd={() => {
                    if (isNative()) controller?.onNativeDragEnd();
                  }}
                  style={{
                    top: `${top()}px`,
                    height: `${height()}px`,
                    ...(hidden()
                      ? { display: "none", "pointer-events": "none" }
                      : {}),
                  }}
                >
                  <div
                    ref={(el) => {
                      innerEls.set(key, el);
                    }}
                    class="dnd-item-inner"
                    data-expanded={expanded() ? "" : undefined}
                  >
                    <Show when={item() !== undefined}>
                      {props.children(item as () => T, expanded)}
                    </Show>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </div>
    </div>
  );
}
