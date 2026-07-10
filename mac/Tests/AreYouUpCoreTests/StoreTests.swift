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
