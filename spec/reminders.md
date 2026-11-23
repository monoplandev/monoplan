# Reminders

**Status: iOS built 2026-11-23 (core read, FFI, the Kit's plan, the iOS
scheduler and Settings rows), decided the same day; not yet run on a
device. macOS, web and the CLI daemon not built.** A reminder is a local
notification for an item with a time, scheduled by each device from what
it last synced (`daemon-plan.md` "Mobile hosts"). There is no server
push and nothing about reminders is in a doc.

Companion to `calendar-plan.md` (the `when` register) and
`events-plan.md` (the `event` flag).

## Decisions in one screen

| Question | Decision |
|---|---|
| What gets a reminder? | **An Open item with a timed `when`** (`YYYY-MM-DDTHH:MM`). An all-day `when` and a deadline get none. |
| Which kinds are there? | **Two, by the `event` flag:** an event, and a scheduled task (any other timed item). |
| What are the settings? | **Events: 10 minutes before / Off. Scheduled tasks: At the time / 10 minutes before / Off.** **Both start Off** (changed the day it was built): a device reminds about nothing until asked to. |
| Where do the settings live? | **On the device** (`UserDefaults`), never synced and never in a doc. Each device has its own, and a new install starts with both Off. |
| Which workspaces? | **Every open one**, those hidden from the calendar included, whoever the item is assigned to. |
| Per-item override? | **None.** To stop one reminder, take the item's time off or turn its kind off. |
| Snooze, dismissal sync? | **None.** Done, binned or re-timed items lose or move their reminder the next time the device sees the change. |

## Shape

- **Core read:** `api::reminders::reminders` (`cli/src/api/reminders.rs`),
  `Engine.reminders`: every open workspace's Open items with a timed
  `when` at or after now, soonest first, as `{id, text, when, event,
  doc_id}`. It applies no setting.
- **Plan:** `Reminders.plan` in `MonoplanMacKit` (`Reminders.swift`,
  Foundation only, tested in `RemindersTests`): the rows and the
  device's `ReminderSettings` give the notifications to have pending,
  soonest first, each firing `lead` minutes ahead of its start. A firing
  time already gone by is left out. The system keeps at most 64 pending
  per app; the plan stops at 60 and the furthest are cut, to come back in
  as nearer ones fire.
- **Floating time:** the trigger is the wall-clock fields with no time
  zone, so a reminder fires at that hour in whatever zone the device is
  then in.

## iOS

`ReminderScheduler.swift`. Every change replaces the whole pending set;
nothing else in the app schedules a notification.

- **When it runs:** on engine start, on every doc change (with the other
  lenses' refresh), on return to the foreground, and when a setting
  changes.
- **Permission:** asked when a setting is turned on in Settings, never
  at launch (both start Off, so nothing is scheduled before that).
  While it is denied, Settings says so and offers the system's
  notification settings.
- **The notification:** the item's text as the title; "In 10 minutes" as
  the body when it fires ahead; the default sound. It is shown while the
  app is open too.
- **A tap** opens the item's page (`AppModel.tappedReminder`, consumed by
  `opensItems()`), after the session has loaded when the tap is what
  started the app.
- **Sign out and account deletion** clear the pending set.
- **Settings:** a Reminders section with the two pickers
  (`ReminderSettings.eventKey` / `taskKey`).

## Limits

- **A device knows only what it last synced.** iOS runs no engine in the
  background, so an item timed on another device reminds on the phone
  only once the app has been opened since.
- **No time-sensitive delivery.** A Focus that silences the app silences
  its reminders; the entitlement is not requested.
- **A delivered notification stays** in Notification Centre after its
  item is done elsewhere.

## Not built

macOS (the Kit's plan is shared; it needs a scheduler and Settings rows),
web (a tab can notify only while open), the CLI daemon (it would double
up with the Mac app on one machine), all-day and deadline reminders, a
daily digest, per-item overrides, synced settings, an assignee filter.
