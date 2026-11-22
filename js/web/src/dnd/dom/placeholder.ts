/** Fade time for a placeholder entering or leaving a slot; matches the
 *  rows' `top` transition so the crossfade tracks the nudge. */
const FADE_MS = 200; // TEMP: debug, restore to 150

interface Shade {
  el: HTMLElement;
  /** Slot this shade marks; null until first placed. */
  slot: number | null;
  /** Opacity it is heading to: 1 for the live shade, 0 for the rest. */
  target: 0 | 1;
  anim: Animation | null;
}

/**
 * DOM-based drop-position placeholder. When the target slot changes, the
 * shade at the old slot fades out where it is while a shade at the new slot
 * fades in, so for a moment two (or, on a fast sweep, a few) are visible.
 * Shades live in one absolutely-positioned layer; each is moved with
 * `transform` and faded with the Web Animations API, so updates stay
 * composited and reflow-free. Spent shades stay in the layer at opacity 0
 * and are reused.
 */
export class DndPlaceholder {
  private layer: HTMLElement;
  private shades: Shade[] = [];
  private live: Shade | null = null;
  private itemHeight: number;
  private height: number;
  private borderRadius: number;
  private scrollTop = 0;

  /** @param gap px shaved off the bottom of the slot so the placeholder
   *  matches gapped cards (e.g. the board). 0 = fills the whole slot,
   *  matching the flush list view. */
  constructor(itemHeight: number, borderRadius: number, gap = 0) {
    this.itemHeight = itemHeight;
    this.height = itemHeight - gap;
    this.borderRadius = borderRadius;
    this.layer = document.createElement("div");
    this.layer.className = "dnd-drop-placeholders";
    this.layer.style.cssText =
      `position:absolute;left:0;right:0;top:0;height:0;` +
      `pointer-events:none;z-index:1;`;
  }

  /** Resize when the slot height or gap changes (config live-update). */
  setSize(itemHeight: number, gap = 0): void {
    this.itemHeight = itemHeight;
    this.height = itemHeight - gap;
    for (const s of this.shades) {
      s.el.style.height = `${this.height}px`;
      this.position(s);
    }
  }

  /** The layer holding every shade; the host appends it as a sibling of
   *  the scroll container. */
  getElement(): HTMLElement {
    return this.layer;
  }

  /**
   * Show the placeholder at `slot` (a slot index in the list's drag
   * coordinate space) given the scroll container's `scrollTop`, or fade it
   * out with `slot === null`. Called on every pointer move and scroll; a
   * repeat of the current slot only repositions, so shades still fading
   * keep tracking their slots through autoscroll.
   */
  render(slot: number | null, scrollTop: number): void {
    this.scrollTop = scrollTop;
    if (slot === null) {
      this.clear();
      return;
    }
    if (this.live?.slot !== slot) {
      if (this.live) this.fade(this.live, 0);
      // Back onto a slot that is still fading out: turn that shade round
      // rather than stacking a second one on it.
      const next =
        this.shades.find((s) => s.slot === slot && s.target === 0) ??
        this.idle() ??
        this.create();
      next.slot = slot;
      this.live = next;
      this.fade(next, 1);
    }
    for (const s of this.shades) this.position(s);
  }

  clear(): void {
    if (!this.live) return;
    this.fade(this.live, 0);
    this.live = null;
  }

  private position(s: Shade): void {
    if (s.slot === null) return;
    const y = s.slot * this.itemHeight - this.scrollTop;
    s.el.style.transform = `translateY(${y}px)`;
  }

  /** A shade that has finished fading out, free for a new slot. */
  private idle(): Shade | undefined {
    return this.shades.find(
      (s) => s.target === 0 && (!s.anim || s.anim.playState === "finished"),
    );
  }

  private create(): Shade {
    const el = document.createElement("div");
    el.className = "dnd-drop-placeholder";
    el.style.cssText =
      `position:absolute;left:0;right:0;top:0;` +
      `height:${this.height}px;` +
      `border-radius:${this.borderRadius}px;` +
      `background:var(--dnd-placeholder-color, #3b82f6);` +
      `opacity:0;transform:translateY(0);will-change:transform,opacity;`;
    this.layer.appendChild(el);
    const s: Shade = { el, slot: null, target: 0, anim: null };
    this.shades.push(s);
    return s;
  }

  /** Animate from wherever the shade is now (mid-fade included) to `to`,
   *  taking the share of FADE_MS the remaining distance needs. */
  private fade(s: Shade, to: 0 | 1): void {
    s.target = to;
    const running = s.anim && s.anim.playState === "running";
    const from = running ? Number(getComputedStyle(s.el).opacity) : 1 - to;
    s.anim?.cancel();
    s.anim = null;
    s.el.style.opacity = String(to);
    const duration = FADE_MS * Math.abs(to - from);
    if (duration < 1 || typeof s.el.animate !== "function") return;
    s.anim = s.el.animate([{ opacity: from }, { opacity: to }], {
      duration,
      easing: "ease",
    });
  }
}
