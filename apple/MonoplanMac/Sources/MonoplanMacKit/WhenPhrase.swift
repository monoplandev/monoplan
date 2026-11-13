// Natural-language date phrases typed into a title ("friday night",
// "tomorrow at 5pm", "14 nov"), found and resolved into the `when`
// register's floating form, and the rule that turns a title change into
// a date suggestion (spec/calendar-plan.md "Typed date phrases"). The
// native twin of the web's `whenParse.ts` and `whenSuggest.ts`.
//
// The recogniser is Foundation's `NSDataDetector` (decided 2026-11-13:
// no chrono port worth having, and no JavaScript in the native apps).
// It reads English and Spanish without being told which, only looks
// forward ("14 nov" after the 14th is next year's), reads a bare small
// hour as afternoon ("friday at 3" is 15:00), and leaves alone the
// title words the web has to filter by hand (a lone "sat" or "sun", a
// bare month, a bare duration). It gives a range and a resolved `Date`
// and nothing else, so this file adds what the web's rules need:
//
//   - All-day or timed. The detector reports an all-day date as 12:00;
//     whether a time was actually written is its `timeIsSignificant`,
//     an undocumented key, read only if the result answers to it.
//     Without it a 12:00 result reads as all-day.
//   - A time-of-day word with no clock time ("friday night") takes the
//     web's estimated hours (morning 09:00, afternoon 15:00, evening
//     18:00, night 20:00), not the detector's (its night is 19:00).
//   - A phrase that is only a time ("3pm", "8:30") is today if that
//     time is still ahead, else tomorrow.
//   - A day already gone ("yesterday", "last friday") is no plan.
//   - Of several phrases the one ending last wins.
//
// Known differences from the web: the detector takes a meal word into
// the phrase as a time ("lunch saturday" is Saturday 12:00), has no
// bare hour ("at 3") and no "in an hour". It also resolves against the
// real clock and time zone, with no reference date to pass: `now` here
// only feeds the rules above, and must be the present.

import Foundation

public struct WhenPhrase: Equatable {
    /// The phrase as typed.
    public let text: String
    /// Normalised register value: `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`.
    public let when: String

    /// The wording with case and whitespace folded: what makes the
    /// phrase in a title before an edit and after it "the same phrase".
    public var key: String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ").lowercased()
    }

    /// The date phrase to act on in `text`, or nil when there is none.
    public static func find(in text: String, now: Date = Date()) -> WhenPhrase? {
        guard let detector, text.contains(where: { !$0.isWhitespace }) else { return nil }
        let string = text as NSString
        let matches = detector.matches(in: text, range: NSRange(location: 0, length: string.length))
        // Matches come in order and never overlap: the last one that
        // resolves is the one ending last.
        for match in matches.reversed() {
            guard let date = match.date else { continue }
            let phrase = string.substring(with: match.range)
            if let when = value(of: phrase, date: date, timed: timeWritten(match, date), now: now) {
                return WhenPhrase(text: phrase, when: when)
            }
        }
        return nil
    }

    /// The phrase is a clock time and nothing else ("3pm", "at 8:30"):
    /// its day was picked here, not typed.
    public var isTimeOnly: Bool {
        when.count >= 16 && Self.matches(Self.timeOnly, text.trimmingCharacters(in: .whitespaces))
    }

    /// What the phrase can mean typed into the date editor of an item
    /// whose date is `current`, likeliest first. Always its own value;
    /// beside it, the half the phrase did not say taken from the item:
    ///
    /// - a day alone over a timed date: that day at the item's time,
    ///   first (a new day keeps the time, as a calendar pick does);
    /// - a time alone over a date: the item's day at that time, first
    ///   unless that day is behind `today` (`YYYY-MM-DD`).
    public func readings(over current: String?, today: String) -> [String] {
        var readings = [when]
        if let current, current.count >= 10 {
            let day = String(current.prefix(10))
            if when.count < 16, current.count >= 16 {
                readings.insert(when + current.dropFirst(10).prefix(6), at: 0)
            } else if isTimeOnly {
                readings.insert(day + when.dropFirst(10), at: day < today ? 1 : 0)
            }
        }
        var seen = Set<String>()
        return readings.filter { seen.insert($0).inserted }
    }

    // MARK: Resolution

    private static let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.date.rawValue)

    /// The Gregorian calendar the register is written in, at the
    /// device's time zone: the zone the detector resolved in, so the
    /// wall-clock reading comes back out as it was typed.
    private static var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .current
        return calendar
    }

    /// Whether the phrase said a time. The detector's own answer where
    /// it has one; else anything but its all-day placeholder, noon.
    private static func timeWritten(_ match: NSTextCheckingResult, _ date: Date) -> Bool {
        if match.responds(to: NSSelectorFromString("timeIsSignificant")),
           let written = match.value(forKey: "timeIsSignificant") as? Bool {
            return written
        }
        let c = calendar.dateComponents([.hour, .minute], from: date)
        return !(c.hour == 12 && c.minute == 0)
    }

    /// Time-of-day words and the hour each estimates (the web's
    /// `TIME_OF_DAY`, both grammars: the detector does not say which
    /// language it read).
    private static let timesOfDay: [(word: NSRegularExpression, hour: Int)] = [
        (#"\bmorning\b|\b(?:por|de|en) la mañana\b"#, 9),
        (#"\bafternoon\b|\btarde\b"#, 15),
        (#"\bevening\b"#, 18),
        (#"\b(?:night|tonight)\b|\bnoche\b"#, 20),
    ].map { (try! NSRegularExpression(pattern: $0.0, options: .caseInsensitive), $0.1) }

    /// A phrase that is a clock time and nothing else: "3pm", "8:30",
    /// "a las 5".
    private static let timeOnly = try! NSRegularExpression(
        pattern: #"^(?:at|a las?|@)?\s*\d{1,2}(?:[:.h]\d{2})?\s*(?:[ap]\.?m\.?|o'clock|h)?$"#,
        options: .caseInsensitive)

    private static func matches(_ expression: NSRegularExpression, _ text: String) -> Bool {
        expression.firstMatch(in: text, range: NSRange(location: 0, length: (text as NSString).length)) != nil
    }

    private static func value(of phrase: String, date: Date, timed: Bool, now: Date) -> String? {
        let calendar = calendar
        let today = calendar.startOfDay(for: now)
        var day = calendar.startOfDay(for: date)
        guard timed else { return day < today ? nil : dayText(day, calendar) }

        var time = calendar.dateComponents([.hour, .minute], from: date)
        let trimmed = phrase.trimmingCharacters(in: .whitespaces)
        if !phrase.contains(where: \.isNumber),
           let estimate = timesOfDay.first(where: { matches($0.word, phrase) }) {
            // A word, not a clock time: our hour for it.
            time.hour = estimate.hour
            time.minute = 0
        } else if matches(timeOnly, trimmed) {
            // Time only: the day is ours to pick, the next such time.
            let at = (time.hour ?? 0) * 60 + (time.minute ?? 0)
            let clock = calendar.dateComponents([.hour, .minute], from: now)
            let ahead = at >= (clock.hour ?? 0) * 60 + (clock.minute ?? 0)
            day = ahead ? today : calendar.date(byAdding: .day, value: 1, to: today) ?? today
        }
        guard day >= today else { return nil }
        return String(format: "%@T%02d:%02d", dayText(day, calendar), time.hour ?? 0, time.minute ?? 0)
    }

    private static func dayText(_ day: Date, _ calendar: Calendar) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: day)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }
}

/// The date suggestion a title raises: the web's `whenSuggest.ts` rule.
/// A date phrase typed into a title never changes the title and never
/// sets a date by itself; it writes the item's `when_suggested`, a
/// question the user answers yes or no on the item's page.
public enum WhenSuggest {
    /// What a title edit does to the item's pending suggestion.
    ///
    /// - A phrase the old title did not have (compared by wording) is
    ///   resolved now and raised. The same wording is never asked
    ///   again, so fixing a typo elsewhere in the title leaves a "no"
    ///   standing.
    /// - A phrase edited out of the title takes its unanswered
    ///   suggestion with it.
    /// - A closed item or one with a date takes no suggestion.
    ///
    /// `.keep` whenever nothing would change, so no empty write is made.
    public static func edit(
        title: String, was oldTitle: String, when: String?, pending: String?, closed: Bool,
        now: Date = Date()
    ) -> SuggestionEdit {
        if closed || when != nil { return .keep }
        let baseline = WhenPhrase.find(in: oldTitle, now: now)?.key
        guard let phrase = WhenPhrase.find(in: title, now: now) else {
            return baseline != nil && pending != nil ? .clear : .keep
        }
        return phrase.key != baseline && phrase.when != pending ? .set(phrase.when) : .keep
    }
}
