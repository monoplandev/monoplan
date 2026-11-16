# Events

**Status: built 2026-11-16 (core, wasm, CLI, FFI, web, iOS, macOS), decided
the same day.** An event is a dated item with the `event` flag set. It
happens on its date instead of being done by it, so it surfaces on the
calendar and in no list, and it goes by instead of going overdue.

This replaces the reserved `events` list (built 2026-10-02, removed
2026-11-16). "From the `events` list" at the end records what changed and
how old docs read.

Companion to `calendar-plan.md` (the `when` register, the agenda),
`data-model.md` (the register), `cli.md`.

## Decisions in one screen

| Question | Decision |
|---|---|
| What is an event? | **An item whose `event` register is `true` and which has a `when`.** Both, always: the flag without a date is inert. One predicate, `is_event` (`ItemView::event` carries the effective value to every client). |
| Where is the flag in the UI? | **A checkbox under the date**, shown once the item has one: it says what the date means. Off by default; the calendar's Add ticks it. |
| Where do events show? | **The calendar** (the Upcoming lens, nav label "Calendar"). No list view, board or list count includes one. |
| Does an event keep its list? | **Yes.** `location` is untouched, so un-flagging puts it back where it was, and an event in a shared workspace's list is that workspace's event. The list just does not show it while it is one. |
| Do events have a checkbox? | **Yes, an optional one.** The row is a task's row. Nothing asks for the tick: when its day passes it leaves the agenda untouched. Status reads Event / Done / Cancelled. |
| What happens when the day passes? | **Nothing.** No write, no state change, no nag. A task in the same position is overdue (`calendar-plan.md`). |
| An event with no `when`? | **Cannot be seen as one.** The flag needs a date to mean anything, so `is_event` requires both and there is no Unscheduled section. Something with no date yet is an ordinary item in Inbox or a list. |
| Why a separate register, not a marker inside `when`? | The `when` grammar stays 10 or 16 characters and a plain string sort, and independent edits merge: one device toggling Event while another changes the time keeps both. |

## Semantics

An event is an ordinary item. Every other register keeps its meaning.

| Event state | Where it shows |
|---|---|
| Open, `when` today or later | the calendar, on its day |
| Open, `when` before today | nowhere on the agenda: it went by. Reachable by search and by its URL |
| Done | the Done view; also the calendar on its day while `when` is today or later, muted and ticked |
| Cancelled | the Done view with a cross; also the calendar on its day while `when` is today or later |
| Binned | the Bin |

- **Lifecycle is untouched.** An event is whatever open state it was when
  flagged. The open ladder is not offered on an event: the four open
  states read as one status, "Event". The value underneath is preserved
  and takes effect again when the flag goes.
- **`deadline`, `duration`, `place`, Starred are untouched.** A deadline
  on an event is legitimate ("tickets by the 3rd") and places and tones
  the row by the agenda's rules. An event can be starred like any Open
  item.
- **Open events are not open work.** They are in no list's Open
  projection (`ItemView::is_listed` is `is_open && !is_event`), so no
  list row, board lane, list count or `open_index` includes one.
- **Deleting a list keeps its events.** `delete_list(ToBin)` bins open
  work only. An event of the deleted list lands in the Inbox bucket
  (never shown there) and stays on the calendar.

### The flag and the date

`event` and `when` are independent registers, so the pair can be torn by
concurrent edits (one device clears the date while another ticks Event).
Two rules keep that harmless:

- **`is_event` needs both.** A flag with no date is no event: the item is
  an ordinary row in its list. Nothing can be invisible.
- **Clearing the date clears the flag**, in the same commit
  (`set_item_when(None)`). So a date set later makes a task, not an event
  again. Only the concurrent case can leave a flag behind; the next date
  then makes an event, which the checkbox shows.

`set_item_event(true)` on an item with no `when` is refused (`Invalid`).

## Core

- Register `event` (bool) on the item map. Absent ≡ false; clearing
  deletes the key.
- `Doc::set_item_event(item_id, bool)`: one commit, no-op when unchanged.
- `ItemView::event` (effective), `is_event()`, `is_listed()`.
- **Projection.** The index tracks `event` beside `open` per item. An
  event keeps its order entry and is treated as a closed item is: out of
  `open_members` / `open_by_list`, back at its entry's position when it
  stops being one.
- **`AppEvent::ItemEventChanged { id, event, open_index }`.** Emitted when
  the effective value flips: a flag write, or a `when` coming or going
  beside a set flag (local, remote or undo). Consumers drop the item from
  its list's Open array and, when `open_index` is set, splice it back in
  there. `ItemAdded` carries `event`.
- **Export / import.** `event: true` on the item, skipped when false.
  Cross-doc moves carry it. `duplicate_item` copies it.
- **Fingerprint** hashes the effective value.

## Clients

- **Web.** The Event checkbox sits under the time row in the task
  dialog's date section (`WhenField`). The calendar's Add opens a capture
  with today prefilled and the box ticked; unticking leaves an Inbox task
  with that date. Status controls collapse to Event / Done / Cancelled on
  an event (`EVENT_STATES`, `statusValue`, `statusLabel`). Find and item
  links open an Open event over the calendar. "Show in …" on an event
  names the calendar.
- **iOS.** An Event toggle in the date editor, above Clear date.
- **macOS.** No date editing in the window yet, so no toggle; status
  reads "Event", and a tray row for an event opens the Calendar lens.
- **CLI.** `item set <id> --event` / `--task` (`--when` in the same call
  counts as the date). `item ls` leaves open events out; `agenda` prints
  them. Rows carry `event: true` in `--json` when set.

## From the `events` list (2026-10-02 to 2026-11-16)

Events used to be items located in a reserved list `events`, rendered as
"Calendar": moving an item in or out was the conversion, and an undated
event sat in an Unscheduled section of the agenda. It went because:

- The "Calendar" list as a move destination beside the Calendar lens was
  confusing.
- Its job was to keep events out of Inbox and the lists. Since Inbox
  became the absence of a list (`inbox-plan.md`), a flag does that
  without a place.
- An event could not belong to a list, so it could not belong to a
  shared workspace's list either.

Removed with it: `LIST_EVENTS` / `EVENTS_NAME`, the `events` literal as a
list id in every API, the Calendar entry in the move palette and list
pickers, the Unscheduled group (web, CLI, `agenda --json`), `nav.events`
and `upcoming.unscheduled`.

**Old docs need no migration.** An item still located in `events` reads
as flagged (`event_flag` in `core/src/doc.rs`), and having no ListMeta row
it projects into Inbox like any orphan. So a dated one is an event with no
list, and an undated one is an Inbox task. The register is written the
first time such an item is moved (`write_location`), and un-flagging one
drops the old location. An export from before the change imports the same
way: `list_id: "events"` becomes Inbox plus the flag. `order/events` is
dead history.

## Deferred

- **Recurrence.** Unsolved, as before: an event is never ticked and the
  clock never writes, so repeat-on-done does not fit.
- **Past days.** Past events are invisible on the agenda until a month
  grid or a backward agenda exists.
- **Events in their list.** A list could show its own events in a section
  of its view. Not built: today only the calendar shows them.
- **macOS toggle**, with date editing in the Mac window.
- **`AgendaGroup.day`** is still optional in the typed API; `None` was
  the Unscheduled group and is no longer produced.

## Testing

- Core unit tests (`core/src/doc.rs`, "the `event` register"): flag on and
  off with exact events and positions, the date requirement, clear
  together, the concurrent tear, remote translation, undo / redo, Focus,
  list delete, close and reopen, the legacy location, export / import.
- `dayGroups.test.ts`, `events.test.ts` (web store): placement, the
  badge, the Open array across flag, date and undo.
- CLI: `agenda` unit tests; `sync_smoke` syncs an event across two
  devices.
