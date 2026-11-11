# 🌉 ClipBridge

<p align="center">
  <strong>Fast, Zero-Knowledge End-to-End Encrypted Cross-Platform Clipboard Synchronization</strong>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#architecture">Architecture</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#pairing-devices">Pairing</a> •
  <a href="#security-model">Security</a> •
  <a href="#self-hosting">Self-Hosting</a>
</p>

---

ClipBridge seamlessly and securely syncs your clipboard across **macOS**, **Android**, and **Linux** (iOS planned). It operates on a **zero-knowledge architecture**: your clipboard contents are encrypted directly on your device with **XChaCha20-Poly1305** before transmission. The relay server routes blind encrypted payloads and never possesses the keys or capability to inspect your data.

## ✨ Features

- ⚡ **Instant Real-Time Sync**: Sub-100ms clipboard synchronization across active devices.
- 🔒 **Zero-Knowledge E2EE**: 256-bit symmetric key (`GroupKey`) generated locally. Server only sees hashed BLAKE3 identifiers and encrypted envelopes.
- 🔢 **6-Digit Ephemeral Pairing**: Connect devices instantly using an intuitive 6-digit code (10-minute TTL, single-use, rendezvous claim) without manually copying long cryptographic keys.
- 👥 **Multi-Tenant Device Groups**: Create isolated device groups or share apps safely with colleagues and family—each group maintains a separate cryptographic boundary.
- 📡 **Dual Transport Modes**:
  - **Local BLE (Bluetooth Low Energy)**: Direct peer-to-peer exchange when devices are nearby without routing through the internet.
  - **Cloudflare Workers Relay**: Global low-latency WebSocket relay backed by Cloudflare Durable Objects with WebSocket Hibernation for minimal resource consumption.
- 📱 **Robust Android Background Sync**: Operates reliably on modern Android (Android 10+) using an Accessibility Service + Logcat monitor fallback with zero UI interruption.
- 🐧 **Linux First-Class Support**: Native Rust CLI and daemon supporting both **X11** and **Wayland** display servers.
- 🍏 **Native macOS MenuBar**: Lightweight SwiftUI menu bar app with real-time status and clipboard preview.

---

## 🏛️ Architecture

```
                  ┌──────────────────────────────────────────────┐
                  │ Cloudflare Workers + Durable Objects Relay  │
                  │   (Zero-Knowledge Blind Routing / Pairing)   │
                  └───────────────────────▲──────────────────────┘
                                          │
                        WebSocket / TLS   │ (Encrypted Envelopes)
                                          ▼
     ┌────────────────────────┬────────────────────────┬────────────────────────┐
     │      macOS Client      │     Android Client     │       Linux Daemon     │
     │      (SwiftUI)         │    (Jetpack Compose)   │        (Rust CLI)      │
     ├────────────────────────┴────────────────────────┴────────────────────────┤
     │                Rust Core Engine (UniFFI / Native FFI)                    │
     │   • Crypto: XChaCha20-Poly1305 AEAD + BLAKE3 Key Derivation             │
     │   • Protocol: Binary Frame Serializer (CBOR) + Anti-Replay Cache         │
     │   • Transports: WebSocket Engine + BLE Framing / GATT Peripheral        │
     └──────────────────────────────────────────────────────────────────────────┘
```

---

## 🚀 Quick Start

### Pre-Built Binaries

Pre-compiled production binaries are available in the [GitHub Releases](https://github.com/VQToan/clipbridge/releases) section:

- **Android**: Download and install `clipbridge.apk`.
- **Linux (x86_64)**: Download `clipbridge-linux-x86_64` (supports Ubuntu, Debian, Fedora, Arch, etc.).

---

### 1. Android Installation
```bash
# Install via ADB or download directly on your phone
adb install -r clipbridge.apk
```
1. Open **ClipBridge**.
2. Grant the required permissions:
   - **Accessibility Service**: Enables automatic background clipboard synchronization.
   - **Notification**: Keeps the sync service persistent.
   - **Battery Optimization**: Set to "Unrestricted" so the system does not kill the background listener.

---

### 2. Linux Setup
```bash
# Make binary executable and move to PATH
chmod +x clipbridge-linux-x86_64
sudo mv clipbridge-linux-x86_64 /usr/local/bin/clipbridge

# Check installation
clipbridge --help
```

**Run the sync daemon:**
```bash
clipbridge watch
```

---

### 3. macOS Setup
Build directly using the Xcode toolchain:
```bash
cd apps/macos
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift build -c release
```
Run `ClipBridge` from the menu bar.

---

## 🔗 Pairing Devices

### Option A: 6-Digit Code (Easiest)
1. On your primary device (e.g. macOS or Android), tap **"Mã ghép nối" (Pair Device)**.
2. A 6-digit code will appear (e.g. `482 195`), valid for 10 minutes.
3. On your second device, tap **"Tham gia nhóm" (Join Group)** and enter the 6-digit code.
4. The devices will automatically exchange the `GroupKey` securely via ephemeral rendezvous and start syncing.

### Option B: Group Key or CLI Claim
On Linux, claim the key via the relay endpoint and join:
```bash
KEY=$(curl -s -X POST https://clipbridge-relay.vqtoan1807.workers.dev/v1/pair/claim \
  -H "Content-Type: application/json" -d '{"code":"482195"}' | jq -r .key)

clipbridge join "$KEY"
clipbridge watch
```

---

## 🛡️ Security Model

1. **Local Key Generation**: The `GroupKey` (256-bit cryptographically secure random key) never leaves the device unencrypted.
2. **Deterministic Blind Group ID**:
   $$\text{group\_id} = \text{BLAKE3}(\text{GroupKey} \parallel \text{"clipbridge:group"})$$
   The relay routes messages exclusively by matching `group_id` headers. The relay cannot derive `GroupKey` from `group_id`.
3. **Payload Encryption**:
   - Algorithm: **XChaCha20-Poly1305** authenticated encryption with random 192-bit nonces.
   - Integrity: Any tampering with ciphertext drops the packet immediately.
4. **Anti-Replay & Echo Prevention**:
   - Each envelope contains a monotonic ULID timestamp and sequence identifier.
   - Devices maintain an in-memory sliding window cache of recent hash digests to prevent self-echo and replay loops.

---

## 🌐 Self-Hosting the Relay

ClipBridge works out of the box with the public relay (`wss://clipbridge-relay.vqtoan1807.workers.dev`), but you can easily self-host your own relay on Cloudflare Workers for free:

```bash
cd workers/relay
npm install
npx wrangler deploy
```

Once deployed, set the custom relay URL in your client's settings.

---

## 🛠️ Project Structure

```
clipbridge/
├── core/
│   ├── clip-crypto/        # XChaCha20-Poly1305 AEAD + BLAKE3 key derivation
│   ├── clip-relay-client/  # WebSocket client with auto-reconnect
│   ├── clip-core/          # Clipboard engine, sqlite storage, platform abstractions
│   ├── clip-ffi/           # UniFFI bindings for Kotlin & Swift
│   └── clip-cli/           # Standalone Linux/macOS CLI & watch daemon
├── apps/
│   ├── android/            # Native Kotlin + Jetpack Compose app
│   └── macos/              # Native SwiftUI macOS MenuBar app
├── workers/
│   └── relay/              # Cloudflare Workers + Durable Objects WebSocket relay
└── docs/                   # Architecture, research & protocol specifications
```

---

## 📄 License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
