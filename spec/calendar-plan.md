# Calendar: plan

**Status: built 2026-09-09 (Phase 0 core / wasm / CLI and Phase 1 web).
Decided 2026-09-09, trimmed the same day.** Adds
a second date to items, `when`, alongside the existing `deadline`, and grows
the Upcoming view into a day-granular agenda over both dates. The first cut
is the basics only: the field, the agenda, and a When control with an
optional time. No month grid, no recurrence rules, no
time zones, no export, each with its extension point reserved so it lands
later without a migration.

Companion to `data-model.md` (fields, mutations), `board.md` (lens model),
`urls.md` (view tokens), `cli.md` (verbs). Amend those in place as each phase
lands; this file is the design record.

## Decisions in one screen

| Question | Decision |
|---|---|
| Why a second field? | `deadline` means "owed by": past it the item is overdue and red. A planned or scheduled day means "happens on" or "act on": past it the item is not owed, it is simply over. Using `deadline` for the second meaning makes every past event scream overdue. The distinction is what happens after the date, so it needs its own field. |
| Replace `deadline`, or flag it? | **Neither. Add `when` beside it.** Both can coexist on one item ("do it Saturday, due Oct 31"), which a single date plus a kind flag can't express. Additive, so six months of real data needs no migration. Same names as Things, which is the model people already know. |
| Date and time: one register or two? | **One register, `when`, shape-discriminated.** `YYYY-MM-DD` is all-day, `YYYY-MM-DDTHH:MM` is timed. Same rule as iCalendar `VALUE=DATE` vs `DATE-TIME`. One register can't tear under concurrent edit (date moved on one device, time set on another), sorts by plain string compare, and clears with one delete. An explicit flag is redundant with the shape and a third thing to keep consistent. |
| Time zone? | **Floating only, now.** A `when` is a wall-clock intent, the devices travel together, and floating keeps sorting a string compare. The grammar reserves an RFC 9557 bracketed IANA suffix (`...T14:00[Europe/London]`) for fixed-instant values. Writers may later default to appending the device zone; untouched values stay floating, so nothing migrates. Note this is the task-manager default, not the calendar default: Apple and Google pin timed events to a zone. |
| Recurrence? | **Not in the first cut.** No `repeat` field, no hint of one in the doc. Repeat-on-done (clone with the date advanced when ticked Done) is the likely first form; RRULE-style schedules are a calendar app's job. |
| Does the clock ever write? | **Never.** A past `when` stays where it was set, on its day in the past. Nothing promotes an item to Live, adds it to Focus, or moves it because a day arrived: every device would race to do it. |
| What does a past `when` mean? | **On an event, it went by; on a task, it is overdue** (decided 2026-11-13, narrowing 2026-09-30, which let every past `when` go by). `when` is fixed and does not roll over: it is never rewritten and never reinterpreted as "today". An event's `when` from a previous day is in the past, and the agenda, which renders no past days, does not select it. An Open task's is still to do: it was planned and not done, so it sits in Overdue until it is ticked, re-planned or cleared. The 2026-09-30 rule predates the `events` list (2026-10-02): once things that merely happen had a home of their own, a dated item outside it is work, and work that silently left the agenda when its day passed was lost. |
| Calendar surface | **One lens: Upcoming becomes the agenda** (day sections, both dates). Day granularity only: no hour grid, no overlap. Items carry an optional `duration` (added 2026-09-16, see "Field: `duration`"), which the agenda reads for multi-day all-day spans and the end-time display but never lays out on an hour axis. A month grid is deferred; drag-to-reschedule on the agenda was built 2026-11-22 ("Agenda"). |
| Time input | **Typed time picker** (`TimePicker.tsx`): a text input with a suggestion list, 12 or 24-hour cycle from the existing time-format preference. Empty means all-day. See "Task surface and rows". |
| Export / CalDAV | **Deferred.** Mapping is recorded below. A subscribable feed needs a server that can read items, which the E2EE server cannot; a non-E2EE CalDAV carve-out is a separate conversation. |
| Schema | **Additive within v4.** No break, no import step. |

## Semantics

| | before the day | on the day | after the day, still Open |
|---|---|---|---|
| `deadline` | upcoming | Today, warning tone | Overdue section, overdue tone (red) |
| `when` on a task | upcoming | Today, neutral tone | Overdue section, overdue tone |
| `when` on an event | upcoming | Today, neutral tone | in the past: not on the agenda, never red |

An overdue deadline moves the row to an Overdue section above Today
(decided 2026-09-16, replacing the earlier fold into Today): what is owed
is visible at a glance and Today reads as today's plan. The section exists
only while something is overdue.

**The two dates place independently** (decided 2026-11-17, replacing
"one row on the earlier of the two days"). The `when` is the item's date
record and puts it on its day. The deadline is a special kind of date on
the calendar: a row of its own on the day it is owed, marked as a
deadline (timer glyph and "Due" in the web's time lane), beside whatever
row the `when` gives the item. So an item planned for Saturday and due
on the 31st is on both days. There is one row per item per group, though:
where the deadline falls on a day the `when` already covers, that row
stands for both and carries a "Due" mark; with both dates behind it the
item is in Overdue once, by its deadline. Before this an item with both
dates was on the calendar once, and whichever date came later was
nowhere on it. The deadline is also a menu-only register on the task
surface since the same day (see "Task surface and rows").

**A task's past `when` is overdue** (decided 2026-11-13). An Open item
that is not an event (`events-plan.md`) whose `when` is over (its whole span ended
before today) joins the Overdue section, placed by its `when`, overdue
tone, whether or not it has a deadline still ahead (that one keeps its
own row on its day). If its deadline is overdue as well, the deadline
places the one row, as for any overdue deadline.
Nothing is written: the `when` keeps its value, and the row leaves Overdue
when the task is ticked, cancelled, re-planned, or has its `when` cleared.
Marking the item an event takes it out too, since an event goes by.

**An event's past `when` is not judged and not surfaced** (decided
2026-09-30 for every item, replacing an earlier rule that listed it under
Overdue for the user to decide; narrowed to events 2026-11-13). It went by
and lives on its day in the past: it does not roll over into Today, is not
rewritten by the clock, and places nothing on the agenda, which renders no
past days. An event with a past `when` and a deadline is placed by the
deadline alone. Past days become visible when a calendar view that can
reach them is built, or when the agenda learns to scroll backwards: the
CLI and iOS agenda did on 2026-11-16 (`agenda --past N`, `spec/cli.md`
§Items), where a past event sits on its day, neutral, above Today. The
web agenda did on 2026-11-17 (`groupByDay`'s `floor`, a year back): the
same past days above Overdue and Today, the view opening scrolled to
Overdue's header, or Today's when nothing is overdue. The web keeps its
own two differences there: a cancelled item stays on its day as a done
one does, and a multi-day `when` is on every past day it covered.

Done and binned items keep both fields untouched, as they keep `deadline`
today. The fields are never cleared by a transition. Restore brings the
dates back with the item.

**A closed task is on the day it was closed** (decided 2026-11-18,
narrowing the rule below to events; the web first, the engine's agenda
and so the CLI and the Apple calendars the same day). A done or
cancelled task, timed or not, leaves both its dates. Closed today, it is
one row at the end of Today, muted, with no time in the lane (see
"Completed today" under "Agenda"); from the next day it is
off the calendar, and past days hold events only (the Done view is the
record of what was finished). A done row on the day it was planned for
could not say whether it was done then or some other day, and a tick at
a clock time claimed something that may never have happened. Showing
them on every past day was built first and dropped the same day: it
repeated the Done view and filled the past with days that held nothing
else. What was planned and got done, on which day, is no longer
readable off the calendar. An event keeps the rule below: it
happened on its `when` whatever was ticked.

**`when` survives Done; `deadline` does not** (decided 2026-09-16). A
ticked item still happened, or will happen, on its day, so it stays on the
calendar in the same slot: muted, tick shown, nothing reordered. A deadline is
"owed by", and Done settles the debt, so a done item's deadline places
nothing. Overdue is Open-only. **Cancelled follows Done** (decided
2026-10-14, reversing 2026-09-25): a cancelled item keeps its `when` slot,
muted with the cross, so the day records what was called off as well as
what happened; its deadline places nothing. The earlier rule dropped it from
the calendar entirely, and a cancelled event vanishing from its day read as
a missing record rather than a tidy one. The fields themselves stay on the
item (reopening brings the slot back). A checkless
"event" item kind was considered
and rejected: it would be a second item kind touching the board, Focus,
Overdue, repeat-on-done and the CLI, and would still need a close action
because the clock never writes. Ticking is that action.

**Events are a flag beside `when`, not a kind** (decided and built
2026-11-16, `events-plan.md`; from 2026-10-02 until then they were items
of a reserved `events` list). The rejection of a kind stands: the flag
says what the date means and changes one thing, what a past `when` means
(it went by). Where a dated item shows is Inbox's rule, not the flag's:
one with no list is here and not in Inbox (`inbox-plan.md`).
Its calendar row has no checkbox while it is open (2026-11-17, web, iOS,
macOS): a dot holds the box's place (an upright pill, a calendar's event bar, since
2026-11-18 on the web, iOS and macOS, as tall as the row), and the box shows, ticked or
crossed, once the event is done or cancelled from the item surface or
the row's menu (`events-plan.md`). The close-action
objection lapsed on 2026-09-30 when a past `when` stopped needing a
decision.

`when` and `deadline` are independent. Neither derives from the other and
neither is required by the other.

## Field: `when`

Item register, string, optional. Add to the `Item` table in `data-model.md`:

```
When      = Date | DateTime
Date      = YYYY "-" MM "-" DD                  ; 10 chars, floating calendar date
DateTime  = Date "T" HH ":" MM                  ; 16 chars, floating wall-clock
; reserved, rejected until fixed-instant support lands:
Fixed     = DateTime "[" IanaZone "]"           ; RFC 9557, e.g. 2026-07-13T14:00[Australia/Melbourne]
```

- Absent ≡ unset. Clearing deletes the key (as `deadline` does).
- Validation: `Date` uses the existing calendar-date check (month range,
  day-in-month, leap years). `DateTime` adds `HH` in `00..=23` and `MM` in
  `00..=59`. Seconds are never stored. Anything else, including a bracketed
  suffix, is rejected with `Invalid`. Input is trimmed before validation, as
  `parse_deadline` does; the stored value is the normalised form.
- **All-day** ≡ 10-character value. There is no default time. On the day,
  an all-day `when` is owed all day. Where it sorts depends on what it
  is: an all-day event leads the day, an untimed task follows the timed
  rows (see "Within a day" under "Agenda").
- **End time** is not stored; see "Field: `duration`" below.
- **Sort key** inside a band is the raw string: `2026-07-13T09:00` <
  `2026-07-13T14:00`. Until 2026-11-17 it was the whole within-day order
  (`2026-07-13` < `2026-07-13T09:00`, so untimed rows led their day); the
  bands of "Within a day" now come first.
- **Day key** is the first ten characters, shared with `deadline`. Every
  bucketing and comparison between the two fields runs on the day key.
- **Floating**: a `when` renders as the same wall-clock value on every device
  regardless of zone. Never construct a `Date` from the string with
  `new Date(stamp)` (UTC parse shifts the day in negative-offset zones); split
  the parts and build local, as `parseLocalDateParts` does. Time formatting
  respects the existing time-format preference (`format.tsx`).
- **Reserved suffix**: the validator accepts exactly 10 or 16 characters.
  When fixed instants land, the suffix becomes a strict superset, old data
  needs no migration, and an absent suffix continues to mean floating. An
  IANA name rather than an offset, because an offset pinned today is wrong for
  a future date after the next DST change. Old clients treat an unparseable
  value as absent, matching the `DefaultView` rule in `board.md`.

## Field: `duration` (added 2026-09-16)

Item register, integer minutes, optional. `1..=525600` (one year of days).

- **Why a length and not an end.** The tearing argument that put date and
  time in one register applies again. If device A moves the start while
  device B sets an absolute end, the item can end before it starts and
  something has to repair it. A duration is invariant under moving the
  start, so concurrent edits merge into a sensible item with no repair rule.
  It is also what every calendar preserves when a start is dragged, and
  overnight spans need no disambiguation: 23:00 for 120 minutes ends at
  01:00 next day. iCalendar allows `DURATION` in place of `DTEND`, so the
  export mapping is unchanged.
- **Ceiling (raised 2026-11-02).** `MAX_DURATION_MINUTES` is one year of
  days (`365 × 1440`). It is a typo guard against a stray digit in the
  CLI's `--duration 3d` form, not a model limit: mainstream calendars
  put no bound on a single event's span, and iCalendar's `DURATION` has
  none. The original week was chosen on 2026-09-16 when the register was
  a timed length only; when all-day spans began reading it as days
  (2026-10-07) the week quietly became a seven-day cap on trips and
  conferences, which is what the raise undoes. Recurrence is a separate
  question (`events-plan.md` "Deferred"): a span is one continuous thing,
  a repeat is many short ones, and the two compose rather than substitute.
- **Read against the shape of `when`.** The core never cross-checks the
  two registers (a check would reintroduce invalid states under
  concurrent edit). Beside a timed `when` the length is minutes from the
  start. Beside an all-day `when` it counts whole days (added
  2026-10-07, see "Multi-day all-day spans" below); beside an absent one
  views ignore it. Clearing `when` deletes `duration` in the same commit;
  timed → all-day keeps it so re-adding a time restores the end.
- **A start implies an end (added 2026-09-22).** `set_item_when` with a
  timed value on an item that has no duration writes the default of 60
  minutes (`DEFAULT_DURATION_MINUTES`) in the same commit and reports it
  with `ItemDurationChanged`. An existing length is never touched, so
  moving the start keeps the end moving with it. This is a default at
  write time, not a check: the registers stay independent under
  concurrent edit. Import writes the register directly and does not
  default. The task dialog's new-item buffer mirrors the rule so the end
  field appears as soon as a start time is typed.
- **Agenda placement** (changed 2026-10-29): a span is placed on every
  day it covers, from today onward, so an overnight span is on both its
  days and a three-day one on three (see "Agenda", Placement). Before
  that it sat on its start day only.
- **Multi-day all-day spans (added 2026-10-07).** A Tuesday-to-Thursday
  conference is an all-day `when` of Tuesday with a `duration` of three
  days in minutes (`3 × 1440`): the span covers `ceil(duration / 1440)`
  days, so its last day is that many minus one past the start, as
  iCalendar's `DURATION:P3D` beside an all-day `DTSTART` does. A length
  under a day beside an all-day `when` (left behind when a timed value's
  time is stripped) is a single day; so is no length at all, which is
  what a single-day item stores (picking the start day as the end drops
  the register). The register is the same one, so no core change: only
  the reading differs by the shape of `when`. Crossing shapes in the UI: timed → all-day
  reads the minutes as days by the rule above (Tue 09:00 → Thu 10:00 is
  `2 × 1440 + 60`, which rounds up to three days, Tue–Thu); all-day →
  timed reshapes a whole-day span of two or more days to end on the same
  last day with the default one-hour slot (`3 × 1440` → `2 × 1440 + 60`),
  since taken as minutes it would end a day late.
- **UI shows an end time, stores a length.** The task dialog's time row is
  start and end, the arrow glyph inset in the end field. The end field
  reads start + duration and renders only once a start time exists, and
  then always has a value via the default above. Neither time field has a
  clear control of its own; one ✕ after the end field strips the time
  (back to all-day), and the date input's ✕ removes the whole value.
  Committing an end writes the difference in minutes; an end at or before
  the start on the clock means the next day. Moving the start leaves the
  end shifting along with it.
- **End date (added 2026-10-07).** Under the end time, beside the date
  input, an end-date input reads the day the span ends on (start day +
  the days the length runs past it, by the all-day or timed reading
  above) and opens a plain calendar: no quick actions, no Remove, since
  the end is never absent, only implied. It shows whenever a date is set;
  while the end is the start day itself it reads dim, like a placeholder.
  Beside a timed start, picking a day keeps the end's clock time and
  writes the length up to it; days the register cannot hold are dimmed
  and inert (before the start day, the start day while the end is not
  past the start on the clock, and anything past the one-year ceiling).
  Once the end is on a later day, retyping the end time keeps that day
  rather than snapping back to the start day's next-day rule. Beside an
  all-day date, picking a day writes whole days (the start day drops the
  length); days before the start and past the seventh are inert.
- **Mutation** `set_item_duration(item_id, Option<u32>)`, event
  `ItemDurationChanged { id, duration }`, `ItemAdded.duration`, export
  `duration` (skipped when unset), wasm `setItemDuration`, CLI
  `monoplan item set <id> --duration <minutes | 1h30m | 3d | ->` and a `+<len>` suffix on
  the `@when` tag.

## Mutation and events

- `set_item_when(item_id, when: Option<&str>)`: `Some(value)` validates per
  the grammar and writes the `when` register with the normalised value;
  `None` deletes the key. One commit. Rejects malformed values with
  `Invalid`. Mirrors `set_item_deadline`, plus the `duration` coupling
  above (default on a timed set, clear on a clear).
- `AppEvent::ItemWhenChanged { id, when: Option<String> }`, the raw value
  after the write. `ItemAdded` gains a `when` field. The wasm event dispatch
  gains `itemWhenChanged` and `itemAdded.when`.
- Export dump (`ItemDump`) gains `when`, skipped when unset so existing dumps
  stay byte-identical. Import accepts it and validates.
- wasm: `setItemWhen(itemId, when?: string)` on both replica surfaces, next to
  `setItemDeadline`.
- Search (`spec/search.md`) does not tokenise either date. A date query
  belongs to the calendar lens, not the palette.
- Focus (`spec/focus.md`) is untouched. A `when` is not a deferred Focus
  entry; on its day the item surfaces in Today and the user pulls it into
  Focus from there. Focus stays curated.

## Agenda (Upcoming, generalised)

Upcoming keeps its `upcoming` token and shape; its nav entry reads
"Calendar" (renamed 2026-09-16; `calendar` stays reserved as a URL token
for the month grid). `groupByDeadline` becomes
`groupByDay` over both fields:

- **Rows** are Open items with a `deadline` or a `when` of today or later
  (or both), plus closed (Done or Cancelled) items with a `when` of today
  or later (see "`when` survives Done" above). A `when` before today places
  nothing on a closed item or an event, unless its span (below) still
  reaches today; on an Open task it places the row in Overdue.
  Binned items never appear.
- **Closed rows** place by `when` only, on that day, neutral tone, in the
  same within-day order as everything else, so ticking never moves a row.
  Their deadline places nothing and badges muted. A done item whose `when` is past is not in
  Overdue (Open-only) or on any day from today on: it is on its day in
  the past, above Today, on every client that reads past days (the web
  since 2026-11-17). On the web a closed task leaves its dates instead
  (see "Completed today" below), which stands in for the "show
  completed" toggle this paragraph used to promise.
- **Overdue** (web; iOS and macOS since 2026-11-17): any Open row with a `deadline` day before today goes
  to an Overdue section above Today, placed by the deadline whatever `when`
  says (it is owed now; a `when` of today or later still has its own row
  on its day). An Open task (not an event) whose `when` span
  ended before today goes there too, placed by its `when` (built
  2026-11-13, `isSlippedTask` in `dayGroups.ts`). Oldest first by the
  raw string of the placing field, then `created_at`, the two kinds
  interleaved. Rendered only when non-empty. The CLI `agenda` still folds
  these into Today, leading it in the overdue tone, and so does the
  engine's agenda the native apps read: their Calendar lifts those rows
  out of Today's group into the section (`AgendaTable.swift`,
  `AgendaCollection.swift`), and opens on the section's header and brings
  the "Today" button back to it when it is there, since the days gone by
  sit above it. The macOS tray and the iOS widget keep the fold.
- **Placement** of every other row (changed 2026-11-17, see "The two
  dates place independently"): the `when` places the item on its day,
  ignoring one whose span is over, and the deadline places a row of its
  own on its day (`placedBy: "deadline"`), unless the `when` covers that
  day, where the one row is the `when`'s (`isDueRow` in `dayGroups.ts`).
  The CLI and engine agenda apply the same rule per day
  (`build_agenda_from`): where both land on one day, Today's overdue
  fold included, the row is the `when`'s, toned by the more urgent of
  the two, or the deadline's when both dates are behind us. A deadline's
  own row shows no clock on the native calendars, whatever time the item
  is planned for on its other day.
- **Spans** (web, built 2026-10-29): a `when` with a `duration` covers
  `whenSpan` days (`dayGroups.ts`: whole days beside an all-day start,
  the clock end beside a timed one, so 23:00 + 2h is two days; the end
  is exclusive, so a span ending at 00:00 exactly stops on the day
  before, here and in the CLI's `when_end_day`, changed 2026-11-18). A row
  placed by such a `when` is repeated on every day of the span from
  today onward, each copy stamped with its position (`DayRow.span`),
  shown as "Day 2" on the sub line under the title, beside the workspace
  and place, on every day including the first; the total is not shown, and a timed
  span shorter than 24 hours (an evening that runs past midnight) carries
  no day label at all.
  The time lane reads what the day holds of the span (on an event; a
  task's lane is empty wherever "All day" would stand, since 2026-11-17:
  a task with no time is for the day, it does not fill it): "All day" on every
  day of an all-day span; for a timed one the start clock on the first
  day, "All day" on the days it runs through, and the end clock on the
  last ("until 03:00", since 2026-11-13). A span already under
  way starts on Today with its count intact. A row placed by its
  deadline, or sent to Overdue, is one row: the deadline is one day. The
  same id therefore appears down several days (a span's, and the days of
  a date and a deadline); the selection order and the view's flat item
  list keep each id once, at its first row. A selected item on several
  days draws its highlight on one row only: the one the pointer picked,
  else its earliest day on the page. Plain ↑ / ↓ step row by row (since
  2026-11-17), so each of an item's rows is a stop and the highlight moves
  to it; shift-ranges stay by item. The CLI `agenda` still places on
  the start day only.
- **Tone** is per row: a deadline's row is overdue (deadline day < today)
  or today-warning (deadline day = today); a `when`'s row is overdue (a
  task's `when` over) or today-warning (it carries a deadline that is
  today); otherwise neutral.
- **Within a day**, three bands (changed 2026-11-17; `day_band` in
  `cli/src/commands/agenda.rs`, `dayBand` in `dayGroups.ts`):
  1. **All-day events**: a row placed by a 10-character `when` on an
     item with the event flag. A birthday or a conference is the day's
     backdrop, so it leads, as on every calendar.
  2. **Timed rows**: placed by a 16-character `when`, tasks and events
     alike, in clock order. The day's fixed points.
  3. **Untimed tasks**: a row placed by a 10-character `when` on a
     task, and every row placed by a deadline. For the day at no
     particular time, so they come after what has one.

  Inside a band, the raw string of the field that placed the row, then
  `created_at`. A timed span's later days stay in the timed band and
  lead it (their raw string is an earlier day's). The band is the row's
  own (shape of `when`, event flag, placing field), never the lifecycle,
  so ticking still does not move a row. Why: a surface that cuts from
  the bottom (the widgets, the tray) keeps the timed rows, and as the
  day's timed events go by the untimed tasks are what is left. Overdue
  (and the CLI's overdue fold in Today) has no bands: it is by date.
  Every surface shares the one order: the web sorts in `groupByDay`, the
  CLI, tray, widgets and the Apple apps read `build_agenda_from`.
- **Badges**: the placing date is carried by the day header and not repeated,
  except in Overdue, where the deadline shows its actual date (existing
  `pastAsDate` rule). A task there by its `when` shows the planned day
  in the time lane ("7 Sept") in place of the clock or "All day". The other field, when present, shows as its own badge so a row
  reads "Sat 13 · due 31 Oct". Timed rows show the time as a leading label.
- Today is always present (after Overdue, when that exists), empty if
  nothing is due, so the surface anchors on the current day.
- **Nav badge** (web, built): the Upcoming entry carries a count of Open
  items owed or on today: an overdue deadline, a task `when` that is over
  (overdue tone), a deadline today, or a `when`
  on today, a span covering today included. Its tone is the most urgent counted (overdue red, deadline-today
  warning, otherwise the ordinary faint count); hidden at zero and not gated
  by the list-counts preference. Future days and done rows do not count, and neither
  does an event's past `when`: it has gone by (`attentionBadge` in
  `dayGroups.ts`).
- Not a `Dnd` listbox (day sections, rows of two heights), but a
  listbox in the list view's sense (web, built 2026-10-14): it runs the
  Workspace's `DndSelection` over every day's rows concatenated and answers
  the same keys through the shared `applySelectionKeyAction`, so ↑ / ↓ and
  shift-ranges cross day boundaries. Click selects and the side pane
  follows; Enter / double-click opens; `x`, `f`, `m`, ⌫, ⌘C, ⌘D act on the
  selection as on a list. ⌘↑ / ⌘↓ (reorder) and ⌘A (select all: the
  agenda reaches a year back) do nothing here.
- **Drag to reschedule** (web, built 2026-11-22, mouse only): a press on
  a row that moves a few px picks up the selection (the row selected
  first if it is not) under the list view's overlay stack; the day
  section under the pointer is the target, tinted, and the edges
  autoscroll. The drop writes `dropOnDay` (`dayGroups.ts`) for each
  carried item in one action batch, so one undo step; the rules are
  "Reschedule" under "Deferred" below. A row closed today does not pick
  up, and is left where it is when carried in a selection. Escape
  cancels. Not built: touch, ⌘↑ / ⌘↓ nudging by a day, the month grid.
- **Empty days** (web, 2026-11-22): by default every day has a section,
  from the first past day with something on it to the later of the last
  placed day and two weeks ahead (`fillEmptyDays`). "Hide empty days" in
  the view's filter menu (a device preference, unchecked by default, not
  counted as a filter and not cleared by Clear) shows only the days
  something is on. An empty day is its heading over a blank row-height
  slot (`data-drop-day`) saying "Nothing scheduled" (Today's "Nothing
  due today"). The filter menu is always on this view now; its
  workspace rows only with more than one doc.
- **Week breaks** (web, 2026-11-22): the day that opens a week (the
  locale's first day from `Intl.Locale` week info, else Monday; with
  days missing, the first day group of a new week) has "Week N" at the
  end of its header, N the ISO week of the
  Monday inside that week (`weekStarts`). Weekends are not marked: the
  heading's weekday already says so.
- **Windowed by day** (web, 2026-11-22): only the day sections from one
  pane's height above the pane to one below are in the DOM; spacers stand
  in for the rest at each day's last measured height, else an estimate
  from its rows. The window follows the scroll, keeping the top row where
  it is as estimates turn into measurements; a jump to an undrawn day (a
  far row, back to Today) draws it first.
- **Capture** (web): the Add button on this view opens the task dialog on
  a new event: no list, `when` prefilled to today and the Event box under
  it ticked. Unticking the box makes it a task with that date, still
  shown here and not in Inbox (`inbox-plan.md` "Membership").
- **Completed today** (web, built 2026-11-18, `groupByDay` in
  `dayGroups.ts`): a closed (done or cancelled) task, timed or not and
  with a `when` or only a deadline, that was closed today is placed on
  Today (`placedBy: "done"`, from `lifecycleAt`; a day ahead of this
  device's clock reads as today), after Today's other rows in the order
  closed, its title muted and its lane empty. One closed before today is
  not on the agenda. A row just closed lingers where it was (its planned
  day, Overdue) for three seconds first, as on a list. Events do not
  move: they stay on their `when` as the day's record. This narrows
  "ticking never moves a row" to events; the reason is that a day loaded
  with small tasks became noise to read through once half were ticked.
  A "N completed items" line the rows folded behind was built first and
  dropped the same day for showing them outright.
  The engine's agenda follows the same rule since the same day
  (`build_agenda_from`, `placed_by: done`), so the CLI and the iOS and
  macOS calendars show the same rows, muted and with no time, without
  the linger: a ticked row moves at once. Two differences stay: the
  engine never lists a cancelled item, so only done tasks are there; and
  the Mac tray and the iOS widget leave done tasks out altogether, being
  what is left of the day (`AgendaQuery.completed`).
- **Past mark** (web, 2026-11-18): on a day gone by, an event's row
  leads with the event pill in place of the
  the checkbox, open or done alike: they are a record of what was on,
  and nothing there asks for a tick. A cancelled one leads with a bare
  cross, so the day still says what was called off.
- **List** (web, 2026-11-18; iOS the same day, from the agenda row's
  `list_name` / `list_icon`; macOS already named the list and now leads
  it with its icon): a row whose item is in a list names it on
  the sub line under the title (the list's own icon, or the default list glyph, + name), after the workspace
  and before the place. An item with no list, which lives on the
  calendar alone, shows nothing there.
- **Events** (`events-plan.md`): an event's row is a task's row, except
  that while open it leads with a dot, not a checkbox (2026-11-17; the
  upright event pill in the dot's place on the web, 2026-11-18).
  Everything else above applies to events unchanged. An event always has a
  `when`, so there is no Unscheduled section (there was one while events
  were a list, removed 2026-11-16).
- **Mini calendar** (web, built 2026-11-22): a month grid pinned at
  the top of the sidebar, above the nav list, for orientation. It is the date pickers' corvu
  grid (`MonthGrid.tsx`) with nothing selected and today marked. A
  click on a day opens the agenda with that day's header at the top of
  the pane. With empty days shown, the agenda's range widens to take in
  the picked day until the agenda is left. With them hidden, the jump
  lands on the next day that has rows. "Show mini calendar" in the
  sidebar's blank-space context menu (beside "Show list counts") shows
  and hides it, a device preference (shown by default). The URL
  does not change: the `upcoming_<day>` anchor (§URLs) stays unbuilt.

## Task surface and rows

- The task dialog and side panel gain a **When** control beside Deadline,
  same field-with-popover pattern as `DeadlineField`: Today, Tomorrow, the
  calendar grid, Remove. The time (blank ≡ all-day) has its own row in the
  dates band, shown once a date is set. Changing the date keeps the time;
  Remove clears both.
- **Time picker** is the app's own typed control (`TimePicker.tsx`): a
  plain text input showing the stored time in the app's hour cycle and, while
  focused, a listbox of suggestions (every quarter hour, narrowed by what is
  typed; `timeSuggest.ts` has the grammar). Enter or a click commits the row
  under the cursor; Escape, Tab and blur revert to the stored value. It is
  the one time control on every surface: the task dialog's dates band (start
  and end). Kobalte's segmented `TimeField` was the first cut and has been
  removed.
- **Hour cycle** follows the time-format preference, not the locale alone.
  `format.tsx` gains `hourCycle(locale): 12 | 24`: `"12h"` → 12, `"24h"` →
  24, `"auto"` → whatever `Intl.DateTimeFormat(locale, { hour: "numeric" })`
  resolves to (`h11` / `h12` → 12, `h23` / `h24` → 24). The result is passed
  as the picker's `cycle` every time, never left to a component default, so
  the picker and every formatted time in the app agree by construction.
  Changing the preference in Settings re-renders the picker in the new
  cycle; the stored value is unaffected (it is always 24-hour `HH:MM`).
- **Value bridge.** The picker's value is `{ hour, minute }`, plain numbers,
  no date library, or null for all-day. The 16-character register maps to
  the pair; the 10-character register maps to null. A commit is always a
  complete pair or null, so there is no partial state to hold: against a set
  date every commit writes through (timed, or back to all-day with the date
  kept), and the time row only shows against a set date. The inset ✕ on the
  picker is how the user drops back to all-day without removing the date.
- The calendar grid (`CalendarPicker`) is shared with Deadline and carries
  no time field: the time is edited in the task surface's dates band.
  Neither register has a surface outside the task dialog / side panel (the
  standalone Deadline calendar modal was dropped 2026-10-08; its "Set
  deadline…" now opens the item on the deadline popover, as Set date… does).
- **The deadline is a menu-only register** (web, decided 2026-11-17). An
  item without one shows no deadline control: "Set deadline…" is in the
  task surface's "…" menu (and the selection panel's actions, `SelectionPanel.tsx`),
  and opens the item on its deadline band with the calendar showing. It
  was in the row context menu too until later the same day; that menu now
  only removes. Once set,
  the band sits under the date and time section, where it is changed and
  removed; the row context menu (one Open row) then offers "Remove
  deadline". The "+" chip
  that sat beside the list picker for this is gone, and so is the
  new-item capture's deadline buffer: a capture has no row or "…" menu
  yet, so a deadline is added once the item exists. The band's input is
  the planned-date input without the time (same day): typed into, with
  the suggestion list above the calendar (Today / Tomorrow while unset),
  Enter on emptied text removing it; typed times are not read. The native apps show
  a set deadline and have no way to set one yet.
- `WhenBadge` beside `DeadlineBadge` on list rows and board cards. When both
  are set, `when` renders first. Muted on done/binned items as deadline is.
- Row context menus gain the same quick actions for When (Today, Tomorrow,
  Remove, acting on the whole target set). Set date… opens the item with its
  date popover showing, so it is offered for a single row only.
- i18n: a `when` message group mirroring `deadline` (label, unset, today,
  tomorrow; time-of-day formatting defers to the existing
  preference).
- **Typed date phrases** (built 2026-11-03; reworked 2026-11-13 from an
  inline title token to a suggestion). A date phrase typed into a title
  ("friday night", "tomorrow at 5pm", "14 nov") raises a **date
  suggestion**: "Suggested: Fri 14 Nov 08:30", shown in orange where
  the date would be, answered with a tick or a ✕. The title is never
  rewritten and no date is set without the yes. The rule is `whenSuggest.ts`, the
  control `WhenSuggestion.tsx`; the grammar is `whenParse.ts` over
  chrono-node's casual parser, English or Spanish by locale (about 14 KB
  gzipped per language).
  - The question is the item's `when_suggested` register
    (`spec/data-model.md`): a resolved `when`-shaped value. The phrase is
    resolved once, against the moment it was typed, so "friday" cannot
    drift to a later Friday, and a client with no parser (CLI, iOS) can
    show and answer it. It syncs like any register: capture fast, answer
    later, on any device.
  - It is written when a title edit brings a date phrase the title did
    not have before (compared by wording, case and spacing folded): on
    the item a draft row or capture creates, and on a rename as it is
    saved. Only a *changed* phrase writes one, so nothing is kept of a
    No: the key is deleted, and fixing a typo elsewhere in the title
    does not ask again, while changing the phrase ("friday" → "friday
    night", "saturday") does. A phrase edited out of the title takes its
    unanswered suggestion with it. A title that arrives by sync or undo
    raises nothing here: whoever typed it did.
  - Yes moves the value into `when` (one commit, one undo step; the
    default hour's `duration` beside a timed value, as any `when` write
    gives). Setting a date any other way also clears the suggestion.
  - It shows only in the task surface, on an open item with no `when`:
    in the dates band in place of the date input, shaped like it, with
    a tick and a ✕ at the right edge (a "Set date…" from a menu still
    opens the field). While a title is being
    typed there the question shows live, before anything is written,
    and is answered straight away (Yes sets the date, No just marks the
    wording as asked). Lists and boards show nothing of it (an in-row
    badge was built and dropped the same day, 2026-11-13: a question
    does not belong in a list): an item captured from a draft row still
    gets its suggestion, seen when the item is opened.
  - chrono matches whole words only, so a half-typed "frida" or a
    suffixed "tomorrows" is never a phrase. Only a phrase that pins a
    day (explicit day, weekday, relative day) or a clock time is taken.
    A bare month, "next month", and a three-letter weekday on its own
    ("sun", "wed") are not. Of several phrases the one ending last wins.
  - A clock time gives a timed value; a bare 1–6 with no am/pm reads as
    afternoon, as the time picker orders it. A time-of-day word gives an
    estimated time (morning 09:00, afternoon 15:00, evening 18:00,
    night / tonight 20:00; Spanish "por la mañana", "tarde", "noche");
    being a question, it is not marked as estimated. The word is matched
    in the phrase itself, since chrono implies an hour on phrases that
    said nothing about time. A phrase with neither is all-day. A phrase
    that is only a time ("at 3", "3pm") is today if that time is still
    ahead, else tomorrow. The value is built from the parsed components,
    never a `Date`, so it stays floating.
  - Off for a closed item and for a log-a-completion capture: a date is a
    plan, and a done item has none to make. There is no setting to
    turn it off (the "Recognise dates as you type" switch went
    2026-11-13): a suggestion changes nothing until it is answered.
  - CLI (2026-11-13): `item set --when-suggested <when | ->` raises or
    dismisses one (`spec/cli.md`); the JSON carries `when_suggested`,
    and `set --when` answers yes by setting a date. The CLI reads no
    phrases out of titles.
  - iOS (2026-11-13): the same question on the item sheet, in the date
    row's place, orange with a tick and a cross (`ItemDateField.swift`).
    It is raised when a capture creates the item and when the title
    editor saves a title with a new phrase (`WhenSuggest.edit`, the
    `whenSuggest.ts` rule). The grammar is not chrono's: the recogniser
    is Foundation's `NSDataDetector` (`WhenPhrase.swift` in
    MonoplanMacKit; no chrono port worth having, and no JavaScript in
    the native apps), with our all-day / timed split, time-of-day hours,
    time-only roll-forward and no past days on top. So the two clients
    can disagree at the edges: the detector has no bare hour ("at 3")
    and no "in an hour", and takes a meal word as a time ("lunch
    saturday" is Saturday 12:00). All-day versus timed rests on the
    detector's undocumented `timeIsSignificant`, with "anything but
    12:00 is a time" as the fallback. Not built on iOS: the live
    question while the title is still being typed.
  - The When field's date input takes a typed phrase too (2026-11-11):
    the popover keeps the calendar below a
    list of the first few days the text could mean (`dateSuggest.ts`: the text
    as typed, a trailing partial word completed to "today" / "tomorrow" /
    a weekday / a month, a bare day number in each coming month, a lone
    repeating phrase as its next three occurrences). The start input keeps typed times: an ambiguous clock time such as
    "Fri 9:30" offers both 09:30 and 21:30, while explicit AM/PM or
    24-hour times have one reading per day. Choosing a timed suggestion
    sets both date and time; date-only choices keep any existing time.
    The end-date input continues to write the day only.
    With no start date or typed phrase, Today and Tomorrow are the default
    suggestions. Typing or arrowing through suggestions previews the day
    with an outline in the calendar, bringing its month into view; only
    clicking or pressing Enter commits the suggestion. Pointer hover does
    not change the preview or month, so reaching for the calendar keeps it stable.
    The end-date input does the same, reading phrases from the start
    day once that is ahead of today and leaving out days the span
    cannot end on. Enter on its emptied text drops the end date: the
    span returns to the start day, keeping the end's clock time when
    that is after the start, else the default hour.
  - Not built: typing a phrase into the Deadline field (still a
    read-only popover anchor), and any client outside the web.

## URLs

No new token in the first cut. `upcoming` stays the agenda. Two forms are
reserved so later phases add without renaming:

- `calendar`, for a month-grid lens.
- A day anchor with an underscore, `upcoming_2026-07-13` and
  `calendar_2026-07`.

Amend `spec/urls.md` with the reservation only. (Built 2026-11-05: an
*item* anchor in the same shape, `upcoming_<id>`, opens the item over
the agenda whatever its list, for the macOS tray's Today rows. A day is
not an id, so the day anchor stays free.)

## CLI

`spec/cli.md` Items gains:

- `monoplan item set <item_id> --when <YYYY-MM-DD[THH:MM] | ->`: set or clear (`-`) the
  `when` register.
- `monoplan item set <item_id> --deadline <YYYY-MM-DD | ->`: set or clear the deadline.
  The CLI has no deadline verb today; add both together.
- `monoplan agenda [--days N]`: the agenda as text, one day section per line
  group, default 14 days plus Today's fold.
- `ls` shows a trailing `@<when>` and `!<deadline>` when set.

## iCalendar mapping (recorded, not built)

An Monoplan item is a `VTODO`: `DTSTART` is `when`, `DUE` is `deadline`. That
is the correct shape, but calendar displays ignore `VTODO` (Apple Calendar,
Google Calendar; only Reminders-style apps and CalDAV task clients read them).
Anything meant to appear on a calendar must be a `VEVENT`:

| Monoplan | iCalendar |
|---|---|
| `when` = `2026-07-13` | `DTSTART;VALUE=DATE:20260713`, `DTEND;VALUE=DATE:20260714` (exclusive end, added at export) |
| `when` = `2026-07-13T14:00` | `DTSTART:20260713T140000` (floating: no `Z`, no `TZID`). Apple renders in the viewer's zone; Google pins to the calendar's zone at import. |
| `when` = `...T14:00[Zone]` (future) | `DTSTART;TZID=Zone:20260713T140000` |
| `deadline` | all-day `VEVENT` with a "Due:" summary prefix, since `VEVENT` has no due slot |

Delivery paths, all client-side because the server cannot read items: a
one-shot `.ics` export from web or CLI; native clients writing through
EventKit (which does take `VTODO` into Reminders). A server-side feed or
CalDAV endpoint would need an explicit non-E2EE carve-out and is out of scope
here.

## Deferred, with their extension points

- **Recurrence.** Likely form: an optional `repeat` register `{ every, unit }`
  with `unit` in day/week/month/year; ticking Done spawns a successor with
  `when` advanced and the done item kept, so creation-to-done stats stay
  honest. No RRULE. This form covers tasks only: an event
  (`events-plan.md`) is never ticked, so recurring events are unsolved.
- **Fixed instants.** The bracketed suffix above. Flipping the default means
  writers append the device zone; sorting then needs instant normalisation
  for mixed values, which is why it waits.
- **Month grid.** A workspace-level lens, sibling of Upcoming, on the
  `calendar` token: one month at a time on `@corvu/calendar` (already under
  the date picker), a few row titles per cell in placement order with a
  "+k" overflow, tone per the agenda rules (past cells show the `when` rows that went by
  on them; overdue deadlines stay in Overdue), click or Enter on a day opens the agenda
  anchored on it. Needs the agenda to accept an anchor day. Nothing in the
  data model changes for it.
- **Reschedule** (agenda drag built 2026-11-22, see "Agenda"; the grid
  is not). Drag a row between agenda day sections or grid cells. A
  row placed by `when` moves its `when` with the time part kept; a row placed
  by `deadline` only moves its deadline; a row with both moves `when`, the
  deadline being a commitment that moves only through the explicit control.
  Decided 2026-11-22: a multi-day span moves its start to the drop day
  (whichever day of the span was grabbed), keeping its `duration`; rows
  drag out of Overdue; any day takes a drop, past days included (not
  Overdue, which is not a day); a multi-selection lands every item on the
  drop day (not shifted by a common offset). The drop target is the whole
  day section, an empty day's blank slot included.
  Still to do: ⌘↑ / ⌘↓ nudging the day by one, and touch.
- **Export**, per the mapping above.
- **CalDAV carve-out.** Separate conversation.
- **Week view, hour grid.** Not planned. (A `duration` register landed
  2026-09-16 without either; see "Field: `duration`".)

## Testing

- Core unit tests: grammar acceptance (10 and 16 chars, hour and minute
  bounds, trimming, normalisation), rejection of seconds, offsets, bracketed
  suffixes; set/clear round-trip; event payloads; dump round-trip with and
  without `when`.
- `deadlineGroups.test.ts` becomes the `groupByDay` suite: placement day,
  clamping, tone precedence, within-day ordering including timed rows, items
  with both fields.
- `format.test.ts`: `hourCycle` for each preference and for `"auto"` under a
  12-hour and a 24-hour locale; register ⇄ `{ hour, minute }` bridge
  including the empty and partial cases.
- CLI system test: `when` set on one device, observed on the other after
  sync, cleared, observed cleared.
- Web: typecheck plus source reading (no browser automation here).

## Phases

0. **Core, wasm, CLI.** Built. Field, validator, mutation, events, dump,
   wasm bindings, `when` / `deadline` / `agenda` verbs (`agenda` takes
   `--today` so scripts and tests pin the date). `data-model.md` and
   `cli.md` amended. Unit and system tests.
1. **Web field and agenda.** Built. Store field and mutation, `WhenBadge`,
   `WhenField` with the typed time picker and `hourCycle`, `groupByDay`
   (`dayGroups.ts`, replacing `deadlineGroups.ts`), agenda tone rules and
   leading time label, When submenu on row context menus and the palette,
   duplicate / paste carry `when`, i18n. `urls.md` amended for the reserved
   tokens.

Month grid and reschedule are deferred, not phased; see "Deferred, with their
extension points".
