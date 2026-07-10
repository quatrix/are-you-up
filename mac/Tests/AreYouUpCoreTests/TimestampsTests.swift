import XCTest
@testable import AreYouUpCore

final class TimestampsTests: XCTestCase {
    func testFormatIsRFC3339WithOffset() {
        let s = Timestamps.now()
        let pattern = #"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(Z|[+-]\d{2}:\d{2})$"#
        XCTAssertNotNil(s.range(of: pattern, options: .regularExpression), "got: \(s)")
    }

    func testRoundTripsToSecondPrecision() {
        let date = Date(timeIntervalSince1970: 1_783_000_000)
        let parsed = Timestamps.date(from: Timestamps.string(from: date))
        XCTAssertEqual(parsed?.timeIntervalSince1970, 1_783_000_000)
    }

    func testParsesForeignOffsets() {
        // Same instant, two representations: 22:00+03:00 is 19:00 UTC.
        // Equality (not just non-nil) proves the offset is interpreted.
        let withOffset = Timestamps.date(from: "2026-07-10T22:00:00+03:00")
        let utc = Timestamps.date(from: "2026-07-10T19:00:00Z")
        XCTAssertNotNil(withOffset)
        XCTAssertNotNil(utc)
        XCTAssertEqual(withOffset, utc)
    }
}
