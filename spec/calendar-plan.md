# Calendar: plan

**Status: built 2026-09-09 (Phase 0 core / wasm / CLI and Phase 1 web).
Decided 2026-09-09, trimmed the same day.** Adds
a second date to items, `when`, alongside the existing `deadline`, and grows
the Upcoming view into a day-granular agenda over both dates. The first cut
is the basics only: the field, the agenda, and a When control with an
optional time. No month grid, no drag-to-reschedule, no recurrence rules, no
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
| Calendar surface | **One lens: Upcoming becomes the agenda** (day sections, both dates). Day granularity only: no hour grid, no overlap. Items carry an optional `duration` (added 2026-09-16, see "Field: `duration`"), which the agenda reads for multi-day all-day spans and the end-time display but never lays out on an hour axis. A month grid and drag-to-reschedule are deferred; the agenda's shape does not change when they land. |
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

**A task's past `when` is overdue** (decided 2026-11-13). An Open item
outside the `events` list whose `when` is over (its whole span ended
before today) joins the Overdue section, placed by its `when`, overdue
tone, whether or not it has a deadline still ahead: the placement day is
the earlier of the two dates and that one is behind us. If its deadline is
overdue as well, the deadline places the row, as for any overdue deadline.
Nothing is written: the `when` keeps its value, and the row leaves Overdue
when the task is ticked, cancelled, re-planned, or has its `when` cleared.
Moving the item into `events` takes it out too, since an event goes by.

**An event's past `when` is not judged and not surfaced** (decided
2026-09-30 for every item, replacing an earlier rule that listed it under
Overdue for the user to decide; narrowed to events 2026-11-13). It went by
and lives on its day in the past: it does not roll over into Today, is not
rewritten by the clock, and places nothing on the agenda, which renders no
past days. An event with a past `when` and a deadline is placed by the
deadline alone. Past days become visible when a calendar view that can
reach them is built, or when the agenda learns to scroll backwards.

Done and binned items keep both fields untouched, as they keep `deadline`
today. The fields are never cleared by a transition. Restore brings the
dates back with the item.

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

**Events get a reserved list, not a kind** (decided and built 2026-10-02,
`events-plan.md`). The rejection of a kind stands. An item
located in the reserved list `events` surfaces only on this calendar, with
a checkbox nothing asks it to tick; nothing about the item itself differs, and the
close-action objection lapsed on 2026-09-30 when a past `when` stopped
needing a decision.

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
  an all-day `when` is owed all day and sorts ahead of every timed row.
- **End time** is not stored; see "Field: `duration`" below.
- **Sort key** is the raw string. `2026-07-13` < `2026-07-13T09:00` <
  `2026-07-13T14:00`, so untimed rows lead their day with no special casing.
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
  Their deadline places nothing and badges muted. A done item whose `when` is past drops out:
  Overdue is Open-only and the agenda renders no past days. A month grid or
  backward agenda, when built, shows them on their day with a "show
  completed" toggle if the noise warrants one.
- **Overdue** (web): any Open row with a `deadline` day before today goes
  to an Overdue section above Today, placed by the deadline whatever `when`
  says (it is owed now). An Open task (not an event) whose `when` span
  ended before today goes there too, placed by its `when` (built
  2026-11-13, `isSlippedTask` in `dayGroups.ts`). Oldest first by the
  raw string of the placing field, then `created_at`, the two kinds
  interleaved. Rendered only when non-empty. The CLI `agenda` still folds
  these into Today.
- **Placement** of every other row: an item is placed on its *placement
  day*, the earlier of its `when` day and its `deadline` day, ignoring a
  `when` whose span is over.
- **Spans** (web, built 2026-10-29): a `when` with a `duration` covers
  `whenSpan` days (`dayGroups.ts`: whole days beside an all-day start,
  the clock end beside a timed one, so 23:00 + 2h is two days). A row
  placed by such a `when` is repeated on every day of the span from
  today onward, each copy stamped with its position (`DayRow.span`),
  shown as "Day 2" on the sub line under the title, beside the workspace
  and place, on every day including the first; the total is not shown, and a timed
  span shorter than 24 hours (an evening that runs past midnight) carries
  no day label at all.
  The time lane reads what the day holds of the span: "All day" on every
  day of an all-day span; for a timed one the start clock on the first
  day, "All day" on the days it runs through, and the end clock on the
  last ("until 03:00", since 2026-11-13). A span already under
  way starts on Today with its count intact. A row placed by its
  deadline, or sent to Overdue, is one row: the deadline is one day. The
  same id therefore appears down several days; the selection order and
  the view's flat item list keep each id once, at its first row. A
  selected span draws its highlight on one copy only: the one the pointer
  picked, else its earliest day on the page. The CLI `agenda` still places on
  the start day only.
- **Tone** of a row is the most urgent of: overdue (deadline day < today,
  or a task's `when` over), today-warning (deadline day = today), neutral.
- **Within a day**, order by the raw string of the field that placed the row,
  then `created_at`.
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
  by `showListCounts`. Future days and done rows do not count, and neither
  does an event's past `when`: it has gone by (`attentionBadge` in
  `dayGroups.ts`).
- Not a `Dnd` listbox (day sections, rows of two heights, nothing
  virtualised: the agenda is bounded to overdue plus today onward), but a
  listbox in the list view's sense (web, built 2026-10-14): it runs the
  Workspace's `DndSelection` over every day's rows concatenated and answers
  the same keys through the shared `applySelectionKeyAction`, so ↑ / ↓ and
  shift-ranges cross day boundaries. Click selects and the side pane
  follows; Enter / double-click opens; `x`, `f`, `m`, ⌫, ⌘C, ⌘D act on the
  selection as on a list. ⌘↑ / ⌘↓ (reorder) does nothing here.
  Drag-to-reschedule is deferred (see below).
- **Capture** (web): the Add button on this view opens the task dialog on
  a new event, `when` prefilled to today; the dialog's list picker
  re-files it as a task.
- **Events** (`events-plan.md`): an event's row is a task's row with no
  list label. Open events with neither date
  sit in an **Unscheduled** section between Overdue and Today, oldest
  first, rendered only when non-empty, and the nav badge counts them at
  neutral tone. Everything else above applies to events unchanged.

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
- `WhenBadge` beside `DeadlineBadge` on list rows and board cards. When both
  are set, `when` renders first. Muted on done/binned items as deadline is.
- Row context menus gain the same quick actions for When (Today, Tomorrow,
  Remove, acting on the whole target set). Set date… opens the item with its
  date popover showing, so it is offered for a single row only.
- i18n: a `when` message group mirroring `deadline` (label, unset, today,
  tomorrow; time-of-day formatting defers to the existing
  preference).
- **Typed date phrases** (built 2026-11-03). A date phrase typed into a
  title ("friday night", "tomorrow at 5pm", "14 nov") becomes a `when`.
  One helper (`whenToken.ts`) serves the draft row and the task surface's
  title; the grammar is `whenParse.ts` over chrono-node's casual parser,
  English or Spanish by locale (about 14 KB gzipped per language).
  - The text is scanned on every input. The moment a phrase parses it is
    wrapped in place as a non-editable token that still shows the words
    typed; the resolved date is its hover text. chrono matches whole
    words only, so a half-typed "frida" or a suffixed "tomorrows" is
    never a phrase. The token is a live reading of the words around it:
    typing on after it grows it ("friday" → "friday night" → "friday
    night at 9"), typing ahead of it can grow it too ("next friday"), and
    words that stop parsing dissolve back into plain text. One token at
    most; another phrase elsewhere is left alone while one stands.
  - Backspace just after the token (Delete just before it, or either key
    from inside it) dissolves it into its plain words and rejects that
    phrase: the words stay in the title, the next Backspace eats a
    character of them as usual, and the same wording is not taken again
    while it stands, while a different or longer phrase is. A rejection
    lasts only as long as its exact wording is in the text: edit the
    words (a letter off, a letter back) and the phrase is eligible again.
    Deleting the token any other way rejects the same. Rejections also
    clear when the editor is next focused.
  - Only a phrase that pins a day (explicit day, weekday, relative day)
    or a clock time is taken. A bare month, "next month", and a
    three-letter weekday on its own ("sun", "wed") are not. Of several
    phrases the one ending last wins.
  - A clock time gives a timed `when`; a bare 1–6 with no am/pm reads as
    afternoon, as the time picker orders it. A time-of-day word gives an
    **estimated** time (morning 09:00, afternoon 15:00, evening 18:00,
    night / tonight 20:00; Spanish "por la mañana", "tarde", "noche"),
    stored as a timed `when` and shown with a tilde. The word is matched
    in the phrase itself, since chrono implies an hour on phrases that
    said nothing about time. A phrase with neither is all-day. A phrase
    that is only a time ("at 3", "3pm") is today if that time is still
    ahead, else tomorrow. The value is built from the parsed components,
    never a `Date`, so it stays floating.
  - The value is held in the token, not applied, while the title is being
    typed, so a date the item already has is not overridden mid-thought.
    Leaving the title commits it: the draft row sets `when` on the item
    it creates (Enter, Escape, blur); the task surface, on blur, Enter,
    close or the target changing, applies it to the When field (the
    buffer for a capture, the item otherwise) and strips the words from
    the editor, leaving the plain title with the date beside it. The
    saved title is always the text without the token's words.
  - Off for a closed item and for a log-a-completion capture: a date is a
    plan, and a done item has none to make. A standing token still
    commits. Off everywhere under Settings › General › "Recognise dates
    as you type", a device preference in a cookie beside the time format.
  - The When field's date input takes a typed phrase too (2026-11-11):
    while the text names a date its popover swaps the calendar for a
    list of the first few days it could mean (`dateSuggest.ts`: the text
    as typed, a trailing partial word completed to "today" / "tomorrow" /
    a weekday / a month, a bare day number in each coming month, a lone
    repeating phrase as its next three occurrences). It writes the day
    only; a typed time is dropped, the time row owns that.
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
- **Reschedule.** Drag a row between agenda day sections or grid cells. A
  row placed by `when` moves its `when` with the time part kept; a row placed
  by `deadline` only moves its deadline; a row with both moves `when`, the
  deadline being a commitment that moves only through the explicit control.
  The agenda already carries a selection; what is missing is a pointer
  drag that hit-tests day section rects, or ⌘↑ / ⌘↓ nudging the day by one.
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
