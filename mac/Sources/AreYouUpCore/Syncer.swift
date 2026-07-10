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

    private struct AckResponse: Decodable {
        let accepted: Int
    }

    /// Drains all unsynced samples batch by batch, then calls completion on
    /// the main queue with (samples synced this run, failure reason or nil).
    /// Batches synced before a failure stay marked synced.
    ///
    /// Not re-entrant; callers must not overlap calls (e.g. a timer must not
    /// fire again while a previous drain is still in flight). Completion may
    /// not fire at all if the Syncer is deallocated mid-drain, since the
    /// in-flight network callback captures self weakly.
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
        // Payloads are at most batchLimit tiny samples; 30s generously covers
        // a slow link without letting a stalled-but-trickling response wedge
        // the drain for days on URLSession's much longer default timeout.
        request.timeoutInterval = 30
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload = Payload(source: source,
                              samples: batch.map { .init(ts: $0.ts, idle_s: $0.idleS) })
        do {
            request.httpBody = try JSONEncoder().encode(payload)
        } catch {
            completion(alreadySynced, "encoding payload: \(error)")
            return
        }

        session.dataTask(with: request) { data, response, error in
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
                // A 200 alone isn't proof the real server saw this batch:
                // server_url is arbitrary user config, and a captive portal
                // or transparent proxy on plain http can answer 200 to
                // anything. Requiring the server's own accepted count to
                // match closes that gap - pruneSynced later deletes rows
                // permanently, so marking synced on a false positive would
                // be unrecoverable data loss.
                guard let data, let ack = try? JSONDecoder().decode(AckResponse.self, from: data) else {
                    completion(alreadySynced, "unreadable server ack")
                    return
                }
                guard ack.accepted == batch.count else {
                    completion(alreadySynced, "server ack mismatch: accepted \(ack.accepted) of \(batch.count)")
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
