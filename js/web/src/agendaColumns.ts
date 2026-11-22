// The agenda column view's geometry (`Upcoming.tsx` with
// `layout="columns"`): every day is a column of one fixed width, so
// where a day sits and which days to draw are index arithmetic, with
// nothing to measure.

/** A day column's width and the gap after it, in px. The host hands
 *  both to the CSS as `--agenda-col-w` / `--agenda-col-gap`. */
export const AGENDA_COL_WIDTH = 300;
export const AGENDA_COL_GAP = 12;
/** From one column's left edge to the next one's. */
export const AGENDA_COL_PITCH = AGENDA_COL_WIDTH + AGENDA_COL_GAP;

/** The columns to draw (inclusive indices into `n` days) with the
 *  pane's left edge at `left` content px: from a pane's width before
 *  the pane to one after it. `end` is `start - 1` when there are no
 *  days. */
export function columnWindow(
  left: number,
  pane: number,
  n: number,
  pitch: number = AGENDA_COL_PITCH,
): { start: number; end: number } {
  if (n <= 0) return { start: 0, end: -1 };
  const last = n - 1;
  const start = Math.min(last, Math.max(0, Math.floor((left - pane) / pitch)));
  const end = Math.min(last, Math.max(start, Math.floor((left + 2 * pane) / pitch)));
  return { start, end };
}
