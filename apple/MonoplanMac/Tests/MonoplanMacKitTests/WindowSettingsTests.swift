import XCTest
@testable import MonoplanMacKit

final class WindowSettingsTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "app.monoplan.mac.tests.\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    private let us = Locale(identifier: "en_US")

    func testTypedDatesDefaultsOnAndRoundTrips() {
        let d = defaults()
        XCTAssertEqual(WindowSettings.load(from: d, locale: us, localizations: ["en"]).typedDates, true)
        var s = WindowSettings.load(from: d, locale: us, localizations: ["en"])
        s.typedDates = false
        s.save(to: d)
        XCTAssertEqual(WindowSettings.load(from: d, locale: us, localizations: ["en"]).typedDates, false)
        // The system values are not persisted.
        XCTAssertNil(d.object(forKey: "language"))
        XCTAssertNil(d.object(forKey: "hourCycle"))
    }

    func testListCountsDefaultsOffAndRoundTrips() {
        let d = defaults()
        XCTAssertEqual(WindowSettings.load(from: d, locale: us, localizations: ["en"]).listCounts, false)
        var s = WindowSettings.load(from: d, locale: us, localizations: ["en"])
        s.listCounts = true
        s.save(to: d)
        let back = WindowSettings.load(from: d, locale: us, localizations: ["en"])
        XCTAssertEqual(back.listCounts, true)
        XCTAssertEqual(back.typedDates, true)
    }

    func testLanguageIsTheAppsOwnSpanishOrEnglish() {
        XCTAssertEqual(WindowSettings.language(of: ["es"]), "es")
        XCTAssertEqual(WindowSettings.language(of: ["es-419", "en"]), "es")
        XCTAssertEqual(WindowSettings.language(of: ["en", "es"]), "en")
        XCTAssertEqual(WindowSettings.language(of: ["fr"]), "en")
        XCTAssertEqual(WindowSettings.language(of: []), "en")
    }

    func testHourCycleFollowsTheLocale() {
        XCTAssertEqual(WindowSettings.hourCycle(of: Locale(identifier: "en_US")), "12h")
        XCTAssertEqual(WindowSettings.hourCycle(of: Locale(identifier: "es_ES")), "24h")
        // The 24-Hour Time switch shows up as the locale's hours.
        XCTAssertEqual(WindowSettings.hourCycle(of: Locale(identifier: "en_US@hours=h23")), "24h")
        XCTAssertEqual(WindowSettings.hourCycle(of: Locale(identifier: "es_ES@hours=h12")), "12h")
    }

    func testScripts() {
        let s = WindowSettings(typedDates: false, language: "es", hourCycle: "24h")
        XCTAssertEqual(s.userScript, "window.__monoplanSettings = {\"hourCycle\":\"24h\",\"language\":\"es\",\"listCounts\":false,\"typedDates\":false};")
        XCTAssertEqual(
            HostInbound.settings(s).script,
            "window.__monoplanHost.settings({\"hourCycle\":\"24h\",\"language\":\"es\",\"listCounts\":false,\"typedDates\":false})")
    }
}
