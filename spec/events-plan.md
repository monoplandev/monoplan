# Events: plan

**Status: built 2026-10-02 (Phase 0 core / CLI and Phase 1 web). Decided
the same day.** Adds a second reserved
list, `events`, whose items surface only on the calendar. An event is
something that happens on a day. It does not need ticking off: it goes by.
It can still be ticked or cancelled by anyone who wants the record, and
its row keeps the checkbox for that.

Companion to `calendar-plan.md` (the `when` register, the agenda),
`data-model.md` (reserved lists, `location`), `urls.md`, `cli.md`, which
were amended in place as each phase landed. This file is the design
record.

## Decisions in one screen

| Question | Decision |
|---|---|
| What is an event? | **An item located in the reserved list `events`.** Nothing else: no item field, no kind flag. The test is `location.list_id == "events"`, behind one predicate (`is_event`). |
| Is this the checkless item kind rejected on 2026-09-16? | **No.** That was a second item kind. This is a place. Converting a task to an event and back is an ordinary move, and every item mutation works on an event unchanged. The other objection ("still needs a close action because the clock never writes") fell away on 2026-09-30, when a past `when` stopped needing a decision. See "Why a list and not a kind". |
| Where do events live in the UI? | **The calendar is their home** (the Upcoming lens, nav label "Calendar"). There is no Events nav entry, no list view and no board. The calendar is to `events` what the board is to a user list: its view. |
| Do events have a checkbox? | **Yes, an optional one** (revised 2026-10-02; the first cut replaced it with a dot marker). The row is a task's row. What makes it an event is that nothing asks for the tick: no list holds it as open work, no count includes it, and when its day passes it leaves the agenda untouched. Its status reads Event / Done / Cancelled (see "Status on an event"). |
| What happens when the day passes? | **Nothing.** The event stays Open, with its `when` in the past, and leaves the agenda like any past `when`. No write, no state change, no nag. |
| What about an event with no `when`? | **An Unscheduled section on the calendar**, rendered only when non-empty. The core never requires a `when` on an event (see "Why no invariant"). |
| One list or many calendars? | **One reserved list.** Several calendars (Work, Personal) would be a `kind` register on `ListMeta`; `is_event` is the only call site that would change. Not planned. |
| Schema | **Additive within v4.** A second reserved literal; no new container shape, no version bump, no import step. |

## Why a list and not a kind

The 2026-09-16 rejection in `calendar-plan.md` listed what a checkless
event kind would touch: board lanes, Focus, Overdue, repeat-on-done, the
CLI, plus a close action. Against that list:

- **Board.** `events` has no board and no list view, so lanes never see an
  event. Nothing in `board.md` changes for user lists.
- **Focus.** Unchanged. An event can be pinned like any Open item.
- **Overdue.** Unchanged. Overdue is overdue deadlines only, and a
  deadline on an event behaves as it does anywhere.
- **Close action.** Not needed. Since 2026-09-30 a past `when` is simply
  in the past.
- **CLI.** `events` is a list id; the existing verbs take it.

What remains is the problem the 2026-09-16 decision left open. An event
had to live in Inbox or a user list, where it carried a checkbox, counted
as open work, and stayed in the list for ever once its day had passed
unless someone ticked it. A reserved home that no list view projects
removes all three.

## Semantics

An event is an ordinary item. Every register keeps its meaning.

| Event state | Where it shows |
|---|---|
| Open, `when` today or later | the calendar, on its day |
| Open, `when` before today | nowhere on the agenda: it went by. Reachable by search and by its URL; shown on its day once a month grid or backward agenda exists |
| Open, no `when` | the calendar's Unscheduled section |
| Done | the Done view; also the calendar on its day while `when` is today or later, muted and ticked (the existing "`when` survives Done" rule) |
| Cancelled | the Done view with a cross; off the calendar (the existing rule) |
| Binned | the Bin |

- **Lifecycle is untouched.** An event is Backlog from creation and usually
  stays there. Done and Cancelled mean what they mean on a task. The open
  ladder (Backlog, Todo, In Progress, Review) is not offered on event
  surfaces: the four read as one status, "Event". A value written some
  other way (CLI, a move from a list) is preserved and ignored, and takes
  effect again if the item moves back to a list.
- **`deadline` and `duration` are untouched.** A deadline on an event is
  legitimate ("tickets by the 3rd") and places and tones the row by the
  existing agenda rules.
- **Open events are not open work.** No count includes them except the
  calendar's own badge. Any reflection or analytics surface that counts or
  ages Open items filters out `is_event`.
- **Moving is converting.** Task to event: move to `events`. Event to
  task: move to Inbox or any list. The item keeps its id, dates, notes,
  lifecycle and Focus ref; only `location` changes.

### Why no invariant

"An event must have a `when`" cannot be enforced. `location` and `when`
are independent registers: one device can clear the `when` while another
moves the item into `events`, and both writes survive the merge. A core
check would only reject one of two individually valid edits. So the core
accepts an event with no `when`, and the view gives it a place
(Unscheduled) so it can never be invisible.

For the same reason, clearing `when` on an event does **not** move it to
Inbox. The Unscheduled section has to exist for the concurrent case, and
once it exists a coupled move adds a second composed mutation and a
surprising change of home for no gain. "No date yet" is also a real state
for an event (a dinner with the date to be confirmed).

## Core

- Constants `LIST_EVENTS = "events"` and `EVENTS_NAME = "Events"` beside
  `LIST_INBOX` / `INBOX_NAME`. Like `inbox`, it is addressable by items, is
  not a `ListMeta` row, and orders through `order/events`. Its label is
  client-defined and localized.
- `assert_list_exists` accepts it, so `add_item*` and `move_item` take it
  as a target with no other change. The projection index already
  discovers any list id that items locate to.
- `rename_list`, `set_list_icon`, `set_list_archived`, `move_list` and
  `delete_list` refuse it, as they refuse `inbox`. `set_default_view`
  refuses it too, with `Invalid`: there is no list or board view to save.
  `delete_list` still relocates a deleted list's items to `inbox`, never
  to `events`.
- `ItemView::is_event()`. No new mutation, no new event variant, no new
  register.
- **Order.** `order/events` is maintained by add and move as for any list
  but no view reads it: the calendar orders by date. The Unscheduled
  section orders by `created_at`.
- **Export / import.** An event exports as an ordinary item with
  `list_id: "events"`. The built-in `events` row in `lists` is emitted
  (after `inbox`) only when at least one item locates to it, so existing
  dumps stay byte-identical. Import maps the literal onto the local
  `events` whether or not the row is present, as it does `inbox`.
- **wasm.** No API change (the bundle must be rebuilt to pick up the
  accepted id). The web client names the reserved id as a literal, as it
  does `inbox`: `LIST_EVENTS` beside `isEvent` in `sync/store.ts`.

### Compatibility

Additive within schema v4. The id grammar in `data-model.md` ("uuid-v7 hex
or the literal `inbox`") gains a second literal; neither contains `:`. An
events-unaware client degrades safely: it shows events on its agenda as
ordinary checkbox rows (the agenda runs over all items), finds them in
search and in Done, never lists them under any list, and refuses a move
into `events` with `ListNotFound`. Nothing is hidden that was visible and
nothing is lost.

## Calendar (web)

The agenda in `calendar-plan.md` is unchanged for tasks. Additions:

- **Event rows.** The same row as a task row, checkbox included, with the
  owning-list label omitted. That absence is the event's mark on the
  agenda: every task row carries its list, an event has none worth
  showing. Ticking works as on any row (done, muted, slot kept).
  Placement, tone, within-day order and badges follow the existing rules
  with no special case.
- **Unscheduled.** A section holding every Open event with no `when` and
  no `deadline`, oldest first by `created_at`. Rendered only when
  non-empty, after Overdue and before Today. (An event with a deadline and
  no `when` is placed by its deadline, as any item is.)
- **Nav badge.** `attentionBadge` counts unscheduled events at neutral
  tone, alongside what is owed or on today. Otherwise they would be
  discoverable only by opening the calendar.
- **Capture.** The calendar's Add button (desktop header and mobile
  pill) captures into `events` with `when` prefilled to today, all-day.
  The dialog's list picker switches the new item to Inbox or a list when
  it is a task. A capture left untouched lands on Today, never out of
  sight. Re-filing any capture to Events in the picker prefills today the
  same way when no date is set, and drops a board lane back to Backlog.
- **Move.** "Events" appears as a destination in the move palette and the
  task dialog's list picker, after Inbox, with the calendar glyph. Moving
  a single Open item with no `when` into Events opens it with its date
  popover showing, as Set date… does. A multi-selection just moves;
  undated ones land in Unscheduled.
- **Status on an event** (revised 2026-10-02; the first cut hid the
  status controls and put closing actions in the item menu). The status
  controls stay where a task has them, relabelled: an event's four open
  states collapse into one choice, **Event**, beside **Done** and
  **Cancelled** (`EVENT_STATES`, `statusValue`, `statusLabel`). This is
  display only; nothing is stored differently. Picking Done or Cancelled
  is the ordinary lifecycle write. Picking Event on a closed event reopens
  it (Backlog, the core's un-done rule); on an Open one it changes
  nothing, so an open state brought in from a list is kept. The task
  surface's lifecycle badge and the Status submenu of a row context menu
  (Focus, Done) both use the trimmed set; agenda rows have no context
  menu, so the badge is how an event is closed from the calendar. `x` and
  `⇧x` work on a selected event row as on any row. When, Deadline, Focus,
  Move, Copy link and Bin are unchanged.
- **Task surface.** The dialog and side panel are a task's, with the
  status badge reading "Event" while the event is Open. All other fields
  are unchanged.
- **Elsewhere.** Search indexes events like any item; the owning-list
  column reads "Events".
  Picking an Open event in Find goes to the calendar and opens the item
  (there is no row to select, and a past event has no day). The Done view
  and the Bin list closed and binned events with that same label. In Focus
  an event is an ordinary row, and its "Show in Events" goes to the
  calendar with the item open. No list count, list column or board
  ever includes one.
- **i18n.** `nav.events`, `upcoming.unscheduled`, `upcoming.event` (the
  open status label), in both languages.

## URLs

No new token. `#list_events` is accepted as an alias and canonicalised to
`#upcoming`, as `#list_inbox` is to `#inbox`. Opening `#item_<id>` for an
Open event navigates to the calendar and opens the item, whether or not
its row is on the agenda (a past event opens in the dialog over the
calendar). Closed and binned events resolve to Done and Bin as any item
does.

## CLI

- `events` is a second literal list id, accepted wherever `<list>` is:
  `add --list events`, `mv <id> events`, `ls --list events`.
- `ls --list events` prints events in resolved order with the usual `@` /
  `!` tags and the same state box as any row.
- `agenda` prints event rows like any other. An Unscheduled group
  prints after Today when non-empty. `--json` rows already carry
  `list_id`; the Unscheduled group is `{ day: null, today: false, rows }`
  in second position, and its rows omit `placed_by`.
- `lists ls` is unchanged: reserved lists are not `ListMeta` rows.
- An event added without a `when` is unscheduled until `monoplan when`
  sets one.

## Deferred, with their extension points

- **Recurrence.** The form sketched in `calendar-plan.md`, repeat-on-done,
  does not fit: an event is never ticked, and the clock never writes, so
  nothing spawns the successor. Birthdays and weekly meetings therefore
  need either a rule expanded at read time (an RRULE-shaped register,
  which `calendar-plan.md` calls a calendar app's job) or manual
  duplication. Unsolved here; this is the largest gap in treating Monoplan
  as a calendar.
- **Past days.** Past events are invisible on the agenda. The month grid
  or a backward-scrolling agenda (`calendar-plan.md` "Deferred") is where
  they are seen. Events raise the priority of one of the two.
- **Several calendars.** A `kind` register on `ListMeta` marking a user
  list as an event list; `is_event` becomes a lookup. Sharing
  (`sharing-plan.md`) would then share a calendar as it shares a list.
- **Multi-day all-day events.** Expressible since 2026-10-07: `duration`
  beside an all-day `when` counts whole days (`calendar-plan.md` "Field:
  `duration`"), set from the task dialog's end-date field. The agenda
  still places the item on its start day only; showing it on each day it
  covers is open.
- **Export.** An event maps to a `VEVENT` with no caveat; the "Due:"
  prefix workaround in the iCalendar mapping applies to tasks only.

## Testing

- Core unit tests: add and move into `events`; the refusals (rename,
  icon, archive, move, delete, default view); `is_event`; an event with no
  `when` is accepted; a deleted list's items still land in `inbox`; export
  omits the built-in row when empty and round-trips when not.
- `dayGroups.test.ts`: event rows place by the existing rules;
  Unscheduled membership, order and position; an event with a deadline and
  no `when` is placed by the deadline; the badge counts unscheduled
  events; cancelled and past events place nothing.
- `events.test.ts` (web store): add and move into `events`, the open
  state kept underneath while the picker shows one choice, close and
  reopen, the built-in list label in Find.
- CLI system test: an event added on one device appears on the other's
  `agenda` after sync; `mv` out of `events` makes it an ordinary list
  item with its dates intact.
- Web: typecheck plus source reading (no browser automation here).

## Phases

0. **Core, wasm, CLI.** Built. The reserved id, refusals, `is_event`,
   export / import and the agenda's Unscheduled group.
   `data-model.md` and `cli.md` amended. Unit and system tests.
1. **Web.** Built. Event rows, Unscheduled, badge, capture, move
   destination, trimmed status controls and task surface, Find, URL
   alias, i18n.
   `calendar-plan.md` and `urls.md` amended. Verified by typecheck, unit
   tests and source reading only (no browser automation here).
