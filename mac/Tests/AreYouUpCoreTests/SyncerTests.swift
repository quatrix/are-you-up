import XCTest
@testable import AreYouUpCore

/// Intercepts every request on a stub URLSession; no network involved.
final class StubProtocol: URLProtocol {
    static var requests: [URLRequest] = []
    static var bodies: [Data] = []
    static var statusCode = 200
    /// Consumed one per request (oldest first); falls back to `statusCode`
    /// once exhausted, so a sequence like [200, 500] can be scripted.
    static var statusCodes: [Int] = []
    /// Overrides the ack's "accepted" count; nil mirrors the real sample
    /// count in the request body, as a real server would.
    static var ackOverride: Int?
    /// When set, the request fails at the transport level instead of
    /// completing with a response (simulates a network error).
    static var failWithError: Error?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        Self.requests.append(request)
        let body = Self.body(of: request)
        Self.bodies.append(body)

        if let error = Self.failWithError {
            client?.urlProtocol(self, didFailWithError: error)
            return
        }

        let status = Self.statusCodes.isEmpty ? Self.statusCode : Self.statusCodes.removeFirst()
        let response = HTTPURLResponse(url: request.url!, statusCode: status,
                                       httpVersion: nil, headerFields: nil)!
        let accepted = Self.ackOverride ?? Self.sampleCount(in: body)
        let ackBody = Data(#"{"accepted": \#(accepted)}"#.utf8)
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: ackBody)
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

    /// Mirrors what a real server would ack: the number of samples it saw.
    private static func sampleCount(in body: Data) -> Int {
        guard let json = try? JSONSerialization.jsonObject(with: body) as? [String: Any],
              let samples = json["samples"] as? [[String: Any]] else { return 0 }
        return samples.count
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
        StubProtocol.statusCodes = []
        StubProtocol.ackOverride = nil
        StubProtocol.failWithError = nil
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

    /// The doc comment's core promise: a mid-drain failure keeps everything
    /// synced so far, and only what wasn't sent yet stays unsynced.
    func testPartialDrainOnMidBatchFailure() throws {
        for i in 0..<3 {
            try store.insert(Sample(ts: "2026-07-10T22:0\(i):00+03:00", idleS: i))
        }
        StubProtocol.statusCodes = [200, 500]
        let syncer = makeSyncer(batchLimit: 2)
        let done = expectation(description: "sync completes")
        syncer.syncOnce { synced, failure in
            XCTAssertEqual(synced, 2)
            XCTAssertNotNil(failure)
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
        XCTAssertEqual(try store.unsynced(limit: 10),
                       [Sample(ts: "2026-07-10T22:02:00+03:00", idleS: 2)])
    }

    func testNetworkErrorLeavesSampleUnsynced() throws {
        struct StubNetworkError: Error, LocalizedError {
            var errorDescription: String? { "stub network failure" }
        }
        StubProtocol.failWithError = StubNetworkError()
        try store.insert(Sample(ts: "2026-07-10T22:00:00+03:00", idleS: 4))
        let syncer = makeSyncer()
        let done = expectation(description: "sync completes")
        syncer.syncOnce { synced, failure in
            XCTAssertEqual(synced, 0)
            XCTAssertNotNil(failure)
            XCTAssertTrue(failure?.contains("network") ?? false, "got: \(failure ?? "nil")")
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
        XCTAssertEqual(try store.unsynced(limit: 10).count, 1)
        XCTAssertNil(syncer.lastSuccess)
    }

    func testAckMismatchLeavesSampleUnsynced() throws {
        StubProtocol.ackOverride = 0
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
