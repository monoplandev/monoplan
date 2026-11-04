import XCTest
@testable import MonoplanMacKit

final class MirrorFramingTests: XCTestCase {
    func testParsesEveryRequestKind() throws {
        XCTAssertEqual(try HostRequest(message: ["kind": "session"]), .session)
        XCTAssertEqual(try HostRequest(message: ["kind": "subscribe", "docId": "d1"]), .subscribe(docId: "d1"))
        XCTAssertEqual(try HostRequest(message: ["kind": "unsubscribe", "docId": "d1"]), .unsubscribe(docId: "d1"))
        let bytes = Data([1, 2, 3, 250])
        XCTAssertEqual(
            try HostRequest(message: ["kind": "import", "docId": "d1", "bytes": bytes.base64EncodedString()]),
            .import(docId: "d1", bytes: bytes))
        XCTAssertEqual(try HostRequest(message: ["kind": "window", "action": "drag"]), .window(.drag))
        XCTAssertEqual(try HostRequest(message: ["kind": "window", "action": "doubleClick"]), .window(.doubleClick))
        XCTAssertEqual(
            try HostRequest(message: ["kind": "undoState", "canUndo": true, "canRedo": false]),
            .undoState(canUndo: true, canRedo: false))
    }

    func testRejectsMalformedRequests() {
        XCTAssertThrowsError(try HostRequest(message: "session")) { XCTAssertEqual($0 as? HostRequest.ParseError, .notAnObject) }
        XCTAssertThrowsError(try HostRequest(message: ["kind": "dance"])) { XCTAssertEqual($0 as? HostRequest.ParseError, .unknownKind("dance")) }
        XCTAssertThrowsError(try HostRequest(message: ["kind": "subscribe"])) { XCTAssertEqual($0 as? HostRequest.ParseError, .missing("docId")) }
        XCTAssertThrowsError(try HostRequest(message: ["kind": "import", "docId": "d", "bytes": "%%%"])) {
            XCTAssertEqual($0 as? HostRequest.ParseError, .badBase64("bytes"))
        }
        XCTAssertThrowsError(try HostRequest(message: ["kind": "window"])) { XCTAssertEqual($0 as? HostRequest.ParseError, .missing("action")) }
        XCTAssertThrowsError(try HostRequest(message: ["kind": "undoState", "canUndo": true])) {
            XCTAssertEqual($0 as? HostRequest.ParseError, .missing("canRedo"))
        }
        XCTAssertThrowsError(try HostRequest(message: ["kind": "window", "action": "wiggle"])) {
            XCTAssertEqual($0 as? HostRequest.ParseError, .badValue("action"))
        }
    }

    func testSubscribeReplyCarriesBase64AndDecimalPeer() {
        let reply = HostReply.subscribe(snapshot: Data([9, 8, 7]), peer: UInt64.max - 1)
        XCTAssertEqual(reply["snapshot"] as? String, "CQgH")
        // A u64 beyond Number's precision travels as text.
        XCTAssertEqual(reply["peer"] as? String, "18446744073709551614")
    }

    func testSessionReplyShape() {
        let s = WindowSession(
            accountId: "a", email: "e@x", deviceId: "dev", primaryDocId: "home",
            serverURL: "https://alpha.monoplan.app",
            docs: [.init(docId: "home", name: nil), .init(docId: "other", name: "Other")])
        let r = s.reply
        XCTAssertEqual(r["accountId"] as? String, "a")
        XCTAssertEqual(r["serverUrl"] as? String, "https://alpha.monoplan.app")
        let docs = r["docs"] as? [[String: Any]]
        XCTAssertEqual(docs?.count, 2)
        XCTAssertEqual(docs?[0]["docId"] as? String, "home")
        XCTAssertTrue(docs?[0]["name"] is NSNull)
        XCTAssertEqual(docs?[1]["name"] as? String, "Other")
        // The reply is what a WKScriptMessage reply handler can serialise.
        XCTAssertTrue(JSONSerialization.isValidJSONObject(r))
    }

    func testInboundScriptsQuoteTheirArguments() {
        XCTAssertEqual(
            HostInbound.update(docId: "d1", bytes: Data([0, 255])).script,
            "window.__monoplanHost.update(\"d1\", \"AP8=\")")
        XCTAssertEqual(HostInbound.stopped(docId: "d\"1").script, "window.__monoplanHost.stopped(\"d\\\"1\")")
        XCTAssertEqual(
            HostInbound.sync(online: true, lastSyncedAt: 1_700_000_000_000).script,
            "window.__monoplanHost.sync({online: true, lastSyncedAt: 1700000000000})")
        XCTAssertEqual(
            HostInbound.sync(online: false, lastSyncedAt: nil).script,
            "window.__monoplanHost.sync({online: false, lastSyncedAt: null})")
        XCTAssertEqual(HostInbound.handOff.script, "window.__monoplanHost.handOff()")
        XCTAssertEqual(HostInbound.undo.script, "window.__monoplanHost.undo()")
        XCTAssertEqual(HostInbound.redo.script, "window.__monoplanHost.redo()")
        XCTAssertEqual(HostInbound.find.script, "window.__monoplanHost.find()")
        XCTAssertEqual(HostInbound.fullScreen(true).script, "window.__monoplanHost.fullScreen(true)")
        XCTAssertEqual(HostInbound.fullScreen(false).script, "window.__monoplanHost.fullScreen(false)")
    }

    func testSchemeRoutes() {
        XCTAssertEqual(AppScheme.entryURL().absoluteString, "monoplan://app/mirror.html")
        XCTAssertEqual(AppScheme.entryURL(fragment: "item_abc").absoluteString, "monoplan://app/mirror.html#item_abc")
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://app/")!), .file("mirror.html"))
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://app")!), .file("mirror.html"))
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://app/mirror.html#item_x")!), .file("mirror.html"))
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://app/assets/main-abc.js?v=1")!), .file("assets/main-abc.js"))
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://app/api/devices")!), .api(pathAndQuery: "/api/devices"))
        XCTAssertEqual(
            AppScheme.route(URL(string: "monoplan://app/api/docs/x/members?limit=5")!),
            .api(pathAndQuery: "/api/docs/x/members?limit=5"))
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://app/assets/../secrets")!), .notFound)
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://app/assets/%2e%2e/secrets")!), .notFound)
        XCTAssertEqual(AppScheme.route(URL(string: "monoplan://evil/mirror.html")!), .notFound)
        XCTAssertEqual(AppScheme.route(URL(string: "https://app/mirror.html")!), .notFound)
    }

    func testMimeTypes() {
        XCTAssertEqual(AppScheme.mimeType(for: "mirror.html"), "text/html; charset=utf-8")
        XCTAssertEqual(AppScheme.mimeType(for: "assets/monoplan_core_web_bg-CK9.wasm"), "application/wasm")
        XCTAssertEqual(AppScheme.mimeType(for: "assets/x.js"), "text/javascript; charset=utf-8")
        XCTAssertEqual(AppScheme.mimeType(for: "assets/styles.css"), "text/css; charset=utf-8")
        XCTAssertEqual(AppScheme.mimeType(for: "assets/emoji-data.json"), "application/json; charset=utf-8")
        XCTAssertEqual(AppScheme.mimeType(for: "favicon.ico"), "image/x-icon")
        XCTAssertEqual(AppScheme.mimeType(for: "x.unknownext"), "application/octet-stream")
    }
}
