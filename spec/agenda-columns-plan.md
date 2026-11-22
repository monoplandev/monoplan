# Agenda columns: plan

Decided 2026-11-23. Web only, no core change. Phase 1 built the same day;
phases 2 and 3 are not built.

The calendar (`#upcoming`) has two agenda views, picked by the segmented
control that stands in for the view's title:

- **Agenda list** (segment "List"): the days down the page. What the
  agenda has been all along; see `calendar-plan.md` "Agenda".
- **Agenda columns** (segment "Columns"): the same days side by side, one
  column a day, the pane scrolling sideways.

The UI says "List" / "Columns"; code, comments and specs say agenda list /
agenda columns.

## Decisions

- **One component, two layouts.** The columns are not cards and not a
  second component: `Upcoming.tsx` takes `layout: "list" | "columns"` and
  draws the same day sections and the same rows either way. Everything the
  list does through a day's `<section data-group>` (click, double-click,
  keys, context menu, the "+" and the draft row, drag to reschedule, the
  day-scoped selection, linger, week labels) is shared, not ported. The
  Workspace remounts the component when the mode changes; the selection
  carries across.
- **Same rows.** Both views read `upcomingGroups()`, so placement, Overdue,
  spans, "Hide empty days" and the mini calendar's reach are identical.
  Overdue is its own column, left of Today.
- **Fixed-width columns** (300px, 12px gap, `agendaColumns.ts`). A column's
  place is its index times the pitch, so the column windowing is index
  arithmetic (`columnWindow`) with nothing to measure, and it is a separate
  path from the list's measured-height windowing rather than a
  generalisation of it. Drawn: from one pane's width before the pane to one
  after.
- **Each column scrolls down its own rows** under a pinned header. The view
  opens with Overdue's column (else Today's) at the left edge; the past is a
  scroll left.
- **The mode is a device preference** (`agendaMode.ts`, localStorage, list
  by default). No URL change: `#upcoming` opens whichever was last used.
- **Not a week view or an hour grid** (`calendar-plan.md` still rules that
  out). No multi-day bars across columns: a span repeats a row per day, as
  in the list. No hand ordering within a day. No week snapping.

## Phase 1: columns as a layout of the agenda list (built 2026-11-23)

- `Upcoming` `layout` prop; host `data-layout="columns"`, CSS under
  `.upcoming[data-layout="columns"]` in `styles.css`.
- Column windowing, opening anchor, and the hold when a day is added or
  dropped before the pane (scroll by whole pitches).
- `scrollToToday` / `scrollToDay` (smooth within three pane-widths, a jump
  beyond) and `scrollToKey`; the header's Today button serves both modes.
- Drag to reschedule: unchanged hit test and `dropOnDay`; `DndAutoscroll`
  gained an optional `axis` for the sideways autoscroll.
- The time lane takes its natural width at the row's right edge and no room
  on a row without a time (`data-empty`).
- `test/agendaColumns.test.ts` covers `columnWindow`.

Not checked in a browser at the time of writing.

## Phase 2: what columns add (not built)

- ← / → move the selection to the neighbouring day at the nearest row
  index. Until then ↑ / ↓ walk the rows in page order, so ↓ at the foot of a
  column lands on the top of the next day.
- A vertical wheel over a column header scrolls the days sideways (a mouse
  wheel otherwise only scrolls within a column).

## Phase 3: tidy up (not built)

- `bun test` for the ← / → neighbour pick.
- Delete the orphaned prototype `js/web/src/AgendaColumns.tsx` (cards, not
  imported anywhere).
- Fold the "As built" notes into `calendar-plan.md`'s Agenda section.

## Open

- Titles truncate to one line as in the list, which is tight at 300px;
  wrapping to two lines in columns is undecided.
- If the column path tangles `Upcoming.tsx` further, split the day section
  (header, rows, draft) into its own component that both layouts render.
- The native apps: nothing planned.
