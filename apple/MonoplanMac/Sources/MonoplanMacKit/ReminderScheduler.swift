// Reminders (spec/reminders.md): local notifications for the items
// that have a time ahead, scheduled from what this device last synced.
// The plan (which items, when each fires) is `Reminders.plan`, over
// `Engine.reminders` and the device's two settings; this is the half
// that talks to the system, shared by the iOS and the Mac app. Every
// change replaces the whole pending set: at most `Reminders.limit`
// requests, and nothing else in either app schedules a notification.
//
// Both settings start off, so nothing is scheduled and nothing is
// asked until one is turned on in Settings; permission is asked for
// then. A click on a notification is handed to the app (`onOpen`).
//
// The one file of the Kit that is not Foundation alone: it uses
// UserNotifications, which every target that links the Kit has.

import Foundation
import UserNotifications

/// The item a clicked reminder was for.
public struct ReminderTap: Equatable, Sendable {
    public let docId: String?
    public let itemId: String
    public let listId: String
    public let title: String
}

public final class ReminderScheduler: NSObject, UNUserNotificationCenterDelegate {
    /// One for the process: the centre's delegate has to be in place
    /// by the end of launch for a click that started the app to
    /// arrive, so the app touches this as it launches.
    public static let shared = ReminderScheduler()

    /// A notification was clicked. Set by the app; a click from before
    /// it was set is handed over as it is. Main queue.
    public var onOpen: ((ReminderTap) -> Void)? {
        didSet {
            guard let onOpen, let tapped else { return }
            self.tapped = nil
            onOpen(tapped)
        }
    }

    /// The body of a reminder that fires ahead of its item, given the
    /// minutes ("In 10 minutes"): the app's, since the words are in
    /// its catalog and not the Kit's.
    public var leadText: (Int) -> String = { "\($0) min" }

    private let center = UNUserNotificationCenter.current()
    private var tapped: ReminderTap?
    private var sources: [ReminderSource] = []
    /// What the centre was last given; nil when that is not known (at
    /// launch, and while permission is denied).
    private var pending: [PlannedReminder]?
    /// A permission prompt is up.
    private var asking = false

    private override init() {
        super.init()
        center.delegate = self
    }

    /// The items a reminder may be for, as just read. Main queue.
    public func update(_ sources: [ReminderSource]) {
        self.sources = sources
        reschedule()
    }

    /// Bring the pending set in line with the items, the settings and
    /// the clock. Main queue. `settingsChanged`: the person just
    /// picked a setting, the moment to ask for permission when one is
    /// on, whether or not anything is timed yet.
    public func reschedule(settingsChanged: Bool = false) {
        let settings = ReminderSettings.load()
        let plan = Reminders.plan(sources, settings: settings)
        let wanted = settingsChanged && (settings.event != .off || settings.task != .off)
        if plan == pending && !wanted { return }
        center.getNotificationSettings { [weak self] system in
            let status = system.authorizationStatus
            DispatchQueue.main.async { self?.apply(plan, status: status, ask: wanted || !plan.isEmpty) }
        }
    }

    /// Whether notifications are turned off for the app in the
    /// system's settings, for the app's own Settings to say so.
    /// Answers on the main queue.
    public func readDenied(_ done: @escaping (Bool) -> Void) {
        center.getNotificationSettings { system in
            let denied = system.authorizationStatus == .denied
            DispatchQueue.main.async { done(denied) }
        }
    }

    private func apply(_ plan: [PlannedReminder], status: UNAuthorizationStatus, ask: Bool) {
        switch status {
        case .notDetermined:
            guard ask, !asking else { return }
            asking = true
            center.requestAuthorization(options: [.alert, .sound]) { [weak self] granted, _ in
                DispatchQueue.main.async {
                    self?.asking = false
                    if granted { self?.reschedule() }
                }
            }
        case .denied:
            // Nothing can be delivered; what was pending before the
            // permission went stays the system's to drop.
            pending = nil
        default:
            pending = plan
            center.removeAllPendingNotificationRequests()
            for reminder in plan { center.add(request(reminder)) }
        }
    }

    private static let docKey = "doc"
    private static let itemKey = "item"
    private static let listKey = "list"

    private func request(_ reminder: PlannedReminder) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = reminder.title
        if reminder.lead > 0 { content.body = leadText(reminder.lead) }
        content.sound = .default
        var info: [String: String] = [Self.itemKey: reminder.itemId, Self.listKey: reminder.listId]
        if let docId = reminder.docId { info[Self.docKey] = docId }
        content.userInfo = info
        // No time zone in the fields: a floating time, read in the
        // zone the device is in when it comes round.
        let trigger = UNCalendarNotificationTrigger(dateMatching: reminder.fire, repeats: false)
        return UNNotificationRequest(identifier: reminder.identifier, content: content, trigger: trigger)
    }

    // MARK: UNUserNotificationCenterDelegate

    /// A reminder that comes round while the app is in front is still
    /// shown: what is on screen is rarely the item.
    public func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound])
    }

    public func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let content = response.notification.request.content
        if response.actionIdentifier == UNNotificationDefaultActionIdentifier,
           let itemId = content.userInfo[Self.itemKey] as? String,
           let listId = content.userInfo[Self.listKey] as? String {
            let tap = ReminderTap(
                docId: content.userInfo[Self.docKey] as? String, itemId: itemId, listId: listId,
                title: content.title)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if let onOpen { onOpen(tap) } else { tapped = tap }
            }
        }
        completionHandler()
    }
}
