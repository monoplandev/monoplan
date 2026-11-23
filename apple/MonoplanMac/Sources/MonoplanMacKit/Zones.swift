// Time zones for the zone pickers of the Mac panel and the iOS date
// editor (`spec/calendar-plan.md` "Fixed instants"): the web's
// `js/web/src/zones.ts`, on Foundation.
//
// The division of labour is the web's: the core owns every conversion
// (one tzdb, one rule for the hours a zone skips or repeats; the apps
// reach it through `WhenZone` in MonoplanCore). Foundation only
// supplies what a person reads and picks from: the list of zone ids,
// each zone's name in the device's language ("Central European Time")
// and its offset on the item's day ("GMT+1"). A label a little out at
// a transition is a label, never a stored value.
//
// A zone is named by its city, read out of the id ("Europe/Paris" is
// "Paris", "America/Argentina/Buenos_Aires" is "Buenos Aires").
//
// Matching: every typed word must begin a word of the zone's city, id,
// name or offset, accents and case aside ("new y", "eastern", "gmt+1",
// "utc+1", "sao paulo"). Zones whose city starts with the query lead,
// then those with a city word starting with its first word, then the
// rest; each tier by city. A blank query is every zone by city, with
// the device's own first.

import Foundation

/// One zone as a picker shows it.
public struct ZoneOption: Equatable, Identifiable, Sendable {
    /// IANA id, as stored.
    public let id: String
    /// The id's last segment, underscores as spaces.
    public let city: String
    /// The zone's name in the device's language, or "" when Foundation
    /// has none.
    public let name: String
    /// Offset on the day asked about, e.g. "GMT+1"; "" when unknown.
    public let offset: String

    public init(id: String, city: String, name: String, offset: String) {
        self.id = id
        self.city = city
        self.name = name
        self.offset = offset
    }

    /// What a closed zone field reads: the offset and the city,
    /// "GMT+1 Paris".
    public var label: String { offset.isEmpty ? city : "\(offset) \(city)" }
}

public enum Zones {
    /// The device's IANA zone.
    public static func deviceZone() -> String { TimeZone.current.identifier }

    /// Every zone id Foundation lists, plus UTC, which it leaves out,
    /// and the device's own.
    public static func ids(
        known: [String] = TimeZone.knownTimeZoneIdentifiers, device: String? = deviceZone()
    ) -> [String] {
        var seen = Set<String>()
        return (known + ["UTC"] + (device.map { [$0] } ?? [])).filter { seen.insert($0).inserted }
    }

    public static func city(_ id: String) -> String {
        let last = id.split(separator: "/").last.map(String.init) ?? id
        return last.replacingOccurrences(of: "_", with: " ")
    }

    /// Noon UTC on a `YYYY-MM-DD` day: the instant offsets are quoted
    /// at. Now, when the day does not parse.
    static func noonUTC(_ day: String) -> Date {
        let parts = day.prefix(10).split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3 else { return Date() }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0) ?? .gmt
        return calendar.date(
            from: DateComponents(year: parts[0], month: parts[1], day: parts[2], hour: 12)) ?? Date()
    }

    /// "GMT", "GMT+1", "GMT-3:30".
    public static func offsetText(seconds: Int) -> String {
        let minutes = abs(seconds) / 60
        guard minutes != 0 else { return "GMT" }
        let sign = seconds < 0 ? "-" : "+"
        let rest = minutes % 60 == 0 ? "" : String(format: ":%02d", minutes % 60)
        return "GMT\(sign)\(minutes / 60)\(rest)"
    }

    /// One zone as a picker shows it, offset as of `day`
    /// (`YYYY-MM-DD`). A zone Foundation does not know (it came from
    /// another device) has its city alone.
    public static func option(_ id: String, locale: Locale = .current, day: String) -> ZoneOption {
        guard let zone = TimeZone(identifier: id) else {
            return ZoneOption(id: id, city: city(id), name: "", offset: "")
        }
        return ZoneOption(
            id: id, city: city(id),
            name: zone.localizedName(for: .generic, locale: locale) ?? "",
            offset: offsetText(seconds: zone.secondsFromGMT(for: noonUTC(day))))
    }

    private static let lock = NSLock()
    private static var cache: (key: String, options: [ZoneOption])?

    /// Every zone by city, offsets as of `day`. Some hundreds of zones
    /// to name, so the last answer is kept: a picker asks again on
    /// every keystroke.
    public static func options(locale: Locale = .current, day: String) -> [ZoneOption] {
        let key = "\(locale.identifier)|\(day.prefix(10))|\(deviceZone())"
        lock.lock()
        defer { lock.unlock() }
        if let cache, cache.key == key { return cache.options }
        let options = ids().map { option($0, locale: locale, day: day) }.sorted {
            let order = $0.city.compare($1.city, options: [.caseInsensitive, .diacriticInsensitive], locale: locale)
            return order == .orderedSame ? $0.id < $1.id : order == .orderedAscending
        }
        cache = (key, options)
        return options
    }

    private static func fold(_ text: String) -> String {
        text.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: nil).lowercased()
    }

    private static let separators = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "/_,()"))

    static func words(_ text: String) -> [String] {
        fold(text).components(separatedBy: separators).filter { !$0.isEmpty }
    }

    /// The zones a typed query names, best first; `first` (the
    /// device's zone) leads the blank list.
    public static func suggestions(
        _ query: String, in options: [ZoneOption], first: String? = nil
    ) -> [ZoneOption] {
        let typed = words(query)
        guard let lead = typed.first else {
            guard let index = options.firstIndex(where: { $0.id == first }) else { return options }
            var rest = options
            let top = rest.remove(at: index)
            return [top] + rest
        }
        let whole = typed.joined(separator: " ")
        var tiers: [[ZoneOption]] = [[], [], []]
        for option in options {
            let city = words(option.city)
            // "utc+1" finds what is written "GMT+1".
            let offset = fold(option.offset)
            let hay = city + words(option.id) + words(option.name)
                + [offset, offset.replacingOccurrences(of: "gmt", with: "utc")]
            guard typed.allSatisfy({ word in hay.contains { $0.hasPrefix(word) } }) else { continue }
            let tier = city.joined(separator: " ").hasPrefix(whole)
                ? 0 : (city.contains { $0.hasPrefix(lead) } ? 1 : 2)
            tiers[tier].append(option)
        }
        return tiers.flatMap { $0 }
    }
}
