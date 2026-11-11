import Foundation
import AppKit
import Combine

enum ConnectionState: String {
    case disconnected = "Disconnected"
    case connecting = "Connecting..."
    case connected = "Connected"
    case paused = "Paused"

    var systemIcon: String {
        switch self {
        case .connected: return "paperclip.circle.fill"
        case .connecting: return "arrow.triangle.2.circlepath.circle"
        case .disconnected: return "exclamationmark.circle"
        case .paused: return "pause.circle.fill"
        }
    }
}

@MainActor
final class SyncEngine: ObservableObject {
    @Published var state: ConnectionState = .disconnected
    @Published var isPaused: Bool = false {
        didSet {
            if isPaused {
                state = .paused
                stopWatcher()
            } else {
                state = .connecting
                startWatcher()
            }
        }
    }
    @Published var recentItems: [FfiHistoryEntry] = []
    @Published var config: AppConfig?
    @Published var statusMessage: String = ""
    @Published var nearbyBlePeersCount: Int = 0

    private var groupKey: FfiGroupKey?
    private var history: FfiHistory?
    private let echoGuard = FfiEchoGuard()
    private var relayClient: FfiRelayClient?
    private var bleManager: BleSyncManager?

    private var pasteboardTimer: Timer?
    private var lastChangeCount: Int = 0
    private var syncTask: Task<Void, Never>?

    init() {
        setupHistory()
        setupBleManager()
        loadConfigOrCreate()
    }

    private func setupBleManager() {
        self.bleManager = BleSyncManager { [weak self] envelope in
            Task { @MainActor [weak self] in
                guard let self = self, let key = self.groupKey else { return }
                self.handleIncomingEnvelope(envelope, key: key, source: "BLE")
            }
        }
    }

    private func setupHistory() {
        let dbPath = AppConfig.historyDbUrl.path
        do {
            self.history = try FfiHistory.open(path: dbPath)
            refreshHistory()
        } catch {
            print("Failed to open history db at \(dbPath):", error)
            self.history = try? FfiHistory.openInMemory()
        }
    }

    func refreshHistory() {
        guard let history = history else { return }
        do {
            self.recentItems = try history.recent(limit: 30)
        } catch {
            print("Failed to load recent history:", error)
        }
    }

    func loadConfigOrCreate() {
        if let existing = AppConfig.load() {
            self.config = existing
            applyConfig(existing)
        } else {
            // Auto initialize a default group
            let key = FfiGroupKey.generate()
            let deviceId = ffiNewDeviceId()
            let deviceName = Host.current().localizedName ?? "Mac"
            let relay = "ws://127.0.0.1:8080"
            let newConfig = AppConfig(
                deviceId: deviceId,
                deviceName: deviceName,
                relayUrl: relay,
                groupKey: key.toBase64url()
            )
            try? newConfig.save()
            self.config = newConfig
            applyConfig(newConfig)
        }
    }

    func joinGroup(pairingUri: String) throws {
        let pairInfo = try FfiPairingInfo.fromUri(uri: pairingUri)
        let key = try pairInfo.groupKey()
        applyNewGroupConfig(key: key, relayUrl: pairInfo.relayUrl())
    }

    func createPairingCode() async throws -> String {
        guard let cfg = config, let key = groupKey else {
            throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "No active group key"])
        }
        let httpBase = cfg.relayUrl.replacingOccurrences(of: "wss://", with: "https://").replacingOccurrences(of: "ws://", with: "http://")
        guard let url = URL(string: "\(httpBase)/v1/pair/create") else {
            throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "Invalid relay URL"])
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload: [String: Any] = [
            "group_key": key.toBase64url(),
            "relay_url": cfg.relayUrl
        ]
        req.httpBody = try JSONSerialization.data(withJSONObject: payload)

        let (data, res) = try await URLSession.shared.data(for: req)
        guard let httpRes = res as? HTTPURLResponse, httpRes.statusCode == 200 else {
            throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "Failed to create pairing code"])
        }
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let code = json["code"] as? String else {
            throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "Invalid server response"])
        }
        return code
    }

    func joinWithCodeOrKey(_ input: String) async throws {
        let cleaned = input.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedCode = cleaned.replacingOccurrences(of: "-", with: "").replacingOccurrences(of: " ", with: "").uppercased()

        // 1. If 6-8 alphanumeric characters, claim from relay rendezvous
        if normalizedCode.count >= 6 && normalizedCode.count <= 8 && normalizedCode.range(of: "^[0-9A-Z]{6,8}$", options: .regularExpression) != nil {
            let httpBase = (config?.relayUrl ?? "wss://clipbridge-relay.vqtoan1807.workers.dev")
                .replacingOccurrences(of: "wss://", with: "https://")
                .replacingOccurrences(of: "ws://", with: "http://")
            guard let url = URL(string: "\(httpBase)/v1/pair/claim") else {
                throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "Invalid relay URL"])
            }
            var req = URLRequest(url: url)
            req.httpMethod = "POST"
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            let payload: [String: Any] = ["code": normalizedCode]
            req.httpBody = try JSONSerialization.data(withJSONObject: payload)

            let (data, res) = try await URLSession.shared.data(for: req)
            guard let httpRes = res as? HTTPURLResponse, httpRes.statusCode == 200 else {
                let errObj = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
                throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: errObj ?? "Invalid or expired pairing code"])
            }
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let groupKeyStr = json["group_key"] as? String,
                  let relayUrl = json["relay_url"] as? String else {
                throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "Malformed response from server"])
            }

            let key = try FfiGroupKey.fromBase64url(s: groupKeyStr)
            applyNewGroupConfig(key: key, relayUrl: relayUrl)
            return
        }

        // 2. If it is a full URI
        if cleaned.hasPrefix("clipbridge://") {
            try joinGroup(pairingUri: cleaned)
            return
        }

        // 3. If raw 43-char base64url group key
        if cleaned.count == 43 {
            let key = try FfiGroupKey.fromBase64url(s: cleaned)
            applyNewGroupConfig(key: key, relayUrl: config?.relayUrl ?? "wss://clipbridge-relay.vqtoan1807.workers.dev")
            return
        }

        throw NSError(domain: "ClipBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "Invalid pairing code, group key, or URI"])
    }

    func createNewGroup() {
        let key = FfiGroupKey.generate()
        applyNewGroupConfig(key: key, relayUrl: config?.relayUrl ?? "wss://clipbridge-relay.vqtoan1807.workers.dev")
    }

    private func applyNewGroupConfig(key: FfiGroupKey, relayUrl: String) {
        let deviceId = config?.deviceId ?? ffiNewDeviceId()
        let deviceName = config?.deviceName ?? (Host.current().localizedName ?? "Mac")
        let newConfig = AppConfig(
            deviceId: deviceId,
            deviceName: deviceName,
            relayUrl: relayUrl,
            groupKey: key.toBase64url()
        )
        try? newConfig.save()
        self.config = newConfig
        applyConfig(newConfig)
    }

    func pairingUri() -> String? {
        guard let cfg = config, let key = groupKey else { return nil }
        let pair = FfiPairingInfo(relayUrl: cfg.relayUrl, groupKey: key)
        return pair.toUri()
    }

    private func applyConfig(_ cfg: AppConfig) {
        do {
            let key = try FfiGroupKey.fromBase64url(s: cfg.groupKey)
            self.groupKey = key
            self.bleManager?.updateGroupKey(key)
            startSync()
        } catch {
            self.state = .disconnected
            self.statusMessage = "Invalid group key: \(error.localizedDescription)"
        }
    }

    func startSync() {
        syncTask?.cancel()
        guard !isPaused else { return }

        startWatcher()

        syncTask = Task {
            await runRelayLoop()
        }
    }

    func stopSync() {
        syncTask?.cancel()
        syncTask = nil
        stopWatcher()
        self.state = .disconnected
    }

    private func startWatcher() {
        lastChangeCount = NSPasteboard.general.changeCount
        pasteboardTimer?.invalidate()
        let timer = Timer(timeInterval: 0.2, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.pollPasteboard()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        pasteboardTimer = timer
    }

    private func stopWatcher() {
        pasteboardTimer?.invalidate()
        pasteboardTimer = nil
    }

    // MARK: - Local Pasteboard Polling
    private func pollPasteboard() {
        guard !isPaused, let groupKey = groupKey, let config = config else { return }
        let pb = NSPasteboard.general
        guard pb.changeCount != lastChangeCount else { return }
        lastChangeCount = pb.changeCount

        // 1. Text check
        if let string = pb.string(forType: .string), !string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            do {
                let payload = try FfiClipPayload.text(text: string)
                if echoGuard.shouldPublish(hash: payload.hash()) {
                    Task {
                        await publishPayload(payload, key: groupKey, sender: config.deviceId)
                    }
                }
            } catch {
                print("Failed to package text payload:", error)
            }
            return
        }

        // 2. Image check
        if let imgData = pb.data(forType: .png) ?? pb.data(forType: .tiff) {
            do {
                let normalized = try ffiNormalizeToPng(bytes: imgData)
                let payload = try FfiClipPayload.image(data: normalized)
                if echoGuard.shouldPublish(hash: payload.hash()) {
                    Task {
                        await publishPayload(payload, key: groupKey, sender: config.deviceId)
                    }
                }
            } catch {
                print("Failed to package image payload:", error)
            }
        }
    }

    private func publishPayload(_ payload: FfiClipPayload, key: FfiGroupKey, sender: String) async {
        do {
            let env = try FfiEnvelope.seal(key: key, sender: sender, payload: payload)

            // 1. Send via BLE to nearby peers immediately
            bleManager?.broadcastEnvelope(env)

            // 2. Also send via Relay WebSocket if connected
            if let client = relayClient {
                try? await client.publish(envelope: env)
            }

            // Save to local history
            let entry = FfiHistoryEntry(
                id: env.id(),
                origin: "local",
                kind: payload.kind(),
                mime: payload.mime(),
                hash: payload.hash(),
                size: UInt64(payload.data().count),
                createdAt: env.createdAt(),
                data: payload.data(),
                isRemote: false
            )
            try? history?.insert(entry: entry)
            refreshHistory()
        } catch {
            print("Failed to publish envelope:", error)
        }
    }

    // MARK: - Remote Relay Loop
    private func runRelayLoop() async {
        var backoffMs: UInt64 = 1000
        while !Task.isCancelled {
            guard let cfg = config, let key = groupKey else {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                continue
            }

            self.state = .connecting
            let since = try? history?.lastRemoteEnvelopeId()

            do {
                let client = try await FfiRelayClient.connect(
                    baseUrl: cfg.relayUrl,
                    deviceId: cfg.deviceId,
                    key: key,
                    since: since
                )
                self.relayClient = client
                self.state = .connected
                self.statusMessage = ""
                backoffMs = 1000 // Reset backoff on successful connect

                while !Task.isCancelled {
                    guard let event = try await client.nextEvent() else {
                        // Connection cleanly closed by server
                        break
                    }
                    handleRelayEvent(event, key: key)
                }
            } catch {
                print("Relay error: \(error.localizedDescription), reconnecting in \(backoffMs)ms")
                self.state = .disconnected
                self.statusMessage = error.localizedDescription
            }

            self.relayClient = nil
            try? await Task.sleep(nanoseconds: backoffMs * 1_000_000)
            backoffMs = min(backoffMs * 2, 30_000)
        }
    }

    private func handleRelayEvent(_ event: FfiRelayEvent, key: FfiGroupKey) {
        switch event {
        case .deliver(let env):
            handleIncomingEnvelope(env, key: key, source: "Relay")
        case .ack(let id):
            print("Server ACK:", id)
        case .error(let code, let msg, let id):
            print("Server Error: [\(code)] \(msg) id=\(id ?? "none")")
        }
    }

    func handleIncomingEnvelope(_ env: FfiEnvelope, key: FfiGroupKey, source: String) {
        if echoGuard.seenEnvelope(id: env.id()) { return }
        do {
            let payload = try env.open(key: key)
            echoGuard.markApplied(hash: payload.hash())

            // Write to system pasteboard
            let pb = NSPasteboard.general
            pb.clearContents()
            switch payload.kind() {
            case .text:
                if let text = payload.asText() {
                    pb.setString(text, forType: .string)
                }
            case .image:
                pb.setData(payload.data(), forType: .png)
            }
            lastChangeCount = pb.changeCount

            // Store in history
            let entry = FfiHistoryEntry(
                id: env.id(),
                origin: "\(env.sender()) (\(source))",
                kind: payload.kind(),
                mime: payload.mime(),
                hash: payload.hash(),
                size: UInt64(payload.data().count),
                createdAt: env.createdAt(),
                data: payload.data(),
                isRemote: true
            )
            try? history?.insert(entry: entry)
            refreshHistory()
        } catch {
            print("Failed to decrypt envelope from \(env.sender()) via \(source):", error)
        }
    }

    // MARK: - User Action: Restore Item to Pasteboard
    func copyToClipboard(_ entry: FfiHistoryEntry) {
        let pb = NSPasteboard.general
        pb.clearContents()
        echoGuard.markApplied(hash: entry.hash)
        switch entry.kind {
        case .text:
            if let str = String(data: entry.data, encoding: .utf8) {
                pb.setString(str, forType: .string)
            }
        case .image:
            pb.setData(entry.data, forType: .png)
        }
        lastChangeCount = pb.changeCount
    }
}
