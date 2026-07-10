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
            try encoder.encode(defaults).write(to: url, options: .atomic)
            return defaults
        }
        return try JSONDecoder().decode(Config.self, from: Data(contentsOf: url))
    }
}
