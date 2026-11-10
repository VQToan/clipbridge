//! Dedicated clipboard listener and applicator thread.
//!
//! # Platform Affinity and Thread Safety
//! Clipboard libraries (such as `arboard`) interface with platform-specific OS subsystems
//! (macOS NSPasteboard / Cocoa runloop, X11 event loops, Wayland data-control protocols).
//! These native APIs are not `Send` or thread-safe on all desktop operating systems.
//!
//! To prevent segmentation faults and race conditions, all direct clipboard interactions
//! are isolated to a single dedicated OS thread (`std::thread::spawn`). Communication
//! with the asynchronous network layer occurs exclusively via cross-thread channels.

use std::time::Duration;

use clip_core::image::{png_to_rgba, rgba_to_png};
use clip_core::{ClipKind, ClipPayload};

/// Event signaling that a new item was copied to the local OS clipboard.
#[derive(Debug, Clone)]
pub struct LocalChange(pub ClipPayload);

/// Command requesting that remote content be written into the local OS clipboard.
#[derive(Debug, Clone)]
pub struct ApplyRemote(pub ClipPayload);

/// Spawns the dedicated clipboard poller and applicator thread.
///
/// Returns a `std::thread::JoinHandle` for monitoring thread lifetime.
pub fn spawn_clipboard_thread(
    poll_interval: Duration,
    no_images: bool,
    local_tx: tokio::sync::mpsc::Sender<LocalChange>,
    apply_rx: std::sync::mpsc::Receiver<ApplyRemote>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("clipbridge-cb".to_string())
        .spawn(move || {
            run_clipboard_loop(poll_interval, no_images, local_tx, apply_rx);
        })
        .expect("failed to spawn clipboard worker thread")
}

fn run_clipboard_loop(
    poll_interval: Duration,
    no_images: bool,
    local_tx: tokio::sync::mpsc::Sender<LocalChange>,
    apply_rx: std::sync::mpsc::Receiver<ApplyRemote>,
) {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(cb) => cb,
        Err(e) => {
            tracing::error!("failed to initialize system clipboard: {e}");
            return;
        }
    };

    let mut last_seen_hash: Option<Vec<u8>> = None;

    // Pre-populate last_seen_hash with whatever is currently in the clipboard
    // so starting the daemon does not broadcast stale pre-existing items.
    read_current_clipboard(&mut clipboard, no_images, &mut last_seen_hash, None);

    loop {
        // Wait up to `poll_interval` for any remote clipboard payloads to apply.
        match apply_rx.recv_timeout(poll_interval) {
            Ok(cmd) => {
                apply_payload_to_clipboard(&mut clipboard, &cmd.0, &mut last_seen_hash);
                // Drain any additional remote items that queued up
                while let Ok(next_cmd) = apply_rx.try_recv() {
                    apply_payload_to_clipboard(&mut clipboard, &next_cmd.0, &mut last_seen_hash);
                }
                // Skip local poll immediately after applying remote payload
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Timeout elapsed: time to poll the local clipboard
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                tracing::debug!("clipboard apply channel disconnected; terminating worker");
                break;
            }
        }

        let should_continue = read_current_clipboard(
            &mut clipboard,
            no_images,
            &mut last_seen_hash,
            Some(&local_tx),
        );
        if !should_continue {
            break;
        }
    }
}

/// Applies a received remote payload to the system clipboard and updates `last_seen_hash`.
fn apply_payload_to_clipboard(
    clipboard: &mut arboard::Clipboard,
    payload: &ClipPayload,
    last_seen_hash: &mut Option<Vec<u8>>,
) {
    match payload.kind {
        ClipKind::Text => {
            if let Some(text) = payload.as_text() {
                if let Err(e) = clipboard.set_text(text) {
                    tracing::warn!("failed to set local clipboard text: {e}");
                } else {
                    *last_seen_hash = Some(payload.hash.clone());
                    tracing::debug!(size = payload.data.len(), "applied remote text to clipboard");
                }
            }
        }
        ClipKind::Image => {
            match png_to_rgba(&payload.data) {
                Ok((width, height, rgba)) => {
                    let image_data = arboard::ImageData {
                        width: width as usize,
                        height: height as usize,
                        bytes: std::borrow::Cow::Owned(rgba),
                    };
                    if let Err(e) = clipboard.set_image(image_data) {
                        tracing::warn!("failed to set local clipboard image: {e}");
                    } else {
                        *last_seen_hash = Some(payload.hash.clone());
                        tracing::debug!(
                            size = payload.data.len(),
                            width,
                            height,
                            "applied remote image to clipboard"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to convert remote PNG payload to RGBA: {e}");
                }
            }
        }
    }
}

/// Reads current system clipboard content and emits `LocalChange` if hash changed.
///
/// Returns `false` if the local change receiver channel was closed (indicating shutdown).
fn read_current_clipboard(
    clipboard: &mut arboard::Clipboard,
    no_images: bool,
    last_seen_hash: &mut Option<Vec<u8>>,
    sender: Option<&tokio::sync::mpsc::Sender<LocalChange>>,
) -> bool {
    // 1. Try reading text
    match clipboard.get_text() {
        Ok(text) => {
            if !text.is_empty() {
                match ClipPayload::text(&text) {
                    Ok(payload) => {
                        if last_seen_hash.as_deref() != Some(&payload.hash) {
                            *last_seen_hash = Some(payload.hash.clone());
                            if let Some(tx) = sender {
                                if let Err(e) = tx.blocking_send(LocalChange(payload)) {
                                    tracing::debug!("local change receiver closed: {e}");
                                    return false;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("failed to construct text payload: {e}");
                    }
                }
                return true;
            }
        }
        Err(_) => {
            // Text not present on clipboard; fall through to image inspection
        }
    }

    // 2. Try reading image (if image sync is enabled)
    if !no_images {
        if let Ok(img) = clipboard.get_image() {
            let width = img.width as u32;
            let height = img.height as u32;
            match rgba_to_png(width, height, &img.bytes) {
                Ok(png_bytes) => match ClipPayload::image_png(png_bytes) {
                    Ok(payload) => {
                        if last_seen_hash.as_deref() != Some(&payload.hash) {
                            *last_seen_hash = Some(payload.hash.clone());
                            if let Some(tx) = sender {
                                if let Err(e) = tx.blocking_send(LocalChange(payload)) {
                                    tracing::debug!("local change receiver closed: {e}");
                                    return false;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("failed to construct image payload: {e}");
                    }
                },
                Err(e) => {
                    tracing::warn!("failed to encode clipboard image to PNG: {e}");
                }
            }
        }
    }

    true
}
