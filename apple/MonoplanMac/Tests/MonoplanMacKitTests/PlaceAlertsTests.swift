import XCTest
@testable import MonoplanMacKit

final class PlaceAlertsTests: XCTestCase {
    /// A point `north` and `east` metres from a spot in Madrid.
    private func point(north: Double = 0, east: Double = 0) -> GeoPoint {
        let lat = 40.4168 + north / 111_195
        let lon = -3.7038 + east / (111_195 * cos(40.4168 * .pi / 180))
        return GeoPoint(lat: lat, lon: lon)
    }

    private func source(
        _ id: String, _ label: String, _ at: GeoPoint, createdAt: Int64 = 0
    ) -> PlaceAlertSource {
        PlaceAlertSource(
            docId: "d", itemId: id, listId: "inbox", title: id, label: label, at: at, createdAt: createdAt)
    }

    func testMetresBetweenPoints() {
        XCTAssertEqual(point().metres(to: point(north: 1000)), 1000, accuracy: 1)
        XCTAssertEqual(point().metres(to: point(east: 250)), 250, accuracy: 1)
    }

    func testTasksAtOneSpotShareARegion() {
        let plan = PlaceAlerts.plan(
            [source("a", "Shop", point()), source("b", "Shop", point()), source("c", "Far", point(north: 5000))],
            origin: point())
        XCTAssertEqual(plan.map { $0.items.map(\.itemId) }, [["a", "b"], ["c"]])
        XCTAssertEqual(plan.map(\.labels), [["Shop"], ["Far"]])
        XCTAssertEqual(plan.map(\.radius), [150, 150])
    }

    func testSpotsWithinARadiusMergeAndTheRegionGrowsToCoverThem() {
        let plan = PlaceAlerts.plan(
            [source("a", "Hardware", point()), source("b", "Stationer", point(east: 80))],
            origin: nil)
        XCTAssertEqual(plan.count, 1)
        XCTAssertEqual(plan[0].labels, ["Hardware", "Stationer"])
        XCTAssertEqual(plan[0].items.map(\.itemId), ["a", "b"])
        XCTAssertEqual(plan[0].radius, 190, accuracy: 1)
        XCTAssertEqual(plan[0].centre.metres(to: point(east: 40)), 0, accuracy: 1)
    }

    func testSpotsFurtherApartThanARadiusStayApart() {
        let plan = PlaceAlerts.plan(
            [source("a", "A", point()), source("b", "B", point(east: 200))], origin: point())
        XCTAssertEqual(plan.map { $0.items.map(\.itemId) }, [["a"], ["b"]])
    }

    func testAChainOfNeighboursStopsAtTheWidestRegion() {
        // Each 120 m from the last: one region over all five would
        // reach 240 m from its middle, 390 m with the base radius.
        let sources = (0..<5).map { source("s\($0)", "P\($0)", point(east: Double($0) * 120)) }
        let plan = PlaceAlerts.plan(sources, origin: nil)
        XCTAssertGreaterThan(plan.count, 1)
        XCTAssertEqual(plan.flatMap(\.items).count, 5)
        for alert in plan { XCTAssertLessThanOrEqual(alert.radius, PlaceAlerts.maxRadius) }
    }

    func testTheSameRegionsWhateverOrderTheTasksComeIn() {
        let sources = (0..<6).map { source("s\($0)", "P\($0)", point(north: Double($0 % 3) * 90, east: Double($0) * 70)) }
        let one = PlaceAlerts.plan(sources, origin: point())
        let other = PlaceAlerts.plan(sources.reversed(), origin: point())
        XCTAssertEqual(one.map(\.identifier), other.map(\.identifier))
        XCTAssertEqual(one.map(\.radius), other.map(\.radius))
    }

    func testTheNearestRegionsAreKept() {
        let sources = (1...30).map { source("s\($0)", "P\($0)", point(north: Double($0) * 1000)) }
        let plan = PlaceAlerts.plan(sources.shuffled(), origin: point())
        XCTAssertEqual(plan.count, PlaceAlerts.limit)
        XCTAssertEqual(plan.map { $0.items[0].itemId }, (1...20).map { "s\($0)" })
    }

    func testWithNoLocationTheNewestPlacesAreKept() {
        let sources = (1...5).map {
            source("s\($0)", "P\($0)", point(north: Double($0) * 1000), createdAt: Int64($0))
        }
        let plan = PlaceAlerts.plan(sources, origin: nil, limit: 2)
        XCTAssertEqual(plan.map { $0.items[0].itemId }, ["s5", "s4"])
    }

    func testARegionAcrossTheAntimeridianIsCentredOnIt() {
        let plan = PlaceAlerts.plan(
            [
                source("a", "West", GeoPoint(lat: 0, lon: 179.9997)),
                source("b", "East", GeoPoint(lat: 0, lon: -179.9997)),
            ], origin: nil)
        XCTAssertEqual(plan.count, 1)
        XCTAssertEqual(abs(plan[0].centre.lon), 180, accuracy: 0.0001)
    }
}
