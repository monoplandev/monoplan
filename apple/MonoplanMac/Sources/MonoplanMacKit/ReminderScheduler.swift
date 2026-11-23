// Reminders (spec/reminders.md): local notifications for the items
// that have a time ahead, scheduled from what this device last synced.
// The plan (which items, when each fires) is `Reminders.plan`, over
// `Engine.reminders` and the device's two settings; this is the half
// that talks to the system, shared by the iOS and the Mac app. Every
// change brings the pending set in line: at most `Reminders.limit`
// requests, and nothing else in either app schedules a notification.
//
// On iOS the same set holds the place alerts (spec/place-alerts.md):
// `PlaceAlerts.plan` over the tasks the app hands in, each a
// notification the system shows on arriving in its region. They take
// their share of the limit first; the times fill the rest.
//
// Every setting starts off, so nothing is scheduled and nothing is
// asked until one is turned on in Settings; permission is asked for
// then. A click on a notification is handed to the app (`onOpen`,
// `onOpenPlaces`).
//
// The one file of the Kit that is not Foundation alone: it uses
// UserNotifications, which every target that links the Kit has, and
// on iOS Core Location for a region's shape.

import Foundation
import UserNotifications
#if os(iOS)
import CoreLocation
#endif

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

    /// A place alert for several tasks was clicked: there is no one
    /// item to open. Set by the app; a click from before it was set
    /// is handed over as it is. Main queue.
    public var onOpenPlaces: (() -> Void)? {
        didSet {
            guard let onOpenPlaces, tappedPlaces else { return }
            tappedPlaces = false
            onOpenPlaces()
        }
    }

    /// The last line of a place alert with more tasks than it lists,
    /// given how many more ("and 2 more"): the app's words.
    public var moreText: (Int) -> String = { "+\($0)" }

    /// How many tasks a place alert lists by name.
    private static let listed = 3

    private let center = UNUserNotificationCenter.current()
    private var tapped: ReminderTap?
    private var tappedPlaces = false
    private var sources: [ReminderSource] = []
    private var placeSources: [PlaceAlertSource] = []
    private var origin: GeoPoint?
    /// What the centre was last given, by identifier; nil when that is
    /// not known (at launch, and while permission is denied).
    private var pending: [String: Planned]?
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

    /// The tasks a place alert may be for, as just read: none while
    /// the app may not use the phone's location, since a region is
    /// watched only with it. `origin` is where the phone last was, if
    /// that is known: the regions nearest it are the ones kept. Main
    /// queue. iOS only: nothing is planned from these elsewhere.
    public func updatePlaces(_ sources: [PlaceAlertSource], origin: GeoPoint?) {
        placeSources = sources
        self.origin = origin
        reschedule()
    }

    /// Bring the pending set in line with the items, the settings and
    /// the clock. Main queue. `settingsChanged`: the person just
    /// picked a setting, the moment to ask for permission when one is
    /// on, whether or not anything is timed yet.
    public func reschedule(settingsChanged: Bool = false) {
        let settings = ReminderSettings.load()
        var places: [PlannedPlaceAlert] = []
        var placesOn = false
        #if os(iOS)
        placesOn = PlaceAlertSettings.isOn()
        if placesOn { places = PlaceAlerts.plan(placeSources, origin: origin) }
        #endif
        // The regions are few and each stands for a place; the times
        // have what is left, and their furthest are what is cut.
        let times = Reminders.plan(sources, settings: settings, limit: Reminders.limit - places.count)
        var plan: [String: Planned] = [:]
        for reminder in times { plan[reminder.identifier] = .time(reminder) }
        for alert in places { plan[alert.identifier] = .place(alert) }
        let wanted = settingsChanged && (settings.event != .off || settings.task != .off || placesOn)
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

    private func apply(_ plan: [String: Planned], status: UNAuthorizationStatus, ask: Bool) {
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
            // Only what changed is touched: a region that stays is
            // left being watched, not dropped and registered again.
            if let pending {
                center.removePendingNotificationRequests(
                    withIdentifiers: pending.keys.filter { plan[$0] == nil })
            } else {
                center.removeAllPendingNotificationRequests()
            }
            for (identifier, planned) in plan where pending?[identifier] != planned {
                switch planned {
                case .time(let reminder): center.add(request(reminder))
                case .place(let alert):
                    #if os(iOS)
                    center.add(request(alert))
                    #else
                    _ = alert
                    #endif
                }
            }
            pending = plan
        }
    }

    /// One pending notification: a reminder for a time, or an alert
    /// for a place.
    private enum Planned: Equatable {
        case time(PlannedReminder)
        case place(PlannedPlaceAlert)
    }

    private static let docKey = "doc"
    private static let itemKey = "item"
    private static let listKey = "list"
    /// On a place alert for several tasks, which has no one item.
    private static let placesKey = "places"

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

    #if os(iOS)
    /// One task: its text, and the place under it, opening the task.
    /// Several: the places, and the tasks under them (the first few
    /// by name), opening the app's map. Shown on every arrival for as
    /// long as it is pending: until the tasks are done, or the place
    /// alerts are turned off.
    private func request(_ alert: PlannedPlaceAlert) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.sound = .default
        if alert.items.count == 1, let item = alert.items.first {
            content.title = item.title
            content.body = alert.labels.first ?? ""
            var info: [String: String] = [Self.itemKey: item.itemId, Self.listKey: item.listId]
            if let docId = item.docId { info[Self.docKey] = docId }
            content.userInfo = info
        } else {
            content.title = ListFormatter.localizedString(byJoining: alert.labels)
            var lines = alert.items.prefix(Self.listed).map(\.title)
            if alert.items.count > Self.listed { lines.append(moreText(alert.items.count - Self.listed)) }
            content.body = lines.joined(separator: "\n")
            content.userInfo = [Self.placesKey: "1"]
        }
        let region = CLCircularRegion(
            center: CLLocationCoordinate2D(latitude: alert.centre.lat, longitude: alert.centre.lon),
            radius: alert.radius, identifier: alert.identifier)
        region.notifyOnEntry = true
        region.notifyOnExit = false
        let trigger = UNLocationNotificationTrigger(region: region, repeats: true)
        return UNNotificationRequest(identifier: alert.identifier, content: content, trigger: trigger)
    }
    #endif

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
           content.userInfo[Self.placesKey] != nil {
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if let onOpenPlaces { onOpenPlaces() } else { tappedPlaces = true }
            }
        } else if response.actionIdentifier == UNNotificationDefaultActionIdentifier,
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
