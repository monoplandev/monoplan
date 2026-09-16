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
| Why a second field? | `deadline` means "owed by": past it the item is overdue and red. A planned or scheduled day means "happens on" or "act on": past it the item is not owed, it slipped. Using `deadline` for the second meaning makes every past event scream overdue. The distinction is what happens after the date, so it needs its own field. |
| Replace `deadline`, or flag it? | **Neither. Add `when` beside it.** Both can coexist on one item ("do it Saturday, due Oct 31"), which a single date plus a kind flag can't express. Additive, so six months of real data needs no migration. Same names as Things, which is the model people already know. |
| Date and time: one register or two? | **One register, `when`, shape-discriminated.** `YYYY-MM-DD` is all-day, `YYYY-MM-DDTHH:MM` is timed. Same rule as iCalendar `VALUE=DATE` vs `DATE-TIME`. One register can't tear under concurrent edit (date moved on one device, time set on another), sorts by plain string compare, and clears with one delete. An explicit flag is redundant with the shape and a third thing to keep consistent. |
| Time zone? | **Floating only, now.** A `when` is a wall-clock intent, the devices travel together, and floating keeps sorting a string compare. The grammar reserves an RFC 9557 bracketed IANA suffix (`...T14:00[Europe/London]`) for fixed-instant values. Writers may later default to appending the device zone; untouched values stay floating, so nothing migrates. Note this is the task-manager default, not the calendar default: Apple and Google pin timed events to a zone. |
| Recurrence? | **Not in the first cut.** No `repeat` field, no hint of one in the doc. Repeat-on-done (clone with the date advanced when ticked Done) is the likely first form; RRULE-style schedules are a calendar app's job. |
| Does the clock ever write? | **Never.** A past `when` stays where it was set; the agenda surfaces it in the Overdue section as a derived view rule. Nothing promotes an item to Live, adds it to Focus, or moves it because a day arrived: every device would race to do it. |
| What does a past `when` mean? | **No opinion yet.** Events and tasks are not yet distinguished: a past `when` on an event is simply over, on a task it may have slipped. Until that distinction exists, `when` is fixed and does not roll over: it is never rewritten, never reinterpreted as "today", and never red. The agenda only lists it under Overdue so the user can decide. |
| Calendar surface | **One lens: Upcoming becomes the agenda** (day sections, both dates). Day granularity only: no hour grid, no durations, no overlap. A month grid and drag-to-reschedule are deferred; the agenda's shape does not change when they land. |
| Time input | **Kobalte `TimeField`**, segmented hour / minute, 12 or 24-hour cycle from the existing time-format preference. Blank means all-day. See "Task surface and rows". |
| Export / CalDAV | **Deferred.** Mapping is recorded below. A subscribable feed needs a server that can read items, which the E2EE server cannot; a non-E2EE CalDAV carve-out is a separate conversation. |
| Schema | **Additive within v4.** No break, no import step. |

## Semantics

| | before the day | on the day | after the day, still Open |
|---|---|---|---|
| `deadline` | upcoming | Today, warning tone | Overdue section, overdue tone (red) |
| `when` | upcoming | Today, neutral tone | Overdue section, muted tone, never red |

A past date of either kind moves the row to an Overdue section above Today
(decided 2026-09-16, replacing the earlier fold into Today): the pile that
needs a decision is visible at a glance and Today reads as today's plan.
The section exists only while something is past.

A past `when` is deliberately not judged. Whether it slipped or simply
happened depends on whether the item is a task or an event, and that
distinction is not made yet. The conservative rule for now: `when` is fixed.
It does not roll over into Today, is not rewritten by the clock, and carries
no tone or label of its own in Overdue until the user ticks, bins, or
reschedules the item. There is deliberately no "slipped" state.

Done and binned items keep both fields untouched, as they keep `deadline`
today. The fields are never cleared by a transition. Restore brings the
dates back with the item.

**`when` survives Done; `deadline` does not** (decided 2026-09-16). A
ticked item still happened, or will happen, on its day, so it stays on the
calendar in the same slot: muted, tick shown, nothing reordered. A deadline is
"owed by", and Done settles the debt, so a done item's deadline places
nothing. Overdue is Open-only. A checkless "event" item kind was considered
and rejected: it would be a second item kind touching the board, Focus,
Overdue, repeat-on-done and the CLI, and would still need a close action
because the clock never writes. Ticking is that action.

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

Item register, integer minutes, optional. `1..=10080` (one week).

- **Why a length and not an end.** The tearing argument that put date and
  time in one register applies again. If device A moves the start while
  device B sets an absolute end, the item can end before it starts and
  something has to repair it. A duration is invariant under moving the
  start, so concurrent edits merge into a sensible item with no repair rule.
  It is also what every calendar preserves when a start is dragged, and
  overnight spans need no disambiguation: 23:00 for 120 minutes ends at
  01:00 next day. iCalendar allows `DURATION` in place of `DTEND`, so the
  export mapping is unchanged.
- **Only meaningful beside a timed `when`.** The core never cross-checks
  the two registers (a check would reintroduce invalid states under
  concurrent edit); views ignore a duration beside an all-day or absent
  `when`. Clearing `when` deletes `duration` in the same commit; timed →
  all-day keeps it so re-adding a time restores the end.
- **Agenda placement is unchanged.** Day granularity on the start; an
  overnight span appears on its start day only.
- **Multi-day all-day spans** (a Tuesday-to-Thursday conference) are not
  expressed. Minutes are the wrong unit for that; revisit if it comes up.
- **UI shows an end time, stores a length.** The task dialog's time row is
  start, arrow, end. The end field reads start + duration and renders only
  once a start time exists. Committing an end writes the difference in
  minutes; an end at or before the start on the clock means the next day.
  Moving the start leaves the end shifting along with it.
- **Mutation** `set_item_duration(item_id, Option<u32>)`, event
  `ItemDurationChanged { id, duration }`, `ItemAdded.duration`, export
  `duration` (skipped when unset), wasm `setItemDuration`, CLI
  `monoplan duration <id> <minutes | 1h30m | ->` and a `+<len>` suffix on
  the `@when` tag.

## Mutation and events

- `set_item_when(item_id, when: Option<&str>)`: `Some(value)` validates per
  the grammar and writes the `when` register with the normalised value;
  `None` deletes the key. One commit. Rejects malformed values with
  `Invalid`. Mirrors `set_item_deadline` exactly.
- `AppEvent::ItemWhenChanged { id, when: Option<String> }`, the raw value
  after the write. `ItemAdded` gains a `when` field. The wasm event dispatch
  gains `itemWhenChanged` and `itemAdded.when`.
- Export dump (`ItemDump`) gains `when`, skipped when unset so existing dumps
  stay byte-identical. Import accepts it and validates.
- wasm: `setItemWhen(itemId, when?: string)` on both engine surfaces, next to
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

- **Rows** are Open items with a `when` or a `deadline` (or both), plus
  Done items with a `when` (see "`when` survives Done" above). Binned items
  never appear.
- **Done rows** place by `when` only, on that day, neutral tone, in the
  same within-day order as everything else, so ticking never moves a row.
  Their deadline places nothing and badges muted. A done item whose `when` is past drops out:
  Overdue is Open-only and the agenda renders no past days. A month grid or
  backward agenda, when built, shows them on their day with a "show
  completed" toggle if the noise warrants one.
- **Overdue** (web): any Open row with a `deadline` day or a `when` day
  before today goes to an Overdue section above Today. Overdue deadlines lead,
  placed by the deadline whatever `when` says (it is owed now), oldest
  first; then past whens, placed by the `when`, oldest first; then
  `created_at`. Rendered only when non-empty. The CLI `agenda` still folds
  these into Today.
- **Placement** of every other row: an item appears exactly once, on its
  *placement day*, the earlier of its `when` day and its `deadline` day.
- **Tone** of a row is the most urgent of: overdue (deadline day < today),
  today-warning (deadline day = today), neutral. A past `when` is neutral.
- **Within a day**, order by the raw string of the field that placed the row,
  then `created_at`.
- **Badges**: the placing date is carried by the day header and not repeated,
  except in Overdue, where it shows its actual date (existing `pastAsDate`
  rule). The other field, when present, shows as its own badge so a row
  reads "Sat 13 · due 31 Oct". Timed rows show the time as a leading label.
- Today is always present (after Overdue, when that exists), empty if
  nothing is due, so the surface anchors on the current day.
- Stays the flat virtualised list it is today, not a `Dnd` listbox.
  Drag-to-reschedule is deferred (see below).

## Task surface and rows

- The task dialog and side panel gain a **When** control beside Deadline,
  same badge-with-popover pattern as `DeadlineField`: Set date…, Today,
  Tomorrow, Remove. Set date… opens the shared calendar modal, which gains an
  optional time field under the grid (blank ≡ all-day). Changing the date
  keeps the time; Remove clears both.
- **Time field** is Kobalte's `TimeField` (`@kobalte/core/time-field`,
  already in the installed 0.13.x), `granularity="minute"`, no seconds. It
  is unstyled and segmented (hour, minute, and a day-period segment in the
  12-hour cycle), keyboard-driven with arrow spin and typed digits, and
  reads the locale from the Kobalte `I18nProvider` that `AppI18nProvider`
  already mounts.
- **Hour cycle** follows the time-format preference, not the locale alone.
  `format.tsx` gains `hourCycle(locale): 12 | 24`: `"12h"` → 12, `"24h"` →
  24, `"auto"` → whatever `Intl.DateTimeFormat(locale, { hour: "numeric" })`
  resolves to (`h11` / `h12` → 12, `h23` / `h24` → 24). The result is passed
  as the field's `hourCycle` prop every time, never left to the component's
  own locale default, so the field and every formatted time in the app agree
  by construction. Changing the preference in Settings re-renders the field
  in the new cycle; the stored value is unaffected (it is always 24-hour
  `HH:MM`).
- **Value bridge.** The field's `value` is `{ hour?, minute? }`, plain
  numbers, no date library. The 16-character register maps to
  `{ hour: HH, minute: MM }`; the 10-character register maps to `{}`.
  `onChange` fires on every segment edit, including partial states (an hour
  typed with the minute still blank, or one segment cleared with Backspace),
  so the dialog holds the field state locally and writes through only when
  it is **complete** (both hour and minute set → timed) or **empty** (both
  unset → all-day). A partial state writes nothing and keeps the previous
  register value; the calendar's date pick applies the last complete or
  empty time. Clearing both segments on a timed value is how the user drops
  back to all-day without removing the date.
- The calendar modal is shared with Deadline. The time field mounts only
  when the modal is opened for `when`; the deadline path is unchanged and
  keeps writing 10-character stamps.
- `WhenBadge` beside `DeadlineBadge` on list rows and board cards. When both
  are set, `when` renders first. Muted on done/binned items as deadline is.
- Row context menus gain the same quick actions for When.
- i18n: a `when` message group mirroring `deadline` (label, unset, today,
  tomorrow; time-of-day formatting defers to the existing
  preference).

## URLs

No new token in the first cut. `upcoming` stays the agenda. Two forms are
reserved so later phases add without renaming:

- `calendar`, for a month-grid lens.
- A day anchor with an underscore, `upcoming_2026-07-13` and
  `calendar_2026-07`.

Amend `spec/urls.md` with the reservation only.

## CLI

`spec/cli.md` Items gains:

- `monoplan when <item_id> <YYYY-MM-DD[THH:MM] | ->`: set or clear (`-`) the
  `when` register.
- `monoplan deadline <item_id> <YYYY-MM-DD | ->`: set or clear the deadline.
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
  honest. No RRULE.
- **Fixed instants.** The bracketed suffix above. Flipping the default means
  writers append the device zone; sorting then needs instant normalisation
  for mixed values, which is why it waits.
- **Month grid.** A workspace-level lens, sibling of Upcoming, on the
  `calendar` token: one month at a time on `@corvu/calendar` (already under
  the date picker), a few row titles per cell in placement order with a
  "+k" overflow, tone per the agenda rules (past cells empty, since the fold
  moved their rows to Today), click or Enter on a day opens the agenda
  anchored on it. Needs the agenda to accept an anchor day. Nothing in the
  data model changes for it.
- **Reschedule.** Drag a row between agenda day sections or grid cells. A
  row placed by `when` moves its `when` with the time part kept; a row placed
  by `deadline` only moves its deadline; a row with both moves `when`, the
  deadline being a commitment that moves only through the explicit control.
  Needs the agenda to become a keyed grouped listbox, which the flat
  virtualised list cannot host. That is the whole cost, and the reason it
  waits.
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
   `WhenField` with the Kobalte time field and `hourCycle`, `groupByDay`
   (`dayGroups.ts`, replacing `deadlineGroups.ts`), agenda tone rules and
   leading time label, When submenu on row context menus and the palette,
   duplicate / paste carry `when`, i18n. `urls.md` amended for the reserved
   tokens.

Month grid and reschedule are deferred, not phased; see "Deferred, with their
extension points".
