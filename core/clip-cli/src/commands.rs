//! Interactive CLI commands and one-shot utility handlers.
//!
//! Provides handlers for `init`, `join`, `pair`, `send-text`, `send-image`,
//! `listen`, `history`, and `status`.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clip_core::envelope::Envelope;
use clip_core::keys::GroupKey;
use clip_core::model::{ClipKind, ClipPayload};
use clip_core::pairing::PairingInfo;
use clip_core::relay::{RelayConfig, RelayConnection, RelayEvent};
use clip_core::storage::{History, HistoryEntry};

use crate::config::{default_device_name, AppConfig, AppPaths};

/// Maximum duration to wait for a relay acknowledgment during one-shot publish operations.
pub const ACK_TIMEOUT: Duration = Duration::from_secs(10);

/// Initializes a new device group and writes initial configuration.
pub fn cmd_init(
    paths: &AppPaths,
    relay: String,
    name: Option<String>,
    force: bool,
) -> Result<()> {
    if paths.config_file.exists() && !force {
        bail!(
            "Configuration already exists at '{}'. Use --force to overwrite.",
            paths.config_file.display()
        );
    }

    paths.ensure_dirs()?;

    let key = GroupKey::generate();
    let device_id = clip_core::new_device_id();
    let device_name = name.unwrap_or_else(default_device_name);

    let cfg = AppConfig::new(&device_id, &device_name, &relay, &key)?;
    cfg.save(&paths.config_file)?;

    let pairing = PairingInfo::new(&cfg.relay_url, &key);
    let group_id = key.group_id();
    let prefix = &group_id[..16];

    println!("Initialized new ClipBridge group.");
    println!("Device ID:       {}", device_id);
    println!("Device Name:     {}", device_name);
    println!("Relay:           {}", cfg.relay_url);
    println!("Group ID prefix: {}", prefix);
    println!("Pairing URI:     {}", pairing.to_uri());

    Ok(())
}

/// Joins an existing group using a pairing URI.
pub fn cmd_join(
    paths: &AppPaths,
    uri: &str,
    name: Option<String>,
    force: bool,
) -> Result<()> {
    if paths.config_file.exists() && !force {
        bail!(
            "Configuration already exists at '{}'. Use --force to overwrite.",
            paths.config_file.display()
        );
    }

    let pairing = PairingInfo::from_uri(uri)
        .context("invalid or unsupported pairing URI")?;
    let key = pairing
        .group_key()
        .context("failed to decode group key from pairing URI")?;

    paths.ensure_dirs()?;

    let device_id = clip_core::new_device_id();
    let device_name = name.unwrap_or_else(default_device_name);

    let cfg = AppConfig::new(&device_id, &device_name, &pairing.relay_url, &key)?;
    cfg.save(&paths.config_file)?;

    let group_id = key.group_id();
    let prefix = &group_id[..16];

    println!("Successfully joined ClipBridge group.");
    println!("Device ID:       {}", device_id);
    println!("Device Name:     {}", device_name);
    println!("Relay:           {}", cfg.relay_url);
    println!("Group ID prefix: {}", prefix);

    Ok(())
}

/// Prints the pairing URI for sharing with secondary devices.
pub fn cmd_pair(paths: &AppPaths) -> Result<()> {
    let cfg = AppConfig::load(&paths.config_file)?;
    let key = cfg.parse_group_key()?;
    let pairing = PairingInfo::new(&cfg.relay_url, &key);
    println!("{}", pairing.to_uri());
    Ok(())
}

/// Sends a plain text payload to the group (one-shot).
pub async fn cmd_send_text(paths: &AppPaths, text: &str) -> Result<()> {
    let cfg = AppConfig::load(&paths.config_file)?;
    let key = cfg.parse_group_key()?;
    let payload = ClipPayload::text(text).context("failed to construct text payload")?;

    send_payload_and_wait_ack(&cfg, &key, paths, payload).await
}

/// Sends an image payload to the group (one-shot).
pub async fn cmd_send_image(paths: &AppPaths, path: &Path) -> Result<()> {
    let cfg = AppConfig::load(&paths.config_file)?;
    let key = cfg.parse_group_key()?;

    let raw_bytes = std::fs::read(path)
        .with_context(|| format!("failed to read image file at '{}'", path.display()))?;

    let png_bytes = clip_core::normalize_to_png(&raw_bytes)
        .context("failed to normalize image to standard PNG format")?;

    let payload = ClipPayload::image_png(png_bytes)
        .context("failed to construct image payload")?;

    send_payload_and_wait_ack(&cfg, &key, paths, payload).await
}

/// Helper that seals, connects, publishes a payload, and waits for server ACK with a timeout.
async fn send_payload_and_wait_ack(
    cfg: &AppConfig,
    key: &GroupKey,
    paths: &AppPaths,
    payload: ClipPayload,
) -> Result<()> {
    let env = Envelope::seal(key, &cfg.device_id, &payload)
        .context("failed to seal payload into encrypted envelope")?;

    let relay_cfg = RelayConfig {
        base_url: cfg.relay_url.clone(),
        device_id: cfg.device_id.clone(),
        key: key.clone(),
        since: None,
    };

    let mut conn = RelayConnection::connect(&relay_cfg)
        .await
        .with_context(|| format!("failed to connect to relay hub at '{}'", cfg.relay_url))?;

    conn.publish(&env)
        .await
        .with_context(|| format!("failed to publish envelope '{}' to relay", env.id))?;

    // Wait for the relay server acknowledgment with a strict timeout
    let ack_id = tokio::time::timeout(ACK_TIMEOUT, async {
        loop {
            match conn.next_event().await {
                Ok(Some(RelayEvent::Ack(id))) => {
                    if id == env.id {
                        return Ok(id);
                    }
                }
                Ok(Some(RelayEvent::Error { code, message, id })) => {
                    bail!("relay rejected envelope with error '{code}': {message} (envelope id: {id:?})");
                }
                Ok(Some(RelayEvent::Deliver(_))) => {
                    // Ignore incoming deliveries while waiting for our publish acknowledgment
                    continue;
                }
                Ok(None) => {
                    bail!("relay connection closed before receiving acknowledgment");
                }
                Err(e) => {
                    bail!("relay connection error while awaiting acknowledgment: {e}");
                }
            }
        }
    })
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "timed out waiting for relay acknowledgment after {} seconds",
            ACK_TIMEOUT.as_secs()
        )
    })??;

    // Record published entry in local history database if accessible
    if let Ok(history) = History::open(&paths.history_db) {
        let entry = HistoryEntry {
            id: env.id.clone(),
            origin: cfg.device_id.clone(),
            kind: payload.kind,
            mime: payload.mime,
            hash: payload.hash,
            size: payload.data.len(),
            created_at: env.created_at,
            data: payload.data,
            is_remote: false,
        };
        let _ = history.insert(&entry);
    }

    println!("Acknowledged: {ack_id}");
    Ok(())
}

/// Listens for remote clipboard events and prints them to stdout without modifying the clipboard.
pub async fn cmd_listen(paths: &AppPaths) -> Result<()> {
    let cfg = AppConfig::load(&paths.config_file)?;
    let group_key = cfg.parse_group_key()?;

    let history = History::open(&paths.history_db).ok();
    let since = history
        .as_ref()
        .and_then(|h| h.last_remote_envelope_id().ok().flatten());

    let relay_cfg = RelayConfig {
        base_url: cfg.relay_url.clone(),
        device_id: cfg.device_id.clone(),
        key: group_key.clone(),
        since,
    };

    let mut conn = RelayConnection::connect(&relay_cfg)
        .await
        .with_context(|| format!("failed to connect to relay at '{}'", cfg.relay_url))?;

    let group_id = group_key.group_id();
    println!(
        "Listening for clipboard events on {} (group: {})...",
        cfg.relay_url,
        &group_id[..16]
    );

    loop {
        tokio::select! {
            event_res = conn.next_event() => {
                match event_res {
                    Ok(Some(RelayEvent::Deliver(env))) => {
                        match env.open(&group_key) {
                            Ok(payload) => {
                                match payload.kind {
                                    ClipKind::Text => {
                                        let text = payload.as_text().unwrap_or("<invalid utf-8>");
                                        let char_count = text.chars().count();
                                        let preview: String = text.chars().take(200).collect();
                                        let suffix = if char_count > 200 { "..." } else { "" };
                                        println!(
                                            "[{}] text ({} bytes): {}{}",
                                            env.sender,
                                            payload.data.len(),
                                            preview,
                                            suffix
                                        );
                                    }
                                    ClipKind::Image => {
                                        println!(
                                            "[{}] image/png {} bytes",
                                            env.sender,
                                            payload.data.len()
                                        );
                                    }
                                }

                                if let Some(ref h) = history {
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
                                    let _ = h.insert(&entry);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("failed to open incoming envelope {}: {}", env.id, e);
                            }
                        }
                    }
                    Ok(Some(RelayEvent::Ack(id))) => {
                        tracing::debug!(envelope_id = %id, "received server ack");
                    }
                    Ok(Some(RelayEvent::Error { code, message, id })) => {
                        tracing::warn!(code = %code, message = %message, ?id, "relay reported error");
                    }
                    Ok(None) => {
                        println!("Relay server closed connection.");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("relay read error: {e}");
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nStopped listening.");
                break;
            }
        }
    }

    Ok(())
}

/// Lists recent history entries from the local database.
pub fn cmd_history(paths: &AppPaths, limit: usize) -> Result<()> {
    if !paths.history_db.exists() {
        println!(
            "No clipboard history database found at '{}'.",
            paths.history_db.display()
        );
        return Ok(());
    }

    let history = History::open(&paths.history_db)
        .context("failed to open history database")?;

    let entries = history
        .recent(limit)
        .context("failed to query recent history entries")?;

    if entries.is_empty() {
        println!("No clipboard history entries recorded.");
        return Ok(());
    }

    println!(
        "{:<26}  {:<23}  {:<10}  {:<6}  {:<10}  PREVIEW",
        "ID", "TIMESTAMP (UTC)", "ORIGIN", "KIND", "SIZE"
    );
    println!("{}", "-".repeat(95));

    for entry in entries {
        let time_str = format_epoch_ms(entry.created_at);
        let origin_str = if entry.is_remote {
            if entry.origin.len() > 10 {
                format!("{}...", &entry.origin[..7])
            } else {
                entry.origin
            }
        } else {
            "local".to_string()
        };
        let kind_str = match entry.kind {
            ClipKind::Text => "text",
            ClipKind::Image => "image",
        };
        let size_str = format!("{} B", entry.size);
        let preview = match entry.kind {
            ClipKind::Text => match std::str::from_utf8(&entry.data) {
                Ok(s) => {
                    let first_line = s.lines().next().unwrap_or("").trim();
                    let count = first_line.chars().count();
                    if count > 35 {
                        format!("{}...", first_line.chars().take(32).collect::<String>())
                    } else {
                        first_line.to_string()
                    }
                }
                Err(_) => "<binary text>".to_string(),
            },
            ClipKind::Image => "[PNG image]".to_string(),
        };

        println!(
            "{:<26}  {:<23}  {:<10}  {:<6}  {:<10}  {}",
            entry.id, time_str, origin_str, kind_str, size_str, preview
        );
    }

    Ok(())
}

/// Displays device status, group ID prefix, and storage locations without revealing secret keys.
pub fn cmd_status(paths: &AppPaths) -> Result<()> {
    let cfg = AppConfig::load(&paths.config_file)?;
    let key = cfg.parse_group_key()?;
    let group_id = key.group_id();
    let prefix = &group_id[..16];

    println!("ClipBridge Configuration:");
    println!("  Device ID:       {}", cfg.device_id);
    println!("  Device Name:     {}", cfg.device_name);
    println!("  Relay URL:       {}", cfg.relay_url);
    println!("  Group ID prefix: {}", prefix);
    println!("  Config File:     {}", paths.config_file.display());
    println!("  History DB:      {}", paths.history_db.display());

    Ok(())
}

/// Formats millisecond Unix epoch time into a UTC timestamp string (`YYYY-MM-DD HH:MM:SS UTC`).
fn format_epoch_ms(ms: i64) -> String {
    let total_secs = ms / 1000;
    let s = (total_secs % 60).max(0);
    let total_mins = total_secs / 60;
    let m = (total_mins % 60).max(0);
    let total_hours = total_mins / 60;
    let h = (total_hours % 24).max(0);
    let mut days = (total_hours / 24).max(0);

    let mut year = 1970;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let days_in_months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];

    let mut month = 1;
    for &dim in &days_in_months {
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    let day = days + 1;

    format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_epoch_ms() {
        // Unix epoch 0ms = 1970-01-01 00:00:00 UTC
        assert_eq!(format_epoch_ms(0), "1970-01-01 00:00:00 UTC");

        // 1762750000000 ms = 2025-11-10 04:46:40 UTC
        let formatted = format_epoch_ms(1762750000000);
        assert!(formatted.starts_with("2025-11-10"));
    }
}
