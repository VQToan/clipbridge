# ClipBridge

Cross-platform clipboard sync for **Android · iOS · macOS · Linux** — text & images, end-to-end encrypted.

> Status: **Research / Planning** (v0.0)

## Docs
- [Research report](docs/research.md) — platform constraints, competitors, tech choices
- [Architecture draft](docs/architecture.md) — components, data flow, domain model

## Planned stack
| Layer | Tech |
|---|---|
| Shared core | Rust (crypto, protocol, storage, sync) + UniFFI |
| Apple (iOS + macOS) | SwiftUI, Keyboard/Share extensions, App Intents |
| Android | Kotlin + Jetpack Compose, IME, Quick Settings Tile |
| Linux | Rust daemon (X11 + Wayland data-control), tray |
| Relay server | Go (Chi + WebSocket), Redis, S3-compatible blob store, APNs/FCM |
