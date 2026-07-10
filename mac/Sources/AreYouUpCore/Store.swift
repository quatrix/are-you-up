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
                ts TEXT PRIMARY KEY NOT NULL,
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
        try check(sqlite3_bind_text(stmt, 1, sample.ts, -1, SQLITE_TRANSIENT))
        try check(sqlite3_bind_int64(stmt, 2, Int64(sample.idleS)))
        guard sqlite3_step(stmt) == SQLITE_DONE else { throw StoreError.sqlite(lastMessage()) }
    }

    // ponytail: TEXT-ordering comparisons across unsynced(), pruneSynced(),
    // and samples(since:) assume a stable UTC offset per device. All three
    // uses are housekeeping/display, not data correctness, so a DST-window
    // inaccuracy is harmless; parse-and-compare if that ever changes.

    /// Lexical (approximately time) order; the server orders by parsed
    /// timestamps anyway.
    public func unsynced(limit: Int) throws -> [Sample] {
        let stmt = try prepare("SELECT ts, idle_s FROM samples WHERE synced = 0 ORDER BY ts LIMIT ?")
        defer { sqlite3_finalize(stmt) }
        try check(sqlite3_bind_int64(stmt, 1, Int64(limit)))
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
                try check(sqlite3_bind_text(stmt, 1, ts, -1, SQLITE_TRANSIENT))
                guard sqlite3_step(stmt) == SQLITE_DONE else { throw StoreError.sqlite(lastMessage()) }
            }
            try exec("COMMIT")
        } catch {
            try? exec("ROLLBACK")
            throw error
        }
    }

    /// Deletes synced rows with ts before the cutoff. Unsynced rows are
    /// never pruned: they are data the server has not seen yet.
    public func pruneSynced(before ts: String) throws {
        let stmt = try prepare("DELETE FROM samples WHERE synced = 1 AND ts < ?")
        defer { sqlite3_finalize(stmt) }
        try check(sqlite3_bind_text(stmt, 1, ts, -1, SQLITE_TRANSIENT))
        guard sqlite3_step(stmt) == SQLITE_DONE else { throw StoreError.sqlite(lastMessage()) }
    }

    /// All samples (synced or not) at or after the cutoff, for the 6h strip.
    public func samples(since ts: String) throws -> [Sample] {
        let stmt = try prepare("SELECT ts, idle_s FROM samples WHERE ts >= ? ORDER BY ts")
        defer { sqlite3_finalize(stmt) }
        try check(sqlite3_bind_text(stmt, 1, ts, -1, SQLITE_TRANSIENT))
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

    /// A failed bind (e.g. OOM) must not be allowed to silently step with a
    /// NULL parameter - `ts TEXT PRIMARY KEY` still permits NULL in sqlite's
    /// rowid tables, and a NULL-ts row would poison every future `rows()`
    /// call (guarded against separately, but better to never write one).
    ///
    /// The message includes sqlite3_errstr(rc) alongside lastMessage():
    /// sqlite3_errmsg can still report stale "not an error" text for some
    /// bind failure modes, so the raw result code's own string is included
    /// too rather than relying on errmsg alone.
    private func check(_ rc: Int32) throws {
        guard rc == SQLITE_OK else {
            throw StoreError.sqlite("\(String(cString: sqlite3_errstr(rc))): \(lastMessage())")
        }
    }

    private func rows(_ stmt: OpaquePointer?) throws -> [Sample] {
        var result: [Sample] = []
        while true {
            switch sqlite3_step(stmt) {
            case SQLITE_ROW:
                guard let tsColumn = sqlite3_column_text(stmt, 0) else {
                    throw StoreError.sqlite("samples.ts column was NULL")
                }
                let ts = String(cString: tsColumn)
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
