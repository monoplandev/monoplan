// Reminders (spec/reminders.md): which notifications a device has
// pending, from the items that have a time ahead and the device's two
// settings. Pure data here; the app target hands the plan to the
// system's notification centre.

import Foundation

/// The device's reminder for an event with a time. The raw value is
/// what `UserDefaults` holds.
public enum EventReminder: String, CaseIterable, Sendable {
    case tenMinutes = "10"
    case off

    /// Minutes ahead of the start; nil for none.
    public var lead: Int? {
        switch self {
        case .tenMinutes: return 10
        case .off: return nil
        }
    }
}

/// The device's reminder for a scheduled task: an item with a time
/// that is not an event.
public enum TaskReminder: String, CaseIterable, Sendable {
    case atTime = "0"
    case tenMinutes = "10"
    case off

    /// Minutes ahead of the start; nil for none.
    public var lead: Int? {
        switch self {
        case .atTime: return 0
        case .tenMinutes: return 10
        case .off: return nil
        }
    }
}

/// The two settings, a device preference: never synced, never in a doc.
/// Both start off: a device reminds about nothing until it is asked to.
public struct ReminderSettings: Equatable, Sendable {
    /// `UserDefaults` keys; absent reads as the default.
    public static let eventKey = "reminderEvents"
    public static let taskKey = "reminderTasks"
    public static let defaultEvent = EventReminder.off
    public static let defaultTask = TaskReminder.off

    public var event: EventReminder
    public var task: TaskReminder

    public init(event: EventReminder = defaultEvent, task: TaskReminder = defaultTask) {
        self.event = event
        self.task = task
    }

    public static func load(from defaults: UserDefaults = .standard) -> ReminderSettings {
        ReminderSettings(
            event: defaults.string(forKey: eventKey).flatMap(EventReminder.init) ?? defaultEvent,
            task: defaults.string(forKey: taskKey).flatMap(TaskReminder.init) ?? defaultTask
        )
    }
}

/// An item a reminder may be for: a row of `Engine.reminders`.
public struct ReminderSource: Equatable, Sendable {
    public let docId: String?
    public let itemId: String
    public let title: String
    /// `YYYY-MM-DDTHH:MM`, floating.
    public let when: String
    public let event: Bool

    public init(docId: String?, itemId: String, title: String, when: String, event: Bool) {
        self.docId = docId
        self.itemId = itemId
        self.title = title
        self.when = when
        self.event = event
    }
}

/// One notification to have pending.
public struct PlannedReminder: Equatable, Sendable {
    public let docId: String?
    public let itemId: String
    public let title: String
    /// Minutes ahead of the item's start that it fires.
    public let lead: Int
    /// When it fires, as wall-clock fields with no time zone: a
    /// floating time fires at that hour wherever the device then is.
    public let fire: DateComponents

    /// One per item: an item has one time, so one reminder.
    public var identifier: String { "reminder/\(docId ?? "")/\(itemId)" }
}

public enum Reminders {
    /// The most the system keeps pending for one app is 64; the plan
    /// stops short of it. What is cut is the furthest off, and comes
    /// back in as the nearer ones fire.
    public static let limit = 60

    /// What to have pending at `now`: each source its setting gives a
    /// reminder to, whose firing time is still ahead, soonest first,
    /// up to `limit`.
    public static func plan(
        _ sources: [ReminderSource], settings: ReminderSettings, now: Date = Date(),
        calendar: Calendar = .current, limit: Int = Reminders.limit
    ) -> [PlannedReminder] {
        let fields: Set<Calendar.Component> = [.year, .month, .day, .hour, .minute]
        var planned: [(at: Date, reminder: PlannedReminder)] = []
        for source in sources {
            guard let lead = source.event ? settings.event.lead : settings.task.lead,
                  source.when.count >= 16,
                  let start = Agenda.start(of: source.when, calendar: calendar),
                  let at = calendar.date(byAdding: .minute, value: -lead, to: start),
                  at > now
            else { continue }
            planned.append((
                at,
                PlannedReminder(
                    docId: source.docId, itemId: source.itemId, title: source.title, lead: lead,
                    fire: calendar.dateComponents(fields, from: at)
                )
            ))
        }
        planned.sort { ($0.at, $0.reminder.identifier) < ($1.at, $1.reminder.identifier) }
        return planned.prefix(limit).map(\.reminder)
    }
}
