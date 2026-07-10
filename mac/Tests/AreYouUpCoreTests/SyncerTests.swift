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
