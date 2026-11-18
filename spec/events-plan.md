# Events

**Status: built 2026-11-16 (core, wasm, CLI, FFI, web, iOS, macOS), decided
the same day.** An event is a dated item with the `event` flag set. The
flag says what the date means: the item happens then, instead of being
done by then. So when the day passes an event goes by, where a task goes
overdue. That is all the flag changes, with one consequence for order
(2026-11-17): within a day an all-day event leads and an untimed task
follows the timed rows (`calendar-plan.md` "Within a day"), since a day
something happens on frames the day and a task for the day fits around it.

Where a dated item shows is a separate rule and does not look at the
flag: Inbox holds only what has no list and no date (`inbox-plan.md`
"Membership"), so a dated item with no list, event or task, shows on the
calendar and not in Inbox, and is labelled "Calendar" wherever its list
is named. A dated item in a user list is a row there.

This replaces the reserved `events` list (built 2026-10-02, removed
2026-11-16). "From the `events` list" at the end records what changed.

Companion to `calendar-plan.md` (the `when` register, the agenda),
`inbox-plan.md` (the date rule), `data-model.md` (the register), `cli.md`.

## Decisions in one screen

| Question | Decision |
|---|---|
| What is an event? | **An item whose `event` register is `true` and which has a `when`.** Both, always: the flag without a date is inert. One predicate, `is_event` (`ItemView::event` carries the effective value to every client). |
| What does the flag change? | **What a passed `when` means** (it went by, not overdue) **and how the status reads** (Event / Done / Cancelled). Nothing about where the item shows. |
| Where is the flag in the UI? | **A checkbox under the date**, shown once the item has one. Off by default; the calendar's Add ticks it. |
| Where do events show? | **Where any dated item does:** on the calendar, and as a row in their list when they have one. One with no list is not in Inbox, like every dated item. |
| Do events have a checkbox? | **Not on the calendar while open** (2026-11-17): nothing asks for the tick, so the row leads with a dot in the box's place (the calendar glyph on the web since 2026-11-18). Marked done or cancelled (item surface, row menu), the row shows the box, ticked or crossed, and unticking it there reopens the event. A deadline's own row keeps its box (it is owed), and so does the event's row in a user list, where the tick is how a passed event is cleared. |
| What happens when the day passes? | **Nothing.** No write, no state change, no nag. It leaves the agenda. In a user list its row stays until it is ticked, moved or binned (see "Deferred"). |
| An event with no `when`? | **Cannot be seen as one.** `is_event` requires both, so there is no Unscheduled section. Something with no date yet is an ordinary item. |
| Why a separate register, not a marker inside `when`? | The `when` grammar stays 10 or 16 characters and a plain string sort, and independent edits merge: one device toggling Event while another changes the time keeps both. |

## Semantics

An event is an ordinary item. Every other register keeps its meaning.

- **Lifecycle is untouched.** The open ladder is not offered on an event:
  the four open states read as one status, "Event". The value underneath
  is preserved and takes effect again when the flag goes.
- **`deadline`, `duration`, `place`, Starred are untouched.** A deadline
  on an event is legitimate ("tickets by the 3rd") and places and tones
  the row by the agenda's rules.
- **Overdue.** An Open task's past `when` is overdue; an event's is not
  judged (`calendar-plan.md` "Semantics"). `is_event` is what tells the
  two apart.
- **Deleting a list** treats its events as it treats every open item:
  binned.

### The flag and the date

`event` and `when` are independent registers, so the pair can be torn by
concurrent edits (one device clears the date while another ticks Event).
Two rules keep that harmless:

- **`is_event` needs both.** A flag with no date is no event.
- **Clearing the date clears the flag**, in the same commit
  (`set_item_when(None)`). So a date set later makes a task, not an event
  again. Only the concurrent case can leave a flag behind; the next date
  then makes an event, which the checkbox shows.

`set_item_event(true)` on an item with no `when` is refused (`Invalid`).

## Core

- Register `event` (bool) on the item map. Absent ≡ false; clearing
  deletes the key.
- `Doc::set_item_event(item_id, bool)`: one commit, no-op when unchanged.
- `ItemView::event` (effective), `is_event()`.
- `AppEvent::ItemEventChanged { id, event }` when the effective value
  flips: a flag write, or a `when` coming or going beside a set flag
  (local, remote or undo). `ItemAdded` carries `event`. No projection
  depends on the flag.
- **Export / import.** `event: true` on the item, skipped when false.
  Cross-doc moves carry it. `duplicate_item` copies it.
- **Fingerprint** hashes the effective value.

## Clients

- **Web.** The Event checkbox sits under the time row in the task
  dialog's date section (`WhenField`). The calendar's Add opens a capture
  with no list, today prefilled and the box ticked. Status controls
  collapse to Event / Done / Cancelled on an event (`EVENT_STATES`,
  `statusValue`, `statusLabel`).
- **iOS.** An Event toggle in the date editor, above Clear date.
- **macOS.** No date editing in the window yet, so no toggle; status
  reads "Event".
- **CLI.** `item set <id> --event` / `--task` (`--when` in the same call
  counts as the date). Rows carry `event: true` in `--json` when set.

## From the `events` list (2026-10-02 to 2026-11-16)

Events used to be items located in a reserved list `events`, rendered as
"Calendar": moving an item in or out was the conversion, and an undated
event sat in an Unscheduled section of the agenda. It went because:

- The "Calendar" list as a move destination beside the Calendar lens was
  confusing.
- Its job was to keep events out of Inbox and the lists. Inbox now leaves
  dated items out by rule, which covers the first; the second was given
  up (an event can now be filed in a list and is a row there).
- An event could not belong to a list, so it could not belong to a
  shared workspace's list either.

Removed with it: `LIST_EVENTS` / `EVENTS_NAME`, the `events` literal as a
list id in every API, the Calendar entry in the move palette and list
pickers, the Unscheduled group (web, CLI, `agenda --json`), `nav.events`
and `upcoming.unscheduled`.

**There is no read path for old docs.** An item still located in `events`
is an orphan: it projects into Inbox like any item whose list is gone
(hidden there while it has a date) and is not an event. Accounts from
before the change are re-created from an export taken with a build that
still had the read path (2026-11-16, commit "No more 'calendar' psuedo
list"), which writes those items as `event: true`.

A first cut the same day (that commit) hid events from every list's Open
projection instead of hiding dated items from Inbox. It was replaced: the
rule needed the core to take rows out of ordered lists, and "Inbox is what
has no list and no date" is the simpler thing to explain.

## Deferred

- **Recurrence.** Unsolved, as before: an event is never ticked and the
  clock never writes, so repeat-on-done does not fit.
- **Past days.** Built everywhere: the CLI (`agenda --past N`) and the
  iOS Calendar page (a year back, above Today) show past events since
  2026-11-16 (`spec/cli.md` §Items), the Mac window's Calendar and the
  web agenda since 2026-11-17. The apps open on Today's header
  (Overdue's when something is overdue, on the Mac and the web). Each
  has a "Today" button that comes back to it.
- **A passed event in a user list** stays an open row there. Muting it,
  or folding passed events away, is a view decision not made yet.
- **macOS toggle**, with date editing in the Mac window.
- **`AgendaGroup.day`** is still optional in the typed API; `None` was
  the Unscheduled group and is no longer produced.

## Testing

- Core unit tests (`core/src/doc.rs`, "the `event` register"): flag on and
  off with exact events, the date requirement, clear together, the
  concurrent tear, close and reopen, export / import.
- `dayGroups.test.ts`, `events.test.ts` (web): placement, the badge, the
  status collapse.
- CLI: `agenda` unit tests; `sync_smoke` syncs an event across two
  devices.
