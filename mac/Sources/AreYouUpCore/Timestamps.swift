import Foundation

// ============================================================================
// RFC 3339 timestamps carrying the device's local UTC offset (ADR-0004),
// e.g. "2026-07-10T23:41:03+03:00". Used for samples, logs, and the API.
// ============================================================================

public enum Timestamps {
    // DateFormatter rather than ISO8601DateFormatter: the latter can only
    // emit Z-normalized UTC, and we want the local offset preserved.
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

    public static func date(from string: String) -> Date? {
        formatter.date(from: string)
    }
}
