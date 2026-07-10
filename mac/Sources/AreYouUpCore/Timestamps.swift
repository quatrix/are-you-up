import Foundation

// ============================================================================
// RFC 3339 timestamps carrying the device's local UTC offset (ADR-0004),
// e.g. "2026-07-10T23:41:03+03:00". Used for samples, logs, and the API.
// ============================================================================

public enum Timestamps {
    // DateFormatter rather than ISO8601DateFormatter: the latter can only
    // emit Z-normalized UTC, and we want the local offset preserved.
    //
    // TODO: DateFormatter reads the system timezone once; if the mac ever
    // observes stale local-time rendering after a timezone change with no
    // process restart, listen for NSSystemTimeZoneDidChangeNotification and
    // rebuild the formatter. Stamped instants stay correct either way (this
    // only affects the offset shown in newly formatted strings), so this is
    // a display-only concern, not a data-correctness one.
    private static let formatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd'T'HH:mm:ssXXXXX"
        formatter.locale = Locale(identifier: "en_US_POSIX")
        return formatter
    }()

    public static func string(from date: Date) -> String {
        formatter.string(from: date)
    }

    public static func now() -> String {
        string(from: Date())
    }

    /// Parses the subset of RFC 3339 this module emits: second precision,
    /// uppercase `T`/`Z`, and a numeric `hh:mm` offset (e.g.
    /// `2026-07-10T22:00:00+03:00` or `...Z`). Fractional seconds and
    /// lowercase `t`/`z` are outside that subset and return `nil`.
    public static func date(from string: String) -> Date? {
        formatter.date(from: string)
    }
}
