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
