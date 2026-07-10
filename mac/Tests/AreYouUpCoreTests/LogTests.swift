import XCTest
@testable import AreYouUpCore

final class LogTests: XCTestCase {
    private var path: String!

    override func setUp() {
        path = FileManager.default.temporaryDirectory
            .appendingPathComponent("log-\(UUID().uuidString).log").path
    }

    override func tearDown() {
        try? FileManager.default.removeItem(atPath: path)
    }

    func testAppendsTailFriendlyLinesAndHidesDebugByDefault() throws {
        let log = Log(path: path)
        log.info("first")
        log.error("second")
        log.debug("hidden by default")
        let lines = try String(contentsOfFile: path, encoding: .utf8)
            .split(separator: "\n")
        XCTAssertEqual(lines.count, 2)
        XCTAssertTrue(lines[0].contains("[INFO] first"), "got: \(lines[0])")
        XCTAssertTrue(lines[1].contains("[ERROR] second"), "got: \(lines[1])")
        XCTAssertNotNil(lines[0].range(of: #"^\d{4}-\d{2}-\d{2}T"#, options: .regularExpression),
                        "missing timestamp prefix, got: \(lines[0])")
    }

    func testDebugLinesAppearWhenEnabled() throws {
        let log = Log(path: path, debugEnabled: true)
        log.debug("visible")
        let content = try String(contentsOfFile: path, encoding: .utf8)
        XCTAssertTrue(content.contains("[DEBUG] visible"))
    }

    func testAppendsAcrossRelaunch() throws {
        Log(path: path).info("before relaunch")
        Log(path: path).info("after relaunch")
        let lines = try String(contentsOfFile: path, encoding: .utf8)
            .split(separator: "\n")
        XCTAssertEqual(lines.count, 2)
        XCTAssertTrue(lines[0].contains("before relaunch"), "got: \(lines[0])")
        XCTAssertTrue(lines[1].contains("after relaunch"), "got: \(lines[1])")
    }
}
