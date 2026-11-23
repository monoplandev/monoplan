// Place alerts (spec/place-alerts.md), the half that needs the phone's
// location: whether the app may use it, and where the phone last was.
// The system watches the regions and shows the alerts itself, with the
// app not running, on when-in-use permission alone; the app only has
// to hold that permission, with precise location, and to say which
// regions are nearest when there are more than the system will watch.
//
// Nothing here runs until the setting is on. The fix is one request
// each time the app comes to the front, kept in memory only, and never
// leaves the phone.

import CoreLocation
import MonoplanMacKit

final class PlaceAlertLocation: NSObject, ObservableObject, CLLocationManagerDelegate {
    enum Access {
        /// Not asked yet.
        case unknown
        case allowed
        /// Allowed, without precise location: the system watches no
        /// region for the app.
        case approximate
        case denied
    }

    @Published private(set) var access: Access = .unknown
    /// Where the phone last was, once a fix has come in.
    private(set) var origin: GeoPoint?
    /// The access or the fix changed. Main queue.
    var onChange: (() -> Void)?

    private let manager = CLLocationManager()
    /// A fix is wanted as soon as there is permission for one.
    private var wanted = false

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
        access = Self.access(of: manager)
    }

    /// The setting is on: ask for permission if that has not been
    /// settled, and for one fix when there is permission.
    func locate() {
        wanted = true
        switch manager.authorizationStatus {
        case .notDetermined: manager.requestWhenInUseAuthorization()
        case .authorizedWhenInUse, .authorizedAlways: manager.requestLocation()
        default: break
        }
    }

    private static func access(of manager: CLLocationManager) -> Access {
        switch manager.authorizationStatus {
        case .notDetermined: return .unknown
        case .authorizedWhenInUse, .authorizedAlways:
            return manager.accuracyAuthorization == .fullAccuracy ? .allowed : .approximate
        default: return .denied
        }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let next = Self.access(of: manager)
        if next == .denied { origin = nil }
        if (next == .allowed || next == .approximate) && wanted { manager.requestLocation() }
        guard next != access else { return }
        access = next
        onChange?()
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let last = locations.last else { return }
        origin = GeoPoint(lat: last.coordinate.latitude, lon: last.coordinate.longitude)
        onChange?()
    }

    /// No fix this time: the regions are picked from the last one, or
    /// by which places are newest.
    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {}
}
