import XCTest
@testable import MonoplanMacKit

final class PageZoomTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "app.monoplan.mac.tests.\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    func testStepsWalkTheLadderAndStopAtTheEnds() {
        XCTAssertEqual(PageZoom.stepIn(from: 1), 1.1)
        XCTAssertEqual(PageZoom.stepOut(from: 1), 0.9)
        XCTAssertEqual(PageZoom.stepIn(from: 1.2), 1.2)
        XCTAssertEqual(PageZoom.stepOut(from: 0.8), 0.8)
        XCTAssertEqual(PageZoom.stepIn(from: PageZoom.stepIn(from: 1)), 1.2)
        XCTAssertEqual(PageZoom.stepOut(from: PageZoom.stepOut(from: 1)), 0.8)
        // Off the ladder snaps to the neighbouring level.
        XCTAssertEqual(PageZoom.stepIn(from: 1.05), 1.1)
        XCTAssertEqual(PageZoom.stepOut(from: 1.05), 1)
    }

    func testLoadDefaultsToOneAndRoundTrips() {
        let d = defaults()
        XCTAssertEqual(PageZoom.load(from: d), 1)
        PageZoom.save(1.2, to: d)
        XCTAssertEqual(PageZoom.load(from: d), 1.2)
        // Back to 1 clears the key.
        PageZoom.save(1, to: d)
        XCTAssertNil(d.object(forKey: PageZoom.defaultsKey))
        XCTAssertEqual(PageZoom.load(from: d), 1)
        // A value off the ladder, or of the wrong type, reads as 1.
        d.set(1.05, forKey: PageZoom.defaultsKey)
        XCTAssertEqual(PageZoom.load(from: d), 1)
        d.set("big", forKey: PageZoom.defaultsKey)
        XCTAssertEqual(PageZoom.load(from: d), 1)
    }
}
