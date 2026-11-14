// The agenda projection as `monoplan agenda --json` prints it (the
// `--json` contract is in spec/cli.md): what the iOS widget's snapshot
// file holds (`AgendaSnapshot`). The apps read the engine's typed
// `AgendaGroup` instead; the Mac tray's rendering of it is in the app
// target (`TrayMenu.swift`).

import Foundation

public struct AgendaDay: Decodable, Equatable {
    /// `YYYY-MM-DD`, or nil for the Unscheduled group.
    public let day: String?
    public let today: Bool
    public let rows: [AgendaRow]
}

public struct AgendaRow: Decodable, Equatable {
    public let id: String
    public let text: String
    public let listId: String
    public let state: String
    /// `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed, floating).
    public let when: String?
    /// Minutes.
    public let duration: Int?
    public let deadline: String?
    /// The item's place, when it has one.
    public let place: PlaceInfo?
    public let placedBy: String?
    public let tone: String
    /// `--all` only: the row's workspace. `docName` is nil for Home
    /// ("Personal") and for an unnamed doc.
    public let docId: String?
    public let docName: String?

    public var isDone: Bool { state == "done" }
    public var isTimed: Bool { (when?.count ?? 0) >= 16 }
}

public enum Agenda {
    public static func parse(_ json: String) throws -> [AgendaDay] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([AgendaDay].self, from: Data(json.utf8))
    }

    /// Start of a `when`, in the local calendar. Floating values are
    /// wall-clock intents (spec/calendar-plan.md), so local is the only
    /// sensible reading; a reserved `[Zone]` suffix is ignored for now.
    public static func start(of when: String, calendar: Calendar = .current) -> Date? {
        let core = when.split(separator: "[", maxSplits: 1).first.map(String.init) ?? when
        guard core.count >= 10,
              let y = Int(core.prefix(4)),
              let m = Int(core.dropFirst(5).prefix(2)),
              let d = Int(core.dropFirst(8).prefix(2))
        else { return nil }
        var comps = DateComponents(year: y, month: m, day: d)
        if core.count >= 16 {
            guard let hh = Int(core.dropFirst(11).prefix(2)),
                  let mm = Int(core.dropFirst(14).prefix(2))
            else { return nil }
            comps.hour = hh
            comps.minute = mm
        }
        return calendar.date(from: comps)
    }
}
