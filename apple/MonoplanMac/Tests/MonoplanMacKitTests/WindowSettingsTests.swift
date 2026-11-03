import XCTest
@testable import MonoplanMacKit

final class WindowSettingsTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "app.monoplan.mac.tests.\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    func testTypedDatesDefaultsOnAndRoundTrips() {
        let d = defaults()
        XCTAssertEqual(WindowSettings.load(from: d), WindowSettings(typedDates: true))
        WindowSettings(typedDates: false).save(to: d)
        XCTAssertEqual(WindowSettings.load(from: d), WindowSettings(typedDates: false))
    }

    func testScripts() {
        let s = WindowSettings(typedDates: false)
        XCTAssertEqual(s.userScript, "window.__monoplanSettings = {\"typedDates\":false};")
        XCTAssertEqual(HostInbound.settings(s).script, "window.__monoplanHost.settings({\"typedDates\":false})")
    }
}
