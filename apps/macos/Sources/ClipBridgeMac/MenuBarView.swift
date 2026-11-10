import SwiftUI

struct MenuBarView: View {
    @ObservedObject var engine: SyncEngine
    @State private var showingJoinSheet = false
    @State private var showingSettingsSheet = false
    @State private var copiedPairingLink = false

    var body: some View {
        VStack(spacing: 0) {
            // Header: Status & Quick Controls
            headerSection
                .padding(.horizontal, 16)
                .padding(.vertical, 12)

            Divider()

            // Main Content: Recent Clips List
            clipsSection

            Divider()

            // Footer: Pairing & App Controls
            footerSection
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
        }
        .frame(width: 340, height: 480)
        .sheet(isPresented: $showingJoinSheet) {
            JoinGroupView(engine: engine, isPresented: $showingJoinSheet)
        }
        .sheet(isPresented: $showingSettingsSheet) {
            SettingsView(engine: engine, isPresented: $showingSettingsSheet)
        }
    }

    // MARK: - Header
    private var headerSection: some View {
        HStack {
            HStack(spacing: 6) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 8, height: 8)
                Text(engine.state.rawValue)
                    .font(.system(size: 13, weight: .medium))
            }

            Spacer()

            Toggle(isOn: $engine.isPaused) {
                Text(engine.isPaused ? "Resume" : "Pause")
                    .font(.system(size: 11))
            }
            .toggleStyle(.button)
            .controlSize(.small)
        }
    }

    private var statusColor: Color {
        switch engine.state {
        case .connected: return .green
        case .connecting: return .orange
        case .disconnected: return .red
        case .paused: return .gray
        }
    }

    // MARK: - Clips List
    private var clipsSection: some View {
        Group {
            if engine.recentItems.isEmpty {
                VStack(spacing: 8) {
                    Spacer()
                    Image(systemName: "doc.on.clipboard")
                        .font(.system(size: 28))
                        .foregroundColor(.secondary)
                    Text("No clips yet")
                        .font(.system(size: 13, weight: .medium))
                        .foregroundColor(.secondary)
                    Text("Copied text or images will sync automatically.")
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 24)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List {
                    ForEach(engine.recentItems, id: \.id) { entry in
                        ClipRowView(entry: entry) {
                            engine.copyToClipboard(entry)
                        }
                    }
                }
                .listStyle(.plain)
            }
        }
    }

    // MARK: - Footer
    private var footerSection: some View {
        VStack(spacing: 8) {
            HStack {
                Button(action: copyPairingLink) {
                    HStack(spacing: 4) {
                        Image(systemName: copiedPairingLink ? "checkmark" : "qrcode")
                        Text(copiedPairingLink ? "Link Copied!" : "Share / Pair")
                    }
                    .font(.system(size: 11))
                }
                .buttonStyle(.bordered)
                .controlSize(.small)

                Button("Join...") {
                    showingJoinSheet = true
                }
                .buttonStyle(.bordered)
                .controlSize(.small)

                Spacer()

                Button(action: { showingSettingsSheet = true }) {
                    Image(systemName: "gearshape")
                        .font(.system(size: 12))
                }
                .buttonStyle(.plain)

                Button(action: { NSApplication.shared.terminate(nil) }) {
                    Image(systemName: "power")
                        .font(.system(size: 12))
                        .foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func copyPairingLink() {
        guard let uri = engine.pairingUri() else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(uri, forType: .string)
        copiedPairingLink = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            copiedPairingLink = false
        }
    }
}

// MARK: - Clip Row
struct ClipRowView: View {
    let entry: FfiHistoryEntry
    let onCopy: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: onCopy) {
            HStack(alignment: .top, spacing: 10) {
                // Icon
                Image(systemName: entry.kind == .text ? "text.alignleft" : "photo")
                    .font(.system(size: 13))
                    .foregroundColor(.accentColor)
                    .frame(width: 20)

                // Content
                VStack(alignment: .leading, spacing: 2) {
                    if entry.kind == .text, let text = String(data: entry.data, encoding: .utf8) {
                        Text(text)
                            .lineLimit(2)
                            .font(.system(size: 12))
                            .foregroundColor(.primary)
                    } else if entry.kind == .image {
                        HStack {
                            Text("Image (\(ByteCountFormatter.string(fromByteCount: Int64(entry.size), countStyle: .file)))")
                                .font(.system(size: 12, weight: .medium))
                                .foregroundColor(.primary)
                            Spacer()
                        }
                    }

                    HStack(spacing: 6) {
                        Text(relativeTimeString(from: entry.createdAt))
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)

                        Text("•")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)

                        Text(entry.isRemote ? "From remote" : "This Mac")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                }

                Spacer()
            }
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func relativeTimeString(from ms: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000.0)
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}

// MARK: - Join Sheet
struct JoinGroupView: View {
    @ObservedObject var engine: SyncEngine
    @Binding var isPresented: Bool
    @State private var uriText: String = ""
    @State private var errorMessage: String = ""

    var body: some View {
        VStack(spacing: 16) {
            Text("Join Existing Group")
                .font(.headline)

            Text("Paste the pairing URI (clipbridge://pair?d=...) from another device:")
                .font(.caption)
                .foregroundColor(.secondary)

            TextEditor(text: $uriText)
                .font(.system(size: 11, design: .monospaced))
                .frame(height: 80)
                .border(Color.secondary.opacity(0.3), width: 1)

            if !errorMessage.isEmpty {
                Text(errorMessage)
                    .font(.caption)
                    .foregroundColor(.red)
            }

            HStack {
                Button("Cancel") { isPresented = false }
                Spacer()
                Button("Join") {
                    do {
                        try engine.joinGroup(pairingUri: uriText.trimmingCharacters(in: .whitespacesAndNewlines))
                        isPresented = false
                    } catch {
                        errorMessage = error.localizedDescription
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(uriText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 360)
    }
}

// MARK: - Settings Sheet
struct SettingsView: View {
    @ObservedObject var engine: SyncEngine
    @Binding var isPresented: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("ClipBridge Settings")
                .font(.headline)

            if let cfg = engine.config {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Device Name").font(.caption).foregroundColor(.secondary)
                    Text(cfg.deviceName).font(.system(size: 12, weight: .medium))

                    Text("Device ID").font(.caption).foregroundColor(.secondary)
                    Text(cfg.deviceId).font(.system(size: 11, design: .monospaced))

                    Text("Relay Endpoint").font(.caption).foregroundColor(.secondary)
                    Text(cfg.relayUrl).font(.system(size: 11, design: .monospaced))
                }
            }

            Divider()

            HStack {
                Spacer()
                Button("Done") { isPresented = false }
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(20)
        .frame(width: 340)
    }
}
