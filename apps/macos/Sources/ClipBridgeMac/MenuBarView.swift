import SwiftUI

struct MenuBarView: View {
    @ObservedObject var engine: SyncEngine
    @State private var showingPairSheet = false
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
        .sheet(isPresented: $showingPairSheet) {
            PairDeviceView(engine: engine, isPresented: $showingPairSheet)
        }
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
                Button(action: { showingPairSheet = true }) {
                    HStack(spacing: 4) {
                        Image(systemName: "link.badge.plus")
                        Text("Pair Device")
                    }
                    .font(.system(size: 11, weight: .medium))
                }
                .buttonStyle(.borderedProminent)
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

// MARK: - Pair Device Sheet
struct PairDeviceView: View {
    @ObservedObject var engine: SyncEngine
    @Binding var isPresented: Bool
    @State private var pairingCode: String = ""
    @State private var isLoading = true
    @State private var errorMessage: String = ""
    @State private var copiedCode = false
    @State private var copiedLink = false

    var body: some View {
        VStack(spacing: 16) {
            Text("Ghép đôi thiết bị mới")
                .font(.headline)

            Text("Nhập mã kết nối này trên điện thoại hoặc thiết bị khác của bạn để kết nối vào nhóm:")
                .font(.caption)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)

            if isLoading {
                ProgressView()
                    .padding(.vertical, 12)
            } else if !pairingCode.isEmpty {
                VStack(spacing: 8) {
                    Text(formattedCode(pairingCode))
                        .font(.system(size: 32, weight: .bold, design: .monospaced))
                        .tracking(4)
                        .padding(.vertical, 8)
                        .padding(.horizontal, 20)
                        .background(Color.secondary.opacity(0.1))
                        .cornerRadius(8)

                    Text("Mã có hiệu lực trong 10 phút")
                        .font(.caption2)
                        .foregroundColor(.secondary)
                }

                HStack(spacing: 12) {
                    Button(action: copyCode) {
                        HStack(spacing: 4) {
                            Image(systemName: copiedCode ? "checkmark" : "doc.on.doc")
                            Text(copiedCode ? "Đã chép mã" : "Chép mã 6 số")
                        }
                    }
                    .buttonStyle(.bordered)

                    Button(action: copyLink) {
                        HStack(spacing: 4) {
                            Image(systemName: copiedLink ? "checkmark" : "link")
                            Text(copiedLink ? "Đã chép link" : "Chép link ghép đôi")
                        }
                    }
                    .buttonStyle(.bordered)
                }
            } else {
                Text(errorMessage.isEmpty ? "Không thể lấy mã ghép đôi" : errorMessage)
                    .font(.caption)
                    .foregroundColor(.red)
            }

            Divider()

            HStack {
                Spacer()
                Button("Đóng") { isPresented = false }
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(20)
        .frame(width: 380)
        .onAppear {
            loadPairingCode()
        }
    }

    private func formattedCode(_ c: String) -> String {
        if c.count == 6 {
            let p1 = c.prefix(3)
            let p2 = c.suffix(3)
            return "\(p1) \(p2)"
        }
        return c
    }

    private func copyCode() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(pairingCode, forType: .string)
        copiedCode = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { copiedCode = false }
    }

    private func copyLink() {
        guard let uri = engine.pairingUri() else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(uri, forType: .string)
        copiedLink = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { copiedLink = false }
    }

    private func loadPairingCode() {
        isLoading = true
        errorMessage = ""
        Task {
            do {
                let code = try await engine.createPairingCode()
                await MainActor.run {
                    self.pairingCode = code
                    self.isLoading = false
                }
            } catch {
                await MainActor.run {
                    self.errorMessage = error.localizedDescription
                    self.isLoading = false
                }
            }
        }
    }
}

// MARK: - Join Sheet
struct JoinGroupView: View {
    @ObservedObject var engine: SyncEngine
    @Binding var isPresented: Bool
    @State private var inputText: String = ""
    @State private var isJoining = false
    @State private var errorMessage: String = ""

    var body: some View {
        VStack(spacing: 16) {
            Text("Tham gia nhóm thiết bị")
                .font(.headline)

            Text("Nhập mã kết nối 6 số, mã nhóm (Group Key) hoặc link ghép đôi từ thiết bị khác:")
                .font(.caption)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)

            TextField("Ví dụ: 839 201 hoặc IdJwKRJ...", text: $inputText)
                .font(.system(size: 13, design: .monospaced))
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: .infinity)

            if !errorMessage.isEmpty {
                Text(errorMessage)
                    .font(.caption)
                    .foregroundColor(.red)
            }

            HStack {
                Button("Hủy") { isPresented = false }
                Spacer()
                Button(action: handleJoin) {
                    if isJoining {
                        ProgressView().controlSize(.small)
                    } else {
                        Text("Kết nối")
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isJoining)
            }
        }
        .padding(20)
        .frame(width: 380)
    }

    private func handleJoin() {
        isJoining = true
        errorMessage = ""
        Task {
            do {
                try await engine.joinWithCodeOrKey(inputText)
                await MainActor.run {
                    self.isJoining = false
                    self.isPresented = false
                }
            } catch {
                await MainActor.run {
                    self.errorMessage = error.localizedDescription
                    self.isJoining = false
                }
            }
        }
    }
}

// MARK: - Settings Sheet
struct SettingsView: View {
    @ObservedObject var engine: SyncEngine
    @Binding var isPresented: Bool
    @State private var showResetConfirm = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Cài đặt ClipBridge")
                .font(.headline)

            if let cfg = engine.config {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Tên thiết bị").font(.caption).foregroundColor(.secondary)
                    Text(cfg.deviceName).font(.system(size: 12, weight: .medium))

                    Text("Mã thiết bị (Device ID)").font(.caption).foregroundColor(.secondary)
                    Text(cfg.deviceId).font(.system(size: 11, design: .monospaced))

                    Text("Mã nhóm hiện tại (Group Key)").font(.caption).foregroundColor(.secondary)
                    Text(cfg.groupKey.prefix(12) + "...").font(.system(size: 11, design: .monospaced))

                    Text("Máy chủ Relay").font(.caption).foregroundColor(.secondary)
                    Text(cfg.relayUrl).font(.system(size: 11, design: .monospaced))
                }
            }

            Divider()

            VStack(alignment: .leading, spacing: 6) {
                Text("Tạo bộ thiết bị riêng biệt")
                    .font(.subheadline)
                    .fontWeight(.medium)
                Text("Dùng khi bạn muốn chia sẻ app cho người khác hoặc muốn tạo nhóm đồng bộ mới hoàn toàn tách biệt:")
                    .font(.caption2)
                    .foregroundColor(.secondary)

                Button(role: .destructive, action: { showResetConfirm = true }) {
                    Label("Tạo nhóm thiết bị mới", systemImage: "sparkles")
                        .font(.system(size: 11))
                }
                .buttonStyle(.bordered)
            }
            .confirmationDialog(
                "Tạo nhóm thiết bị mới?",
                isPresented: $showResetConfirm,
                titleVisibility: .visible
            ) {
                Button("Tạo nhóm mới", role: .destructive) {
                    engine.createNewGroup()
                    isPresented = false
                }
                Button("Hủy", role: .cancel) {}
            } message: {
                Text("Các thiết bị khác đang kết nối vào nhóm cũ sẽ không thể đồng bộ với máy này nữa cho đến khi được ghép đôi lại.")
            }

            Divider()

            HStack {
                Spacer()
                Button("Xong") { isPresented = false }
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(20)
        .frame(width: 360)
    }
}
