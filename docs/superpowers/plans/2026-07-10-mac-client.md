# Mac Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Menu-bar app that samples seconds-since-last-input every 30s into
local sqlite and syncs batches to the backend every 60s.

**Architecture:** SwiftPM package in `mac/` with two targets: `AreYouUpCore`
(library: Store, Syncer, Config, Log, Timestamps, IdleTime - everything
testable) and `AreYouUp` (executable: AppKit glue - AppDelegate, status
item, history strip). Zero third-party dependencies; sqlite3, AppKit,
CoreGraphics and URLSession ship with macOS. Spec:
`docs/superpowers/specs/2026-07-10-are-you-up-design.md`.

**Tech Stack:** Swift 5.9+, SwiftPM, XCTest, SQLite3 C API, AppKit.

**Threading rule (applies everywhere):** `Store` is main-thread-only. Timers
run on the main run loop; `Syncer` marshals its URLSession callbacks back to
the main queue before touching the store.

**Conventions that apply to every commit in this plan:** semantic commit
titles, no co-author lines, prose wrapped, single-line commands.

## File structure

```
mac/
  Package.swift
  Sources/AreYouUpCore/
    Timestamps.swift     RFC 3339 local-offset formatting/parsing
    Store.swift          sqlite buffer: insert, unsynced, markSynced, prune, window
    Config.swift         config.json load-or-create-with-defaults
    Log.swift            append-only tail-friendly text log
    Syncer.swift         batch POST /v1/samples, mark synced on 200
    IdleTime.swift       CGEventSource seconds-since-any-input
  Sources/AreYouUp/
    main.swift           NSApplication bootstrap (accessory policy)
    AppDelegate.swift    wiring: config, store, syncer, timers, pause
    StatusItemController.swift  NSStatusItem + menu
    HistoryStripView.swift      6h activity strip drawing
  Tests/AreYouUpCoreTests/
    TimestampsTests.swift
    StoreTests.swift
    ConfigTests.swift
    LogTests.swift
    SyncerTests.swift
  Makefile               build / test / install / uninstall
  launchd/com.are-you-up.mac.plist   LaunchAgent template
  README.md
```

---

### Task 1: Package scaffold + Timestamps

**Files:**
- Create: `mac/Package.swift`
- Create: `mac/Sources/AreYouUpCore/Timestamps.swift`
- Create: `mac/Sources/AreYouUp/main.swift` (placeholder so the package builds)
- Test: `mac/Tests/AreYouUpCoreTests/TimestampsTests.swift`

- [ ] **Step 1: Create the package manifest and placeholder main**

Create `mac/Package.swift`:

```swift
// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "are-you-up-mac",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "are-you-up", targets: ["AreYouUp"]),
    ],
    targets: [
        .target(name: "AreYouUpCore"),
        .executableTarget(name: "AreYouUp", dependencies: ["AreYouUpCore"]),
        .testTarget(name: "AreYouUpCoreTests", dependencies: ["AreYouUpCore"]),
    ]
)
```

Create `mac/Sources/AreYouUp/main.swift` (replaced in Task 6):

```swift
// Placeholder so the package builds before the app glue exists (Task 6).
print("are-you-up mac client: app glue not built yet")
```

- [ ] **Step 2: Write the failing tests**

Create `mac/Tests/AreYouUpCoreTests/TimestampsTests.swift`:

```swift
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
        XCTAssertNotNil(Timestamps.date(from: "2026-07-10T22:00:00+03:00"))
        XCTAssertNotNil(Timestamps.date(from: "2026-07-10T19:00:00Z"))
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd mac && swift test`
Expected: FAIL to compile, `cannot find 'Timestamps' in scope`.

- [ ] **Step 4: Implement Timestamps**

Create `mac/Sources/AreYouUpCore/Timestamps.swift`:

```swift
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd mac && swift test`
Expected: PASS, 3 tests, zero failures.

- [ ] **Step 6: Commit**

```bash
git add mac
git commit -m "feat(mac): SwiftPM scaffold and RFC 3339 local-offset timestamps"
```

---

### Task 2: Store (sqlite sample buffer)

**Files:**
- Create: `mac/Sources/AreYouUpCore/Store.swift`
- Test: `mac/Tests/AreYouUpCoreTests/StoreTests.swift`

- [ ] **Step 1: Write the failing tests**

Create `mac/Tests/AreYouUpCoreTests/StoreTests.swift`:

```swift
import XCTest
@testable import AreYouUpCore

final class StoreTests: XCTestCase {
    private var path: String!
    private var store: Store!

    override func setUpWithError() throws {
        path = FileManager.default.temporaryDirectory
            .appendingPathComponent("store-\(UUID().uuidString).db").path
        store = try Store(path: path)
    }

    override func tearDown() {
        store = nil
        try? FileManager.default.removeItem(atPath: path)
    }

    func testInsertedSamplesComeBackUnsyncedInOrder() throws {
        try store.insert(Sample(ts: "2026-07-10T22:00:30+03:00", idleS: 2))
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 1))
        XCTAssertEqual(try store.unsynced(limit: 10), [
            Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 1),
            Sample(ts: "2026-07-10T22:00:30+03:00", idleS: 2),
        ])
    }

    func testUnsyncedRespectsLimit() throws {
        for i in 0..<3 {
            try store.insert(Sample(ts: "2026-07-10T22:0\(i):00+03:00", idleS: i))
        }
        XCTAssertEqual(try store.unsynced(limit: 2).count, 2)
    }

    func testMarkSyncedRemovesFromUnsynced() throws {
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 1))
        try store.insert(Sample(ts: "2026-07-10T22:00:30+03:00", idleS: 2))
        try store.markSynced(["2026-07-10T22:00:00+03:00"])
        XCTAssertEqual(try store.unsynced(limit: 10), [Sample(ts: "2026-07-10T22:00:30+03:00", idleS: 2)])
    }

    func testInsertSameTimestampReplaces() throws {
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 1))
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 5))
        XCTAssertEqual(try store.unsynced(limit: 10), [Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 5)])
    }

    func testPruneDeletesOnlyOldSyncedRows() throws {
        try store.insert(Sample(ts: "2026-07-01T10:00:00+03:00", idleS: 1)) // old, will be synced
        try store.insert(Sample(ts: "2026-07-01T10:00:30+03:00", idleS: 2)) // old, stays unsynced
        try store.insert(Sample(ts: "2026-07-10T10:00:00+03:00", idleS: 3)) // recent, will be synced
        try store.markSynced(["2026-07-01T10:00:00+03:00", "2026-07-10T10:00:00+03:00"])
        try store.pruneSynced(before: "2026-07-03T00:00:00+03:00")
        XCTAssertEqual(try store.samples(since: "2026-01-01T00:00:00+03:00"), [
            Sample(ts: "2026-07-01T10:00:30+03:00", idleS: 2),
            Sample(ts: "2026-07-10T10:00:00+03:00", idleS: 3),
        ])
    }

    func testSamplesSinceReturnsWindow() throws {
        try store.insert(Sample(ts: "2026-07-10T21:00:00+03:00", idleS: 1))
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 2))
        XCTAssertEqual(try store.samples(since: "2026-07-10T21:30:00+03:00"),
                       [Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 2)])
    }

    func testPersistsAcrossReopen() throws {
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 1))
        store = nil
        store = try Store(path: path)
        XCTAssertEqual(try store.unsynced(limit: 10).count, 1)
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd mac && swift test`
Expected: FAIL to compile, `cannot find 'Store' in scope`.

- [ ] **Step 3: Implement Store**

Create `mac/Sources/AreYouUpCore/Store.swift`:

```swift
import Foundation
import SQLite3

// sqlite3_bind_text needs this destructor constant so sqlite copies the
// string before the Swift buffer goes away.
private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

public struct Sample: Equatable {
    public let ts: String
    public let idleS: Int

    public init(ts: String, idleS: Int) {
        self.ts = ts
        self.idleS = idleS
    }
}

public enum StoreError: Error {
    case sqlite(String)
}

/// Local sample buffer. Main-thread-only by design (see plan threading rule);
/// callers marshal to the main queue.
public final class Store {
    private var db: OpaquePointer?

    public init(path: String) throws {
        let dir = (path as NSString).deletingLastPathComponent
        if !dir.isEmpty {
            try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        }
        guard sqlite3_open(path, &db) == SQLITE_OK else {
            throw StoreError.sqlite(lastMessage())
        }
        try exec("PRAGMA journal_mode=WAL")
        try exec("""
            CREATE TABLE IF NOT EXISTS samples (
                ts TEXT PRIMARY KEY,
                idle_s INTEGER NOT NULL,
                synced INTEGER NOT NULL DEFAULT 0
            )
            """)
    }

    deinit {
        sqlite3_close(db)
    }

    public func insert(_ sample: Sample) throws {
        let stmt = try prepare("INSERT OR REPLACE INTO samples (ts, idle_s, synced) VALUES (?, ?, 0)")
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_text(stmt, 1, sample.ts, -1, SQLITE_TRANSIENT)
        sqlite3_bind_int64(stmt, 2, Int64(sample.idleS))
        guard sqlite3_step(stmt) == SQLITE_DONE else { throw StoreError.sqlite(lastMessage()) }
    }

    /// Oldest-first so the server receives samples in time order.
    public func unsynced(limit: Int) throws -> [Sample] {
        let stmt = try prepare("SELECT ts, idle_s FROM samples WHERE synced = 0 ORDER BY ts LIMIT ?")
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_int64(stmt, 1, Int64(limit))
        return try rows(stmt)
    }

    public func markSynced(_ timestamps: [String]) throws {
        try exec("BEGIN")
        do {
            let stmt = try prepare("UPDATE samples SET synced = 1 WHERE ts = ?")
            defer { sqlite3_finalize(stmt) }
            for ts in timestamps {
                sqlite3_reset(stmt)
                sqlite3_clear_bindings(stmt)
                sqlite3_bind_text(stmt, 1, ts, -1, SQLITE_TRANSIENT)
                guard sqlite3_step(stmt) == SQLITE_DONE else { throw StoreError.sqlite(lastMessage()) }
            }
            try exec("COMMIT")
        } catch {
            try? exec("ROLLBACK")
            throw error
        }
    }

    // ponytail: TEXT-ordering comparisons below assume a stable UTC offset
    // per device. Pruning and the 6h strip are housekeeping/display, so a
    // DST-window inaccuracy is harmless. Parse-and-compare if it ever isn't.

    /// Deletes synced rows with ts before the cutoff. Unsynced rows are
    /// never pruned: they are data the server has not seen yet.
    public func pruneSynced(before ts: String) throws {
        let stmt = try prepare("DELETE FROM samples WHERE synced = 1 AND ts < ?")
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_text(stmt, 1, ts, -1, SQLITE_TRANSIENT)
        guard sqlite3_step(stmt) == SQLITE_DONE else { throw StoreError.sqlite(lastMessage()) }
    }

    /// All samples (synced or not) at or after the cutoff, for the 6h strip.
    public func samples(since ts: String) throws -> [Sample] {
        let stmt = try prepare("SELECT ts, idle_s FROM samples WHERE ts >= ? ORDER BY ts")
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_text(stmt, 1, ts, -1, SQLITE_TRANSIENT)
        return try rows(stmt)
    }

    // MARK: - sqlite plumbing

    private func prepare(_ sql: String) throws -> OpaquePointer? {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            throw StoreError.sqlite(lastMessage())
        }
        return stmt
    }

    private func exec(_ sql: String) throws {
        guard sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK else {
            throw StoreError.sqlite(lastMessage())
        }
    }

    private func rows(_ stmt: OpaquePointer?) throws -> [Sample] {
        var result: [Sample] = []
        while true {
            switch sqlite3_step(stmt) {
            case SQLITE_ROW:
                let ts = String(cString: sqlite3_column_text(stmt, 0))
                result.append(Sample(ts: ts, idleS: Int(sqlite3_column_int64(stmt, 1))))
            case SQLITE_DONE:
                return result
            default:
                throw StoreError.sqlite(lastMessage())
            }
        }
    }

    private func lastMessage() -> String {
        String(cString: sqlite3_errmsg(db))
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd mac && swift test`
Expected: PASS, all tests (3 Timestamps + 7 Store), zero failures.

- [ ] **Step 5: Commit**

```bash
git add mac/Sources/AreYouUpCore/Store.swift mac/Tests/AreYouUpCoreTests/StoreTests.swift
git commit -m "feat(mac): sqlite sample buffer with sync tracking and pruning"
```

---

### Task 3: Config

**Files:**
- Create: `mac/Sources/AreYouUpCore/Config.swift`
- Test: `mac/Tests/AreYouUpCoreTests/ConfigTests.swift`

- [ ] **Step 1: Write the failing tests**

Create `mac/Tests/AreYouUpCoreTests/ConfigTests.swift`:

```swift
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd mac && swift test`
Expected: FAIL to compile, `cannot find 'Config' in scope`.

- [ ] **Step 3: Implement Config**

Create `mac/Sources/AreYouUpCore/Config.swift`:

```swift
import Foundation

public struct Config: Codable, Equatable {
    public var serverURL: String
    public var source: String

    enum CodingKeys: String, CodingKey {
        case serverURL = "server_url"
        case source
    }

    public init(serverURL: String, source: String) {
        self.serverURL = serverURL
        self.source = source
    }

    public static let defaults = Config(serverURL: "http://127.0.0.1:8080", source: "macbook")

    /// Loads the config, writing the defaults on first run so the file is
    /// discoverable and editable. A malformed file throws rather than being
    /// silently replaced: the user probably made a typo they want to fix.
    public static func load(path: String) throws -> Config {
        let url = URL(fileURLWithPath: path)
        if !FileManager.default.fileExists(atPath: path) {
            let dir = (path as NSString).deletingLastPathComponent
            if !dir.isEmpty {
                try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(defaults).write(to: url)
            return defaults
        }
        return try JSONDecoder().decode(Config.self, from: Data(contentsOf: url))
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd mac && swift test`
Expected: PASS, zero failures.

- [ ] **Step 5: Commit**

```bash
git add mac/Sources/AreYouUpCore/Config.swift mac/Tests/AreYouUpCoreTests/ConfigTests.swift
git commit -m "feat(mac): config.json with write-defaults-on-first-run"
```

---

### Task 4: Log

**Files:**
- Create: `mac/Sources/AreYouUpCore/Log.swift`
- Test: `mac/Tests/AreYouUpCoreTests/LogTests.swift`

- [ ] **Step 1: Write the failing tests**

Create `mac/Tests/AreYouUpCoreTests/LogTests.swift`:

```swift
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
    }

    func testDebugLinesAppearWhenEnabled() throws {
        let log = Log(path: path, debugEnabled: true)
        log.debug("visible")
        let content = try String(contentsOfFile: path, encoding: .utf8)
        XCTAssertTrue(content.contains("[DEBUG] visible"))
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd mac && swift test`
Expected: FAIL to compile, `cannot find 'Log' in scope`.

- [ ] **Step 3: Implement Log**

Create `mac/Sources/AreYouUpCore/Log.swift`:

```swift
import Foundation

/// Append-only text log made to be tailed:
///   2026-07-10T23:41:03+03:00 [INFO] synced 12 samples
/// Open-append-close per line: at a few lines per minute the cost is
/// nothing and the file handle can never go stale.
public final class Log {
    private let path: String
    private let debugEnabled: Bool

    public init(path: String, debugEnabled: Bool = false) {
        self.path = path
        self.debugEnabled = debugEnabled
        let dir = (path as NSString).deletingLastPathComponent
        if !dir.isEmpty {
            try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        }
    }

    public func info(_ message: String) { write("INFO", message) }
    public func error(_ message: String) { write("ERROR", message) }

    public func debug(_ message: String) {
        if debugEnabled { write("DEBUG", message) }
    }

    private func write(_ level: String, _ message: String) {
        let line = "\(Timestamps.now()) [\(level)] \(message)\n"
        guard let data = line.data(using: .utf8) else { return }
        if !FileManager.default.fileExists(atPath: path) {
            FileManager.default.createFile(atPath: path, contents: nil)
        }
        guard let handle = FileHandle(forWritingAtPath: path) else { return }
        defer { try? handle.close() }
        _ = try? handle.seekToEnd()
        try? handle.write(contentsOf: data)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd mac && swift test`
Expected: PASS, zero failures.

- [ ] **Step 5: Commit**

```bash
git add mac/Sources/AreYouUpCore/Log.swift mac/Tests/AreYouUpCoreTests/LogTests.swift
git commit -m "feat(mac): tail-friendly file logger"
```

---

### Task 5: Syncer

**Files:**
- Create: `mac/Sources/AreYouUpCore/Syncer.swift`
- Test: `mac/Tests/AreYouUpCoreTests/SyncerTests.swift`

- [ ] **Step 1: Write the failing tests**

Create `mac/Tests/AreYouUpCoreTests/SyncerTests.swift`:

```swift
import XCTest
@testable import AreYouUpCore

/// Intercepts every request on a stub URLSession; no network involved.
final class StubProtocol: URLProtocol {
    static var requests: [URLRequest] = []
    static var bodies: [Data] = []
    static var statusCode = 200

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        Self.requests.append(request)
        Self.bodies.append(Self.body(of: request))
        let response = HTTPURLResponse(url: request.url!, statusCode: Self.statusCode,
                                       httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("{}".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    /// URLProtocol exposes the body only as a stream; read it fully.
    private static func body(of request: URLRequest) -> Data {
        guard let stream = request.httpBodyStream else { return request.httpBody ?? Data() }
        stream.open()
        defer { stream.close() }
        var data = Data()
        let bufferSize = 4096
        let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: bufferSize)
        defer { buffer.deallocate() }
        while stream.hasBytesAvailable {
            let read = stream.read(buffer, maxLength: bufferSize)
            if read <= 0 { break }
            data.append(buffer, count: read)
        }
        return data
    }
}

final class SyncerTests: XCTestCase {
    private var store: Store!
    private var dbPath: String!

    override func setUpWithError() throws {
        dbPath = FileManager.default.temporaryDirectory
            .appendingPathComponent("syncer-\(UUID().uuidString).db").path
        store = try Store(path: dbPath)
        StubProtocol.requests = []
        StubProtocol.bodies = []
        StubProtocol.statusCode = 200
    }

    override func tearDown() {
        store = nil
        try? FileManager.default.removeItem(atPath: dbPath)
    }

    private func makeSyncer(batchLimit: Int = 1000) -> Syncer {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StubProtocol.self]
        return Syncer(store: store,
                      serverURL: URL(string: "http://example.test:8080")!,
                      source: "macbook",
                      session: URLSession(configuration: config),
                      batchLimit: batchLimit)
    }

    func testSuccessfulSyncMarksSamplesAndReportsCount() throws {
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 4))
        try store.insert(Sample(ts: "2026-07-10T22:00:30+03:00", idleS: 34))
        let syncer = makeSyncer()
        let done = expectation(description: "sync completes")
        syncer.syncOnce { synced, failure in
            XCTAssertNil(failure)
            XCTAssertEqual(synced, 2)
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
        XCTAssertEqual(try store.unsynced(limit: 10), [])
        XCTAssertNotNil(syncer.lastSuccess)
    }

    func testPayloadShape() throws {
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 4))
        let syncer = makeSyncer()
        let done = expectation(description: "sync completes")
        syncer.syncOnce { _, _ in done.fulfill() }
        wait(for: [done], timeout: 5)

        XCTAssertEqual(StubProtocol.requests.first?.url?.path, "/v1/samples")
        XCTAssertEqual(StubProtocol.requests.first?.value(forHTTPHeaderField: "Content-Type"),
                       "application/json")
        let json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: StubProtocol.bodies[0]) as? [String: Any])
        XCTAssertEqual(json["source"] as? String, "macbook")
        let samples = try XCTUnwrap(json["samples"] as? [[String: Any]])
        XCTAssertEqual(samples.count, 1)
        XCTAssertEqual(samples[0]["ts"] as? String, "2026-07-10T22:00:00+03:00")
        XCTAssertEqual(samples[0]["idle_s"] as? Int, 4)
    }

    func testDrainsInBatches() throws {
        for i in 0..<3 {
            try store.insert(Sample(ts: "2026-07-10T22:0\(i):00+03:00", idleS: i))
        }
        let syncer = makeSyncer(batchLimit: 2)
        let done = expectation(description: "sync completes")
        syncer.syncOnce { synced, failure in
            XCTAssertNil(failure)
            XCTAssertEqual(synced, 3)
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
        XCTAssertEqual(StubProtocol.requests.count, 2)
        XCTAssertEqual(try store.unsynced(limit: 10), [])
    }

    func testServerErrorKeepsSamplesUnsynced() throws {
        StubProtocol.statusCode = 500
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 4))
        let syncer = makeSyncer()
        let done = expectation(description: "sync completes")
        syncer.syncOnce { synced, failure in
            XCTAssertEqual(synced, 0)
            XCTAssertNotNil(failure)
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
        XCTAssertEqual(try store.unsynced(limit: 10).count, 1)
        XCTAssertNil(syncer.lastSuccess)
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd mac && swift test`
Expected: FAIL to compile, `cannot find 'Syncer' in scope`.

- [ ] **Step 3: Implement Syncer**

Create `mac/Sources/AreYouUpCore/Syncer.swift`:

```swift
import Foundation

/// Ships unsynced samples to the server in batches and marks them synced on
/// 200. Failures leave rows unsynced; the next timer tick retries. All
/// completions and store access are marshalled to the main queue (Store is
/// main-thread-only).
public final class Syncer {
    public private(set) var lastSuccess: Date?

    private let store: Store
    private let endpoint: URL
    private let source: String
    private let session: URLSession
    private let batchLimit: Int

    public init(store: Store, serverURL: URL, source: String,
                session: URLSession = .shared, batchLimit: Int = 1000) {
        self.store = store
        self.endpoint = serverURL.appendingPathComponent("v1/samples")
        self.source = source
        self.session = session
        self.batchLimit = batchLimit
    }

    private struct Payload: Encodable {
        struct Item: Encodable {
            let ts: String
            let idle_s: Int
        }
        let source: String
        let samples: [Item]
    }

    /// Drains all unsynced samples batch by batch, then calls completion on
    /// the main queue with (samples synced this run, failure reason or nil).
    /// Batches synced before a failure stay marked synced.
    public func syncOnce(completion: @escaping (_ synced: Int, _ failure: String?) -> Void) {
        dispatchPrecondition(condition: .onQueue(.main))
        step(alreadySynced: 0, completion: completion)
    }

    private func step(alreadySynced: Int, completion: @escaping (Int, String?) -> Void) {
        let batch: [Sample]
        do {
            batch = try store.unsynced(limit: batchLimit)
        } catch {
            completion(alreadySynced, "reading unsynced samples: \(error)")
            return
        }
        if batch.isEmpty {
            completion(alreadySynced, nil)
            return
        }

        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload = Payload(source: source,
                              samples: batch.map { .init(ts: $0.ts, idle_s: $0.idleS) })
        do {
            request.httpBody = try JSONEncoder().encode(payload)
        } catch {
            completion(alreadySynced, "encoding payload: \(error)")
            return
        }

        session.dataTask(with: request) { _, response, error in
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if let error {
                    completion(alreadySynced, "network: \(error.localizedDescription)")
                    return
                }
                let status = (response as? HTTPURLResponse)?.statusCode ?? -1
                guard status == 200 else {
                    completion(alreadySynced, "server returned status \(status)")
                    return
                }
                do {
                    try self.store.markSynced(batch.map(\.ts))
                } catch {
                    completion(alreadySynced, "marking synced: \(error)")
                    return
                }
                self.lastSuccess = Date()
                self.step(alreadySynced: alreadySynced + batch.count, completion: completion)
            }
        }.resume()
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd mac && swift test`
Expected: PASS, all core tests, zero failures.

- [ ] **Step 5: Commit**

```bash
git add mac/Sources/AreYouUpCore/Syncer.swift mac/Tests/AreYouUpCoreTests/SyncerTests.swift
git commit -m "feat(mac): batch syncer with mark-on-200 and retry-by-next-tick"
```

---

### Task 6: IdleTime + AppKit glue (status item, menu, history strip)

**Files:**
- Create: `mac/Sources/AreYouUpCore/IdleTime.swift`
- Replace: `mac/Sources/AreYouUp/main.swift`
- Create: `mac/Sources/AreYouUp/AppDelegate.swift`
- Create: `mac/Sources/AreYouUp/StatusItemController.swift`
- Create: `mac/Sources/AreYouUp/HistoryStripView.swift`

The AppKit layer is deliberately untested glue (per spec); correctness lives
in the core library. No TDD here; the verification is building and a manual
smoke run.

- [ ] **Step 1: Implement IdleTime**

Create `mac/Sources/AreYouUpCore/IdleTime.swift`:

```swift
import CoreGraphics

public enum IdleTime {
    /// Seconds since the last input event of ANY type (kCGAnyInputEventType).
    /// Needs no TCC/accessibility permission (verified on the target machine,
    /// LAB_NOTES.md 2026-07-10).
    public static func secondsSinceLastInput() -> Int {
        // kCGAnyInputEventType is ~0; the Swift enum initializer accepts it.
        // Fall back to mouseMoved if an SDK ever rejects the raw value.
        let anyInput = CGEventType(rawValue: ~UInt32(0)) ?? .mouseMoved
        let seconds = CGEventSource.secondsSinceLastEventType(.combinedSessionState,
                                                              eventType: anyInput)
        return max(0, Int(seconds.rounded()))
    }
}
```

- [ ] **Step 2: Implement the app delegate**

Create `mac/Sources/AreYouUp/AppDelegate.swift`:

```swift
import AppKit
import AreYouUpCore

final class AppDelegate: NSObject, NSApplicationDelegate {
    static let samplePeriod: TimeInterval = 30
    static let syncPeriod: TimeInterval = 60
    /// Display-only: the server owns real classification (ADR-0002). Matches
    /// the server's default threshold_s.
    static let displayThresholdS = 900

    private var log: Log!
    private var store: Store!
    private var syncer: Syncer!
    private var statusItem: StatusItemController!
    private var paused = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        // ARE_YOU_UP_HOME redirects all state for tests/E2E runs.
        let home = ProcessInfo.processInfo.environment["ARE_YOU_UP_HOME"] ?? NSHomeDirectory()
        let appSupport = home + "/Library/Application Support/are-you-up"
        log = Log(path: home + "/Library/Logs/are-you-up.log",
                  debugEnabled: ProcessInfo.processInfo.environment["ARE_YOU_UP_DEBUG"] == "1")
        do {
            let config = try Config.load(path: appSupport + "/config.json")
            guard let serverURL = URL(string: config.serverURL) else {
                log.error("config server_url is not a URL: \(config.serverURL)")
                NSApp.terminate(nil)
                return
            }
            store = try Store(path: appSupport + "/client.db")
            syncer = Syncer(store: store, serverURL: serverURL, source: config.source)
        } catch {
            log.error("startup failed: \(error)")
            NSApp.terminate(nil)
            return
        }

        statusItem = StatusItemController(
            onTogglePause: { [weak self] in self?.togglePause() },
            onQuit: { NSApp.terminate(nil) }
        )
        log.info("started")

        Timer.scheduledTimer(withTimeInterval: Self.samplePeriod, repeats: true) { [weak self] _ in
            self?.sampleTick()
        }
        Timer.scheduledTimer(withTimeInterval: Self.syncPeriod, repeats: true) { [weak self] _ in
            self?.syncTick()
        }
        sampleTick()
        syncTick()
    }

    private func sampleTick() {
        guard !paused else { return }
        let idleS = IdleTime.secondsSinceLastInput()
        do {
            try store.insert(Sample(ts: Timestamps.now(), idleS: idleS))
            try store.pruneSynced(before: Timestamps.string(from: Date().addingTimeInterval(-7 * 86_400)))
        } catch {
            log.error("storing sample: \(error)")
        }
        log.debug("sample idle_s=\(idleS)")
        refreshStatus(idleS: idleS)
    }

    private func syncTick() {
        guard !paused else { return }
        syncer.syncOnce { [weak self] synced, failure in
            guard let self else { return }
            if let failure {
                self.log.info("sync failed after \(synced) samples, will retry: \(failure)")
            } else if synced > 0 {
                self.log.info("synced \(synced) samples")
            } else {
                self.log.debug("sync ok, nothing new")
            }
            self.refreshStatus(idleS: IdleTime.secondsSinceLastInput())
        }
    }

    private func togglePause() {
        paused.toggle()
        log.info(paused ? "paused" : "resumed")
        refreshStatus(idleS: IdleTime.secondsSinceLastInput())
    }

    private func refreshStatus(idleS: Int) {
        let since = Timestamps.string(from: Date().addingTimeInterval(-6 * 3600))
        let history = (try? store.samples(since: since)) ?? []
        statusItem.update(idleS: idleS, paused: paused,
                          lastSuccess: syncer.lastSuccess, history: history)
    }
}
```

- [ ] **Step 3: Implement the status item and history strip**

Create `mac/Sources/AreYouUp/StatusItemController.swift`:

```swift
import AppKit
import AreYouUpCore

/// Menu bar presence: icon reflecting state, menu with status, last sync,
/// 6h history strip, pause/resume, quit.
final class StatusItemController: NSObject {
    private let item: NSStatusItem
    private let stateLine = NSMenuItem(title: "starting...", action: nil, keyEquivalent: "")
    private let syncLine = NSMenuItem(title: "Last sync: never", action: nil, keyEquivalent: "")
    private let pauseItem: NSMenuItem
    private let historyView = HistoryStripView(frame: NSRect(x: 0, y: 0, width: 280, height: 40))
    private let onTogglePause: () -> Void
    private let onQuit: () -> Void

    init(onTogglePause: @escaping () -> Void, onQuit: @escaping () -> Void) {
        self.onTogglePause = onTogglePause
        self.onQuit = onQuit
        self.item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        self.pauseItem = NSMenuItem(title: "Pause",
                                    action: #selector(StatusItemController.togglePause),
                                    keyEquivalent: "")
        super.init()

        item.button?.title = "●"
        let menu = NSMenu()
        menu.autoenablesItems = false
        stateLine.isEnabled = false
        syncLine.isEnabled = false
        menu.addItem(stateLine)
        menu.addItem(syncLine)
        menu.addItem(.separator())
        let historyItem = NSMenuItem()
        historyItem.view = historyView
        menu.addItem(historyItem)
        menu.addItem(.separator())
        pauseItem.target = self
        menu.addItem(pauseItem)
        let quitItem = NSMenuItem(title: "Quit", action: #selector(StatusItemController.quit),
                                  keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(quitItem)
        item.menu = menu
    }

    func update(idleS: Int, paused: Bool, lastSuccess: Date?, history: [Sample]) {
        let active = idleS < AppDelegate.displayThresholdS
        item.button?.title = paused ? "⏸" : (active ? "●" : "○")
        stateLine.title = paused ? "Paused" : (active ? "Active (idle \(idleS)s)" : "Idle (\(idleS)s)")
        syncLine.title = "Last sync: " + (lastSuccess.map(relative) ?? "never")
        pauseItem.title = paused ? "Resume" : "Pause"
        historyView.samples = history
    }

    private func relative(_ date: Date) -> String {
        let s = Int(-date.timeIntervalSinceNow)
        if s < 60 { return "\(s)s ago" }
        if s < 3600 { return "\(s / 60)m ago" }
        return "\(s / 3600)h \(s % 3600 / 60)m ago"
    }

    @objc private func togglePause() { onTogglePause() }
    @objc private func quit() { onQuit() }
}
```

Create `mac/Sources/AreYouUp/HistoryStripView.swift`:

```swift
import AppKit
import AreYouUpCore

/// Draws the last 6 hours as colored segments: green = active, gray = idle,
/// background showing through = no data (asleep, off, or paused).
final class HistoryStripView: NSView {
    var samples: [Sample] = [] {
        didSet { needsDisplay = true }
    }

    private let window6h: TimeInterval = 6 * 3600
    private let samplePeriod: TimeInterval = AppDelegate.samplePeriod

    override func draw(_ dirtyRect: NSRect) {
        let inset = bounds.insetBy(dx: 12, dy: 12)
        NSColor.quaternaryLabelColor.setFill()
        NSBezierPath(roundedRect: inset, xRadius: 3, yRadius: 3).fill()

        let end = Date()
        let start = end.addingTimeInterval(-window6h)
        for sample in samples {
            guard let t = Timestamps.date(from: sample.ts), t >= start, t <= end else { continue }
            let x = inset.minX + CGFloat(t.timeIntervalSince(start) / window6h) * inset.width
            let width = max(1, CGFloat(samplePeriod / window6h) * inset.width)
            let active = sample.idleS < AppDelegate.displayThresholdS
            (active ? NSColor.systemGreen : NSColor.systemGray).setFill()
            NSRect(x: x, y: inset.minY, width: width, height: inset.height).fill()
        }
    }
}
```

- [ ] **Step 4: Replace main.swift**

Replace `mac/Sources/AreYouUp/main.swift`:

```swift
import AppKit

// Accessory: menu bar only, no dock icon, no main window.
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
```

- [ ] **Step 5: Build and run the tests**

Run: `cd mac && swift build && swift test`
Expected: builds cleanly, all tests PASS.

- [ ] **Step 6: Manual smoke run**

Run the app pointing all its state at a scratch dir (avoids touching real
`~/Library`; if the sandbox denies anything, prefer widening
`ARE_YOU_UP_HOME` to a permitted path over disabling the sandbox):

```bash
cd mac && mkdir -p tmp/smoke-home && swift build
ARE_YOU_UP_HOME="$PWD/tmp/smoke-home" ./.build/debug/are-you-up & APP_PID=$!
sleep 65
sqlite3 "tmp/smoke-home/Library/Application Support/are-you-up/client.db" 'SELECT ts, idle_s, synced FROM samples ORDER BY ts'
tail -5 tmp/smoke-home/Library/Logs/are-you-up.log
kill "$APP_PID"
```

(Run the binary directly rather than `swift run` so the PID you kill is the
app itself, not a wrapper.)

Expected: a status icon appears in the menu bar; sqlite shows 2-3 samples
(synced=0 since no server is running); the log shows `started` and
`sync failed ... will retry` lines. Clicking the icon (if you are at the
machine) shows the menu with status, last sync `never`, the strip, Pause,
Quit.

- [ ] **Step 7: Commit**

```bash
git add mac/Sources
git commit -m "feat(mac): menu bar app wiring sampler, syncer, and 6h history strip"
```

---

### Task 7: Makefile, LaunchAgent, README

**Files:**
- Create: `mac/Makefile`
- Create: `mac/launchd/com.are-you-up.mac.plist`
- Create: `mac/README.md`

- [ ] **Step 1: Write the LaunchAgent template**

Create `mac/launchd/com.are-you-up.mac.plist` (`@BIN@` is substituted by
`make install`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.are-you-up.mac</string>
    <key>ProgramArguments</key>
    <array>
        <string>@BIN@</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
</dict>
</plist>
```

- [ ] **Step 2: Write the Makefile**

Create `mac/Makefile` (recipes must be tab-indented):

```make
PREFIX ?= $(HOME)/.local
BIN = $(PREFIX)/bin/are-you-up
PLIST = $(HOME)/Library/LaunchAgents/com.are-you-up.mac.plist

.PHONY: build test install uninstall

build:
	swift build -c release

test:
	swift test

install: build
	mkdir -p $(PREFIX)/bin
	cp .build/release/are-you-up $(BIN)
	mkdir -p $(HOME)/Library/LaunchAgents
	sed "s|@BIN@|$(BIN)|" launchd/com.are-you-up.mac.plist > $(PLIST)
	launchctl bootout gui/$$(id -u) $(PLIST) 2>/dev/null || true
	launchctl bootstrap gui/$$(id -u) $(PLIST)

uninstall:
	launchctl bootout gui/$$(id -u) $(PLIST) 2>/dev/null || true
	rm -f $(PLIST) $(BIN)
```

- [ ] **Step 3: Verify build and test targets**

Run: `cd mac && make build && make test`
Expected: release build succeeds, tests pass. Do NOT run `make install`
here; installing the LaunchAgent is the user's call.

- [ ] **Step 4: Write mac/README.md**

Create `mac/README.md`:

```markdown
# are-you-up mac client

Menu bar app that records seconds-since-last-input every 30s into local
sqlite and syncs to the backend every 60s. See
`../docs/superpowers/specs/` for the design.

## Paths

- Config:  `~/Library/Application Support/are-you-up/config.json`
  (`{"server_url": "...", "source": "macbook"}`, created on first run)
- Data:    `~/Library/Application Support/are-you-up/client.db`
- Log:     `~/Library/Logs/are-you-up.log` (tail it; set
  `ARE_YOU_UP_DEBUG=1` for per-sample lines)
- `ARE_YOU_UP_HOME` env var redirects all of the above (used by tests).

## Run / install

    swift run are-you-up      # foreground run
    make install              # release build + LaunchAgent (starts at login)
    make uninstall

## Menu

Icon: ● active, ○ idle (no input for 15 min), ⏸ paused. The menu shows
current status, last successful sync, a 6-hour history strip (green
active / gray idle / empty no-data), Pause/Resume, and Quit. Pausing
stops sampling entirely; paused time uploads nothing.

## Test

    swift test
```

- [ ] **Step 5: Commit**

```bash
git add mac/Makefile mac/launchd mac/README.md
git commit -m "feat(mac): Makefile and LaunchAgent install for background running"
```

---

### Task 8: Joint E2E against the real backend (run only if `backend/` is built)

**Files:** none (verification only)

Skip this task with a note if the backend plan has not landed yet; it is
the only cross-component step.

- [ ] **Step 1: Start the backend and a pointed client**

All commands run from the repo root; each is standalone:

```bash
(cd backend && cargo build --quiet && ARE_YOU_UP_ADDR=127.0.0.1:18080 ARE_YOU_UP_DB="$(mktemp -d)/e2e.db" ./target/debug/are-you-up-backend) &
mkdir -p "mac/tmp/e2e-home/Library/Application Support/are-you-up"
printf '{"server_url": "http://127.0.0.1:18080", "source": "e2e-mac"}' > "mac/tmp/e2e-home/Library/Application Support/are-you-up/config.json"
(cd mac && swift build && ARE_YOU_UP_HOME="$PWD/tmp/e2e-home" ./.build/debug/are-you-up) &
```

- [ ] **Step 2: Wait ~70s, then query the server for derived intervals**

```bash
sleep 70
python3 -c "from datetime import datetime, timedelta, timezone; from urllib.parse import quote; now = datetime.now(timezone.utc).astimezone(); print('http://127.0.0.1:18080/v1/intervals?from=' + quote((now - timedelta(minutes=10)).isoformat(timespec='seconds')) + '&to=' + quote((now + timedelta(minutes=10)).isoformat(timespec='seconds')) + '&source=e2e-mac')" | xargs curl -sf
```

Expected: JSON with at least one interval for `e2e-mac` (state depends on
whether the machine saw input in the last 15 minutes). The client log in
`tmp/e2e-home/Library/Logs/are-you-up.log` shows `synced N samples`.

- [ ] **Step 3: Clean up**

```bash
pkill -f are-you-up-backend || true
pkill -f '.build/debug/are-you-up' || true
rm -rf mac/tmp/e2e-home
```

No commit; this task changes no files.

---

## Plan self-review notes

- Spec coverage: 30s sampling + rounding (Task 6 AppDelegate), sqlite WAL
  buffer with synced flag and 7-day prune (Tasks 2, 6), 60s batch sync of
  <=1000 with mark-on-200 and dumb retry (Task 5), pause = stopped timer =
  gap (Task 6), menu with status/last-sync/6h strip/pause/quit (Task 6),
  tail-friendly log with debug ticks (Task 4), config.json with defaults
  (Task 3), LaunchAgent + make install (Task 7), no third-party deps
  (Package.swift has none).
- Threading rule stated once in the header and enforced in Syncer via
  dispatchPrecondition + main-queue marshalling.
- Type consistency: `Sample(ts:idleS:)`, `Store.unsynced(limit:)`,
  `markSynced(_:)`, `pruneSynced(before:)`, `samples(since:)`,
  `Syncer.syncOnce(completion:)` with `(Int, String?)`,
  `Config.load(path:)`, `Timestamps.string(from:)/date(from:)/now()` are
  used with these exact spellings in every task.
