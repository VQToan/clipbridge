# ClipBridge Android

Native Android application for ClipBridge with Jetpack Compose, Material 3, and Rust core bindings via UniFFI.

## Key Features & Android 10+ Integration
- **Quick Settings Tile (`ClipSyncTileService`)**: 1-tap clipboard synchronization directly from the Android notification shade, bypassing Android 10+ background reading restrictions legally without prohibited accessibility hacks.
- **Text Selection Context Menu (`ACTION_PROCESS_TEXT`)**: Select any text in any app, tap "Send to ClipBridge" to sync instantly.
- **System Share Sheet (`ACTION_SEND`)**: Supports sharing text and images directly into ClipBridge.
- **Continuous Background Sync (`ClipboardSyncService`)**: Foreground service maintaining the E2EE WebSocket connection and writing received clips via `ClipboardManager.setPrimaryClip`.
- **Zero-Knowledge E2EE**: All crypto (BLAKE3 + XChaCha20-Poly1305) and envelope processing runs in the compiled Rust `clip-core` engine via `clip-ffi`.

## Building Native Rust Libraries (.so)
To compile the Rust FFI crate for Android targets:
```bash
# Install target
rustup target add aarch64-linux-android x86_64-linux-android

# Build using cargo-ndk
cd ../../core
cargo ndk -t arm64-v8a -t x86_64 -o ../apps/android/app/src/main/jniLibs build -p clip-ffi --release
```

## Building the APK
```bash
./gradlew assembleDebug
```
