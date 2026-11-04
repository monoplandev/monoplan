import XCTest
@testable import MonoplanMacKit

final class LaunchPreferenceTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "app.monoplan.mac.tests.\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    func testOpenWindowDefaultsOnAndRoundTrips() {
        let d = defaults()
        XCTAssertTrue(LaunchPreference.openWindow(from: d))
        LaunchPreference.setOpenWindow(false, to: d)
        XCTAssertFalse(LaunchPreference.openWindow(from: d))
        LaunchPreference.setOpenWindow(true, to: d)
        XCTAssertTrue(LaunchPreference.openWindow(from: d))
        // A non-boolean value is ignored, not taken as off.
        d.set("yes", forKey: LaunchPreference.openWindowKey)
        XCTAssertTrue(LaunchPreference.openWindow(from: d))
    }
}
