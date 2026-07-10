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
