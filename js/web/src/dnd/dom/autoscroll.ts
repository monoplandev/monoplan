/**
 * Autoscroll controller: scrolls the list container when drag cursor
 * approaches the top or bottom edge. With `axis` "x" it is the left and
 * right edges and the container scrolls sideways ("up" is left, "down"
 * right).
 */
export class DndAutoscroll {
  private direction: "up" | "down" | null = null;
  private startTime = 0;
  private raf: number | null = null;
  private maxSpeed: number;
  private accelerationMs: number;

  constructor(
    private container: HTMLElement,
    private buffer: number,
    public confine: boolean,
    maxSpeed = 12,
    accelerationMs = 500,
    private axis: "x" | "y" = "y",
  ) {
    this.maxSpeed = maxSpeed;
    this.accelerationMs = accelerationMs;
  }

  /**
   * Call on every mousemove/touchmove during drag.
   * @param clientX - Mouse X in viewport coords
   * @param clientY - Mouse Y in viewport coords
   */
  update(clientX: number, clientY: number): void {
    const box = this.container.getBoundingClientRect();
    // On the x axis the same checks run with the two axes swapped.
    const x = this.axis === "x";
    const rect = x
      ? { left: box.top, right: box.bottom, top: box.left, bottom: box.right }
      : box;
    if (x) [clientX, clientY] = [clientY, clientX];

    // Check x-bounds — stop if cursor leaves horizontal bounds
    if (clientX < rect.left || clientX > rect.right) {
      this.stop();
      return;
    }

    const topZone = rect.top + this.buffer;
    const bottomZone = rect.bottom - this.buffer;

    if (this.confine) {
      // Confine mode: only scroll within container y-bounds
      if (clientY < rect.top || clientY > rect.bottom) {
        this.stop();
        return;
      }
      if (clientY < topZone) {
        this.startScroll("up");
      } else if (clientY > bottomZone) {
        this.startScroll("down");
      } else {
        this.stop();
      }
    } else {
      // Unconfined: scroll when in buffer zone OR beyond container edges
      if (clientY < topZone) {
        this.startScroll("up");
      } else if (clientY > bottomZone) {
        this.startScroll("down");
      } else {
        this.stop();
      }
    }
  }

  stop(): void {
    this.direction = null;
    if (this.raf !== null) {
      cancelAnimationFrame(this.raf);
      this.raf = null;
    }
  }

  private startScroll(dir: "up" | "down"): void {
    if (this.direction === dir) return; // already scrolling this direction
    this.direction = dir;
    this.startTime = performance.now();
    if (this.raf === null) {
      this.tick();
    }
  }

  private tick = (): void => {
    if (this.direction === null) return;

    const elapsed = performance.now() - this.startTime;
    const t = Math.min(elapsed / this.accelerationMs, 1);
    // Start at 20% speed, accelerate to max
    const speed = this.maxSpeed * (0.2 + 0.8 * t);
    const delta = this.direction === "up" ? -speed : speed;

    if (this.axis === "x") this.container.scrollLeft += delta;
    else this.container.scrollTop += delta;
    this.raf = requestAnimationFrame(this.tick);
  };
}
