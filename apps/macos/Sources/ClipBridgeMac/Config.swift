import Foundation

/// Application configuration matching the CLI format (~/.config/clipbridge/config.json).
struct AppConfig: Codable {
    var deviceId: String
    var deviceName: String
    var relayUrl: String
    var groupKey: String // base64url

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case deviceName = "device_name"
        case relayUrl = "relay_url"
        case groupKey = "group_key"
    }

    static var defaultDirectory: URL {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let dir = appSupport.appendingPathComponent("ClipBridge", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    static var configFileUrl: URL {
        defaultDirectory.appendingPathComponent("config.json")
    }

    static var historyDbUrl: URL {
        defaultDirectory.appendingPathComponent("history.db")
    }

    static func load() -> AppConfig? {
        // Also check if CLI config exists (~/.config/clipbridge/config.json)
        let homeDir = FileManager.default.homeDirectoryForCurrentUser
        let cliConfig = homeDir.appendingPathComponent(".config/clipbridge/config.json")
        let url = FileManager.default.fileExists(atPath: configFileUrl.path) ? configFileUrl :
                  (FileManager.default.fileExists(atPath: cliConfig.path) ? cliConfig : nil)

        guard let url = url, let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(AppConfig.self, from: data)
    }

    func save() throws {
        let data = try JSONEncoder().encode(self)
        try data.write(to: Self.configFileUrl, options: .atomic)
        // Set permissions 0600
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: Self.configFileUrl.path)
    }
}
