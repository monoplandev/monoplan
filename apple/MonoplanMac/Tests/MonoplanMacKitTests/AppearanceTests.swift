import XCTest
@testable import MonoplanMacKit

final class AppearanceTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "app.monoplan.mac.tests.\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    func testAbsentOrUnknownReadsAsSystem() {
        let d = defaults()
        XCTAssertEqual(AppearancePreference.load(from: d), .system)
        d.set("sepia", forKey: AppearancePreference.defaultsKey)
        XCTAssertEqual(AppearancePreference.load(from: d), .system)
    }

    func testRoundTripAndSystemClearsTheKey() {
        let d = defaults()
        AppearancePreference.dark.save(to: d)
        XCTAssertEqual(d.string(forKey: AppearancePreference.defaultsKey), "dark")
        XCTAssertEqual(AppearancePreference.load(from: d), .dark)
        AppearancePreference.light.save(to: d)
        XCTAssertEqual(AppearancePreference.load(from: d), .light)
        AppearancePreference.system.save(to: d)
        XCTAssertNil(d.object(forKey: AppearancePreference.defaultsKey))
        XCTAssertEqual(AppearancePreference.load(from: d), .system)
    }
}
