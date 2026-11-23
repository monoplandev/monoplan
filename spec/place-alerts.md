# Place alerts

**Status: built on iOS 2026-11-23 (the Kit's plan with tests, the
shared scheduler, the app's location half, a Settings section), decided
the same day. Compiled, not yet run on a device.** A place alert is a
local notification shown on arriving at a place that has open tasks.
The system watches the regions and shows the alert itself, with the app
not running. Nothing about alerts is in a doc, and the phone's location
never leaves it.

Companion to `place-plan.md` (the `place` register; this is its
deferred "Notifications") and `reminders.md` (the scheduler and the
pending set this shares).

## Decisions in one screen

| Question | Decision |
|---|---|
| What gets an alert? | **An Open item that is not an event and whose place has coordinates.** A label-only place cannot be watched. An event's place is where it happens, not somewhere to be told about on arrival. |
| How is it fired? | **`UNLocationNotificationTrigger`**: the system watches a circular region and shows the notification on entry. The app is never woken and needs only when-in-use location, which the Map page already asks for. Region monitoring by the app (`CLMonitor`) was passed over: it needs Always permission for no gain here. |
| What is the setting? | **One switch, "Alert on arrival", off by default**, on the device (`UserDefaults`, `PlaceAlertSettings.key`), never synced. No per-item trigger: the `trigger` register `place-plan.md` sketched is not built. |
| Arrive or leave? | **Arrive only.** |
| Once, or every visit? | **Every visit**, until the tasks are done (a repeating trigger): a one-off would need the app opened to arm it again. |
| How wide is a region? | **150 m** around a place (`PlaceAlerts.radius`). Below about 100 m the system's own accuracy is the limit. No per-place radius. |
| Which workspaces? | **Every open one**, whoever the item is assigned to, as reminders. |

## The plan

`PlaceAlerts.plan` in `MonoplanMacKit` (`PlaceAlerts.swift`, Foundation
only, tested in `PlaceAlertsTests`): the tasks and where the phone last
was give the regions to watch.

1. **Spots.** Tasks at the same coordinates (to five decimals, about a
   metre: the Map page's rule) are one spot.
2. **Merge.** Spots a geofence could not tell apart share a region: a
   spot joins the nearest region whose centre is within one radius of
   it, while the region that covers them all stays within 300 m
   (`maxRadius`). The region's centre is the middle of its spots and
   its radius is 150 m plus the reach to its furthest spot. Spots are
   taken in a fixed order (by latitude, then longitude), so the same
   places always merge the same way. Regions are never widened to make
   them fit the limit.
3. **Trim.** The system watches at most 20 regions for one app. The
   nearest to where the phone last was are kept; with no fix, those
   whose places have the newest tasks.

## The notification

- **One task:** its text as the title, the place's label under it. A
  tap opens the task's page, as a reminder's does.
- **Several:** the places' labels as the title; under it the first
  three tasks by name and "and N more". A tap opens the Map page (there
  is no one item to open).
- The default sound; shown while the app is in front too.

## Shape

- **The read:** `Engine.placedItems(closed: false)`, the Map page's
  read; the app drops events and places without coordinates
  (`AppModel.refreshPlaceAlerts`). No core change.
- **The scheduler:** `ReminderScheduler.updatePlaces` beside its
  `update`. Place alerts and time reminders are one pending set of at
  most 60: the regions take their share first (20 at most) and the
  times fill the rest. The set is brought in line by difference, so a
  region that stays is left being watched and not registered again.
  Regions are planned on iOS only (`#if os(iOS)`).
- **Location:** `PlaceAlertLocation` in the iOS app. Idle until the
  setting is on; then when-in-use permission is asked for, and one fix
  is requested at launch and on each return to the foreground. The fix
  is held in memory only.
- **When it runs:** wherever the reminders refresh does (engine start,
  every doc change, return to the foreground), when the setting
  changes, and when the permission or the fix does.
- **Settings:** a Places section in `SettingsView` with the switch.
  While it is on and location is denied, location is approximate, or
  notifications are denied, the footer says which and a button opens
  the app's page in the system's Settings.

## Limits

- **Precise location is required.** With approximate location the
  system watches no region; the app hands the scheduler no tasks and
  Settings says why.
- **Coarse and not instant.** An alert comes within a minute or so of
  crossing into the region, sometimes several.
- **Already there, no alert.** A task added while at its place alerts
  on the next arrival, not then.
- **A device knows only what it last synced**, as reminders: a task
  done elsewhere still alerts until the app has been opened, and a
  count can be out of date the same way.
- **The 20 are re-picked only when the app is opened.** Someone with
  more than 20 regions who travels far gets no alerts there until then.
- **Not checked on a device:** that the system's cap on location
  triggers is the 20 of app-watched regions (Apple gives no number for
  triggers), and how promptly alerts arrive in practice.

## Not built

A per-item trigger or opt-out, leave alerts, a per-place radius, tasks
dated in the future held back until their day, the Map page narrowed to
the alert's spot on a tap, macOS (the trigger does not exist there),
web.
