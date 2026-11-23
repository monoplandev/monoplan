import XCTest
@testable import MonoplanMacKit

/// The zone pickers' list and matching, as the web's `zones.test.ts`.
final class ZonesTests: XCTestCase {
    private let english = Locale(identifier: "en_GB")

    private func options(_ ids: [String], day: String = "2026-07-13") -> [ZoneOption] {
        ids.map { Zones.option($0, locale: english, day: day) }
    }

    func testCityIsTheLastSegmentOfTheId() {
        XCTAssertEqual(Zones.city("Europe/Paris"), "Paris")
        XCTAssertEqual(Zones.city("America/Argentina/Buenos_Aires"), "Buenos Aires")
        XCTAssertEqual(Zones.city("UTC"), "UTC")
    }

    func testIdsAddUTCAndTheDeviceZoneOnce() {
        let ids = Zones.ids(known: ["Europe/Paris", "Asia/Tokyo"], device: "Asia/Tokyo")
        XCTAssertEqual(ids, ["Europe/Paris", "Asia/Tokyo", "UTC"])
        XCTAssertEqual(Zones.ids(known: ["UTC"], device: "Mars/Olympus"), ["UTC", "Mars/Olympus"])
    }

    func testOffsetText() {
        XCTAssertEqual(Zones.offsetText(seconds: 0), "GMT")
        XCTAssertEqual(Zones.offsetText(seconds: 3600), "GMT+1")
        XCTAssertEqual(Zones.offsetText(seconds: -3 * 3600 - 1800), "GMT-3:30")
        XCTAssertEqual(Zones.offsetText(seconds: 5 * 3600 + 45 * 60), "GMT+5:45")
    }

    func testOffsetIsTheOneOnTheDayAsked() {
        XCTAssertEqual(Zones.option("Europe/Paris", locale: english, day: "2026-07-13").offset, "GMT+2")
        XCTAssertEqual(Zones.option("Europe/Paris", locale: english, day: "2026-01-13").offset, "GMT+1")
        // A timed value's day is read off its front.
        XCTAssertEqual(Zones.option("Europe/Paris", locale: english, day: "2026-01-13T09:00").offset, "GMT+1")
    }

    func testLabelIsOffsetThenCity() {
        XCTAssertEqual(Zones.option("Europe/Paris", locale: english, day: "2026-01-13").label, "GMT+1 Paris")
    }

    func testAnUnknownZoneKeepsItsCity() {
        let option = Zones.option("Europe/Atlantis", locale: english, day: "2026-07-13")
        XCTAssertEqual(option, ZoneOption(id: "Europe/Atlantis", city: "Atlantis", name: "", offset: ""))
        XCTAssertEqual(option.label, "Atlantis")
    }

    func testBlankQueryIsEveryZoneWithTheDeviceFirst() {
        let all = options(["Europe/London", "Europe/Paris", "Asia/Tokyo"])
        XCTAssertEqual(Zones.suggestions("", in: all).map(\.id), all.map(\.id))
        XCTAssertEqual(
            Zones.suggestions("  ", in: all, first: "Asia/Tokyo").map(\.id),
            ["Asia/Tokyo", "Europe/London", "Europe/Paris"])
        XCTAssertEqual(Zones.suggestions("", in: all, first: "Mars/Olympus").map(\.id), all.map(\.id))
    }

    func testEveryTypedWordMustBeginAWord() {
        let all = options(["America/New_York", "America/North_Dakota/New_Salem", "Europe/Paris", "America/Sao_Paulo"])
        XCTAssertEqual(Zones.suggestions("new y", in: all).map(\.id), ["America/New_York"])
        XCTAssertEqual(Zones.suggestions("sao paulo", in: all).map(\.id), ["America/Sao_Paulo"])
        XCTAssertEqual(Zones.suggestions("SÃO", in: all).map(\.id), ["America/Sao_Paulo"])
        XCTAssertEqual(Zones.suggestions("ork", in: all).map(\.id), [])
        // The id's other segments match too.
        XCTAssertEqual(Zones.suggestions("europe", in: all).map(\.id), ["Europe/Paris"])
    }

    func testOffsetsMatchAsGMTOrUTC() {
        let all = options(["Europe/Paris", "Asia/Tokyo"], day: "2026-01-13")
        XCTAssertEqual(Zones.suggestions("gmt+1", in: all).map(\.id), ["Europe/Paris"])
        XCTAssertEqual(Zones.suggestions("utc+9", in: all).map(\.id), ["Asia/Tokyo"])
    }

    func testCityMatchesLead() {
        // Tier 0: the city starts with what is typed. Tier 1: a later
        // city word does. Tier 2: only the name or the id does
        // ("Central European Time").
        let all = options(["Europe/Paris", "America/Costa_Rica", "America/Puerto_Rico", "America/Chicago"])
        XCTAssertEqual(
            Zones.suggestions("c", in: all).map(\.id),
            ["America/Costa_Rica", "America/Chicago", "Europe/Paris"])
        XCTAssertEqual(
            Zones.suggestions("rico", in: all).map(\.id), ["America/Puerto_Rico"])
        XCTAssertEqual(
            Zones.suggestions("ri", in: all).map(\.id), ["America/Costa_Rica", "America/Puerto_Rico"])
    }

    func testOptionsAreByCity() {
        let cities = Zones.options(locale: english, day: "2026-07-13").map(\.city)
        XCTAssertTrue(cities.contains("UTC"))
        XCTAssertTrue(cities.contains("Paris"))
        XCTAssertLessThan(cities.firstIndex(of: "Paris") ?? 0, cities.firstIndex(of: "Tokyo") ?? 0)
    }
}
