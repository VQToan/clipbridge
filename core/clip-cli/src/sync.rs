//! Background clipboard synchronization daemon and reconnection logic.
//!
//! Orchestrates the clipboard polling thread, local SQLite history pruning,
//! envelope sealing/opening, deduplication via `EchoGuard`, and resilient
//! WebSocket connection management with exponential backoff and jitter.

use std::time::Duration;

use clip_core::envelope::Envelope;
use clip_core::guard::EchoGuard;
use clip_core::relay::{RelayConfig, RelayConnection, RelayEvent};
use clip_core::storage::{History, HistoryEntry};

use crate::clipboard::{spawn_clipboard_thread, ApplyRemote, LocalChange};
use crate::config::AppConfig;

/// Initial backoff duration upon connection failure.
pub const BACKOFF_INITIAL: Duration = Duration::from_secs(1);

/// Maximum backoff duration capping exponential growth.
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Exponential multiplier for successive backoff attempts.
pub const BACKOFF_FACTOR: f64 = 2.0;

/// Maximum jitter variation (+/- 25% of current base duration).
pub const JITTER_FACTOR: f64 = 0.25;

/// Retention period for clipboard history entries (24 hours).
pub const HISTORY_RETENTION_HOURS: u64 = 24;

/// Interval between periodic background history pruning runs (1 hour).
pub const HISTORY_PRUNE_INTERVAL: Duration = Duration::from_secs(3600);

/// Calculates exponential backoff with random jitter.
///
/// # Formula
/// 1. `base = min(max, initial * factor^attempt)`
/// 2. `jittered = base * (1 - JITTER_FACTOR + 2 * JITTER_FACTOR * jitter_ratio)`
/// 3. Result is clamped to `[initial * (1 - JITTER_FACTOR), max]`.
pub fn calculate_backoff(
    attempt: u32,
    jitter_ratio: f64,
    initial: Duration,
    max: Duration,
    factor: f64,
) -> Duration {
    let mult = if attempt >= 32 {
        f64::MAX
    } else {
        factor.powi(attempt as i32)
    };

    let base_secs = (initial.as_secs_f64() * mult).min(max.as_secs_f64());
    let jitter_clamped = jitter_ratio.clamp(0.0, 1.0);
    let scale = (1.0 - JITTER_FACTOR) + (2.0 * JITTER_FACTOR * jitter_clamped);
    let min_floor = initial.as_secs_f64() * (1.0 - JITTER_FACTOR);
    let jittered_secs = (base_secs * scale).clamp(min_floor, max.as_secs_f64());

    Duration::from_secs_f64(jittered_secs)
}

/// Computes a pseudo-random ratio in `[0.0, 1.0]` using nanosecond clock entropy.
///
/// Avoids external RNG dependencies while providing sufficient jitter entropy.
fn current_jitter_ratio() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let mixed = (nanos ^ (nanos >> 13) ^ (nanos << 7)) % 10_000;
    (mixed as f64) / 10_000.0
}

/// Prunes history entries older than `HISTORY_RETENTION_HOURS` (24 hours).
fn prune_history(history: &History) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let cutoff_ms = now_ms.saturating_sub((HISTORY_RETENTION_HOURS * 3600 * 1000) as i64);

    match history.prune_older_than(cutoff_ms) {
        Ok(count) => {
            if count > 0 {
                tracing::info!(pruned_count = count, "pruned stale clipboard history");
            }
        }
        Err(e) => {
            tracing::warn!("failed to prune clipboard history: {e}");
        }
    }
}

/// Runs the clipboard synchronization daemon.
///
/// Continuously monitors local clipboard changes and broadcasts them to the paired
/// device group via the WebSocket relay. Concurrently receives remote changes, decrypts
/// them, and updates the local clipboard.
pub async fn run_watch(
    cfg: AppConfig,
    history: History,
    poll_ms: u64,
    no_images: bool,
) -> anyhow::Result<()> {
    // 1. Prune stale history at startup
    prune_history(&history);

    let group_key = cfg.parse_group_key()?;
    let mut guard = EchoGuard::new();

    // 2. Setup channels bridging async runtime and dedicated clipboard thread
    let (local_tx, mut local_rx) = tokio::sync::mpsc::channel::<LocalChange>(64);
    let (apply_tx, apply_rx) = std::sync::mpsc::channel::<ApplyRemote>();

    // 3. Spawn the dedicated clipboard worker thread
    let _clipboard_handle = spawn_clipboard_thread(
        Duration::from_millis(poll_ms),
        no_images,
        local_tx,
        apply_rx,
    );

    // 4. Setup periodic history pruning timer
    let mut prune_timer = tokio::time::interval(HISTORY_PRUNE_INTERVAL);
    prune_timer.tick().await; // Consume immediate first tick

    let mut attempt = 0u32;
    tracing::info!(
        device_id = %cfg.device_id,
        device_name = %cfg.device_name,
        relay = %cfg.relay_url,
        poll_ms = poll_ms,
        no_images = no_images,
        "starting ClipBridge synchronization daemon"
    );

    loop {
        // Fetch latest known remote envelope ID to request catch-up backlog
        let since = match history.last_remote_envelope_id() {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("failed to read last remote envelope ID from database: {e}");
                None
            }
        };

        let relay_cfg = RelayConfig {
            base_url: cfg.relay_url.clone(),
            device_id: cfg.device_id.clone(),
            key: group_key.clone(),
            since: since.clone(),
        };

        tracing::info!(
            relay = %cfg.relay_url,
            since = ?since,
            attempt = attempt,
            "connecting to relay hub..."
        );

        let conn = match RelayConnection::connect(&relay_cfg).await {
            Ok(c) => {
                tracing::info!("connected to relay hub");
                attempt = 0; // Reset backoff upon successful connection
                c
            }
            Err(e) => {
                tracing::warn!("failed to connect to relay hub: {e}");
                let backoff_duration = calculate_backoff(
                    attempt,
                    current_jitter_ratio(),
                    BACKOFF_INITIAL,
                    BACKOFF_MAX,
                    BACKOFF_FACTOR,
                );
                attempt = attempt.saturating_add(1);

                tracing::info!(
                    backoff_ms = backoff_duration.as_millis(),
                    "waiting before reconnection attempt..."
                );

                tokio::select! {
                    _ = tokio::time::sleep(backoff_duration) => continue,
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("received shutdown signal during backoff; stopping daemon");
                        break;
                    }
                }
            }
        };

        let (mut sender, mut receiver) = conn.split();
        let mut disconnected = false;

        while !disconnected {
            tokio::select! {
                // Handle local clipboard change from worker thread
                local_opt = local_rx.recv() => {
                    match local_opt {
                        Some(LocalChange(payload)) => {
                            if guard.should_publish(&payload.hash) {
                                match Envelope::seal(&group_key, &cfg.device_id, &payload) {
                                    Ok(env) => {
                                        let hash_hex: String = payload
                                            .hash
                                            .iter()
                                            .take(4)
                                            .map(|b| format!("{b:02x}"))
                                            .collect();

                                        tracing::info!(
                                            envelope_id = %env.id,
                                            kind = %payload.kind,
                                            size = payload.data.len(),
                                            hash_prefix = %hash_hex,
                                            "publishing local clipboard envelope"
                                        );

                                        if let Err(e) = sender.publish(&env).await {
                                            tracing::warn!("failed to publish envelope to relay: {e}");
                                            disconnected = true;
                                        } else {
                                            let entry = HistoryEntry {
                                                id: env.id,
                                                origin: cfg.device_id.clone(),
                                                kind: payload.kind,
                                                mime: payload.mime,
                                                hash: payload.hash,
                                                size: payload.data.len(),
                                                created_at: env.created_at,
                                                data: payload.data,
                                                is_remote: false,
                                            };
                                            if let Err(e) = history.insert(&entry) {
                                                tracing::warn!("failed to record local item in history: {e}");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("failed to seal clipboard envelope: {e}");
                                    }
                                }
                            } else {
                                tracing::debug!("suppressed echo of recently applied remote clipboard content");
                            }
                        }
                        None => {
                            tracing::debug!("local change channel closed; stopping daemon");
                            return Ok(());
                        }
                    }
                }

                // Handle incoming events from the WebSocket relay
                event_res = receiver.next_event() => {
                    match event_res {
                        Ok(Some(RelayEvent::Deliver(env))) => {
                            if guard.seen_envelope(&env.id) {
                                tracing::debug!(envelope_id = %env.id, "skipping duplicate envelope");
                                continue;
                            }

                            match env.open(&group_key) {
                                Ok(payload) => {
                                    let hash_hex: String = payload
                                        .hash
                                        .iter()
                                        .take(4)
                                        .map(|b| format!("{b:02x}"))
                                        .collect();

                                    tracing::info!(
                                        envelope_id = %env.id,
                                        origin = %env.sender,
                                        kind = %payload.kind,
                                        size = payload.data.len(),
                                        hash_prefix = %hash_hex,
                                        "received remote clipboard envelope"
                                    );

                                    guard.mark_applied(&payload.hash);

                                    if let Err(e) = apply_tx.send(ApplyRemote(payload.clone())) {
                                        tracing::error!("failed to forward remote payload to clipboard thread: {e}");
                                    }

                                    let entry = HistoryEntry {
                                        id: env.id,
                                        origin: env.sender,
                                        kind: payload.kind,
                                        mime: payload.mime,
                                        hash: payload.hash,
                                        size: payload.data.len(),
                                        created_at: env.created_at,
                                        data: payload.data,
                                        is_remote: true,
                                    };
                                    if let Err(e) = history.insert(&entry) {
                                        tracing::warn!("failed to record remote item in history: {e}");
                                    }
                                }
                                Err(e) => {
                                    // Log and skip corrupted or undecryptable envelopes without crashing
                                    tracing::warn!(
                                        envelope_id = %env.id,
                                        "failed to decrypt/open received envelope: {e}"
                                    );
                                }
                            }
                        }
                        Ok(Some(RelayEvent::Ack(id))) => {
                            tracing::debug!(envelope_id = %id, "received server publish acknowledgement");
                        }
                        Ok(Some(RelayEvent::Error { code, message, id })) => {
                            tracing::warn!(code = %code, message = %message, ?id, "relay reported error");
                        }
                        Ok(None) => {
                            tracing::info!("relay server closed connection; initiating reconnect");
                            disconnected = true;
                        }
                        Err(e) => {
                            tracing::warn!("relay connection stream error: {e}");
                            disconnected = true;
                        }
                    }
                }

                // Periodic hourly history cleanup
                _ = prune_timer.tick() => {
                    prune_history(&history);
                }

                // Graceful termination
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("received interrupt signal (Ctrl+C); shutting down gracefully");
                    drop(apply_tx);
                    return Ok(());
                }
            }
        }

        // Apply backoff before re-establishing dropped connection
        let backoff_duration = calculate_backoff(
            attempt,
            current_jitter_ratio(),
            BACKOFF_INITIAL,
            BACKOFF_MAX,
            BACKOFF_FACTOR,
        );
        attempt = attempt.saturating_add(1);

        tracing::info!(
            backoff_ms = backoff_duration.as_millis(),
            "connection lost; waiting before reconnect..."
        );

        tokio::select! {
            _ = tokio::time::sleep(backoff_duration) => continue,
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received interrupt signal during disconnect backoff; shutting down");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation_growth_and_cap() {
        let initial = Duration::from_secs(1);
        let max = Duration::from_secs(30);
        let factor = 2.0;

        // Attempt 0: mid jitter ratio (0.5) should equal initial 1.0s
        let b0 = calculate_backoff(0, 0.5, initial, max, factor);
        assert!((b0.as_secs_f64() - 1.0).abs() < 1e-4);

        // Attempt 1: mid jitter ratio (0.5) should equal 2.0s
        let b1 = calculate_backoff(1, 0.5, initial, max, factor);
        assert!((b1.as_secs_f64() - 2.0).abs() < 1e-4);

        // Attempt 2: 4.0s
        let b2 = calculate_backoff(2, 0.5, initial, max, factor);
        assert!((b2.as_secs_f64() - 4.0).abs() < 1e-4);

        // Attempt 3: 8.0s
        let b3 = calculate_backoff(3, 0.5, initial, max, factor);
        assert!((b3.as_secs_f64() - 8.0).abs() < 1e-4);

        // High attempt count (e.g. 10): must cap at max 30.0s
        let b_high = calculate_backoff(10, 0.5, initial, max, factor);
        assert!((b_high.as_secs_f64() - 30.0).abs() < 1e-4);

        // Extreme attempt count (e.g. 50): must not overflow and cap at max
        let b_extreme = calculate_backoff(50, 0.5, initial, max, factor);
        assert!((b_extreme.as_secs_f64() - 30.0).abs() < 1e-4);
    }

    #[test]
    fn test_backoff_jitter_bounds() {
        let initial = Duration::from_secs(1);
        let max = Duration::from_secs(30);
        let factor = 2.0;

        // Minimum jitter (ratio 0.0) -> 1.0 * (1 - 0.25) = 0.75s
        let b_min = calculate_backoff(0, 0.0, initial, max, factor);
        assert!((b_min.as_secs_f64() - 0.75).abs() < 1e-4);

        // Maximum jitter (ratio 1.0) -> 1.0 * (1 + 0.25) = 1.25s
        let b_max = calculate_backoff(0, 1.0, initial, max, factor);
        assert!((b_max.as_secs_f64() - 1.25).abs() < 1e-4);

        // Jitter must never exceed max cap even with maximum jitter ratio
        let b_capped = calculate_backoff(10, 1.0, initial, max, factor);
        assert!(b_capped <= max);
    }
}
