import XCTest
@testable import AreYouUpCore

final class ConfigTests: XCTestCase {
    private var dir: String!

    override func setUp() {
        dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("config-\(UUID().uuidString)").path
    }

    override func tearDown() {
        try? FileManager.default.removeItem(atPath: dir)
    }

    func testFirstRunWritesDefaultsAndReturnsThem() throws {
        let path = dir + "/config.json"
        let config = try Config.load(path: path)
        XCTAssertEqual(config, Config.defaults)
        let onDisk = try JSONSerialization.jsonObject(
            with: Data(contentsOf: URL(fileURLWithPath: path))) as? [String: Any]
        XCTAssertEqual(onDisk?["server_url"] as? String, "http://127.0.0.1:8080")
        XCTAssertEqual(onDisk?["source"] as? String, "macbook")
    }

    func testExistingFileLoads() throws {
        let path = dir + "/config.json"
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let json = #"{"server_url": "http://ts-box:9999", "source": "mbp16"}"#
        try json.write(toFile: path, atomically: true, encoding: .utf8)
        let config = try Config.load(path: path)
        XCTAssertEqual(config, Config(serverURL: "http://ts-box:9999", source: "mbp16"))
    }

    func testMalformedFileThrows() throws {
        let path = dir + "/config.json"
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        try "not json".write(toFile: path, atomically: true, encoding: .utf8)
        XCTAssertThrowsError(try Config.load(path: path))
    }
}
