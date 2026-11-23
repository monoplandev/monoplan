// Reminders (spec/reminders.md): local notifications for the items
// that have a time ahead, scheduled from what this iPhone last synced.
// The plan (which items, when each fires) is `Reminders.plan` in the
// Kit, over `Engine.reminders` and this device's two settings; this is
// the half that talks to the system. Every change replaces the whole
// pending set: at most `Reminders.limit` requests, and nothing else in
// the app schedules a notification.
//
// Both settings start off, so nothing is scheduled and nothing is
// asked until one is turned on in Settings; permission is asked for
// then. A tap on a notification opens the item's
// page.

import Foundation
import MonoplanCore
import MonoplanMacKit
import UserNotifications

final class ReminderScheduler: NSObject, UNUserNotificationCenterDelegate {
    /// One for the process: the centre's delegate has to be in place
    /// by the end of launch for a tap that started the app to arrive
    /// (`MonoplanApp.init` touches this).
    static let shared = ReminderScheduler()

    /// A notification was tapped. Set by the model; a tap from before
    /// it was set is handed over as it is.
    var onOpen: ((ItemRef) -> Void)? {
        didSet {
            guard let onOpen, let tapped else { return }
            self.tapped = nil
            onOpen(tapped)
        }
    }

    private let center = UNUserNotificationCenter.current()
    private var tapped: ItemRef?
    private var sources: [ReminderSource] = []
    /// What the centre was last given; nil when that is not known (at
    /// launch, and after the settings changed under us).
    private var pending: [PlannedReminder]?
    /// A permission prompt is up.
    private var asking = false

    private override init() {
        super.init()
        center.delegate = self
    }

    /// The items a reminder may be for, as just read.
    func update(_ rows: [Reminder]) {
        sources = rows.map {
            ReminderSource(docId: $0.docId, itemId: $0.id, title: $0.text, when: $0.when, event: $0.event)
        }
        reschedule()
    }

    /// Signed out: nothing is pending for an account that is gone.
    func clear() {
        sources = []
        pending = []
        center.removeAllPendingNotificationRequests()
    }

    /// Bring the pending set in line with the items, the settings and
    /// the clock. Main thread. `settingsChanged`: the person just
    /// picked a setting, the moment to ask for permission when one is
    /// on, whether or not anything is timed yet.
    func reschedule(settingsChanged: Bool = false) {
        let settings = ReminderSettings.load()
        let plan = Reminders.plan(sources, settings: settings)
        let wanted = settingsChanged && (settings.event != .off || settings.task != .off)
        if plan == pending && !wanted { return }
        center.getNotificationSettings { [weak self] system in
            let status = system.authorizationStatus
            DispatchQueue.main.async { self?.apply(plan, status: status, ask: wanted || !plan.isEmpty) }
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
            for reminder in plan { center.add(Self.request(reminder)) }
        }
    }

    private static let docKey = "doc"
    private static let itemKey = "item"

    private static func request(_ reminder: PlannedReminder) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = reminder.title
        if reminder.lead > 0 {
            content.body = String(localized: "In \(reminder.lead) minutes")
        }
        content.sound = .default
        var info: [String: String] = [itemKey: reminder.itemId]
        if let docId = reminder.docId { info[docKey] = docId }
        content.userInfo = info
        // No time zone in the fields: a floating time, read in the
        // zone the device is in when it comes round.
        let trigger = UNCalendarNotificationTrigger(dateMatching: reminder.fire, repeats: false)
        return UNNotificationRequest(identifier: reminder.identifier, content: content, trigger: trigger)
    }

    // MARK: UNUserNotificationCenterDelegate

    /// A reminder that comes round while the app is open is still
    /// shown: the page on screen is rarely the item's.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let content = response.notification.request.content
        if response.actionIdentifier == UNNotificationDefaultActionIdentifier,
           let itemId = content.userInfo[Self.itemKey] as? String {
            let ref = ItemRef(docId: content.userInfo[Self.docKey] as? String, itemId: itemId, title: content.title)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if let onOpen { onOpen(ref) } else { tapped = ref }
            }
        }
        completionHandler()
    }
}
