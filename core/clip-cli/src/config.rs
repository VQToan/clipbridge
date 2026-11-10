//! Configuration handling and platform path resolution for ClipBridge.
//!
//! Stores device identity, relay connection endpoint, and the shared group key.
//! Configuration is persisted as JSON (`config.json`) with strict user-only file
//! permissions (0600 on Unix) because it holds cryptographic secrets.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clip_core::keys::GroupKey;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// Standard file name for the device JSON configuration.
pub const CONFIG_FILE_NAME: &str = "config.json";

/// Standard file name for the persistent SQLite history database.
pub const HISTORY_DB_NAME: &str = "history.db";

/// Root configuration payload stored on disk.
///
/// Contains client device identity and the shared symmetric group key.
///
/// # Security Note
/// The `group_key` grants full decryption access to all group clipboard payloads.
/// The configuration file is written with strict file mode `0600` on Unix platforms.
/// Integration with OS keychains (macOS Keychain, Linux Secret Service / Keyutils,
/// Windows Credential Manager) is planned for a future release to eliminate plaintext
/// secret storage on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    /// Unique identifier for this device (ULID format).
    pub device_id: String,
    /// Human-friendly name displayed to peers during sync events.
    pub device_name: String,
    /// WebSocket relay endpoint URL (`ws://...` or `wss://...`).
    pub relay_url: String,
    /// Shared 32-byte group key encoded as unpadded URL-safe base64 (43 characters).
    pub group_key: String,
}

impl AppConfig {
    /// Constructs a new `AppConfig` validating key and relay formatting.
    pub fn new(
        device_id: impl Into<String>,
        device_name: impl Into<String>,
        relay_url: impl Into<String>,
        key: &GroupKey,
    ) -> Result<Self> {
        let relay_url = relay_url.into();
        if !relay_url.starts_with("ws://") && !relay_url.starts_with("wss://") {
            bail!("relay URL must start with 'ws://' or 'wss://', got: '{relay_url}'");
        }

        Ok(Self {
            device_id: device_id.into(),
            device_name: device_name.into(),
            relay_url,
            group_key: key.to_base64url(),
        })
    }

    /// Decodes the cryptographic `GroupKey` stored in this configuration.
    pub fn parse_group_key(&self) -> Result<GroupKey> {
        GroupKey::from_base64url(&self.group_key)
            .context("failed to decode group key from configuration")
    }

    /// Loads the configuration from the given file path.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!(
                "configuration file not found at '{}'. Run 'clipbridge init' or 'clipbridge join' first.",
                path.display()
            );
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read configuration from '{}'", path.display()))?;

        let cfg: Self = serde_json::from_str(&content).with_context(|| {
            format!(
                "failed to parse JSON configuration from '{}'",
                path.display()
            )
        })?;

        // Validate key immediately to fail early on corrupt files
        let _ = cfg.parse_group_key()?;

        Ok(cfg)
    }

    /// Writes the configuration to disk with strict user-only permissions.
    ///
    /// Parent directories are created if they do not already exist.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create configuration directory '{}'",
                    parent.display()
                )
            })?;
        }

        let json = serde_json::to_string_pretty(self)
            .context("failed to serialize configuration to JSON")?;

        std::fs::write(path, json)
            .with_context(|| format!("failed to write configuration to '{}'", path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms).with_context(|| {
                format!(
                    "failed to set 0600 permissions on secret configuration file '{}'",
                    path.display()
                )
            })?;
        }

        Ok(())
    }
}

/// Resolved filesystem paths for configuration and persistent state.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// Directory containing `config.json`.
    pub config_dir: PathBuf,
    /// Absolute path to `config.json`.
    pub config_file: PathBuf,
    /// Directory containing `history.db`.
    pub data_dir: PathBuf,
    /// Absolute path to `history.db`.
    pub history_db: PathBuf,
}

impl AppPaths {
    /// Resolves standard or overridden application storage paths.
    ///
    /// If `override_dir` is supplied (from `--config-dir` or `CLIPBRIDGE_HOME`),
    /// both configuration and database files are placed in that directory.
    /// Otherwise, standard OS directories via `ProjectDirs` are utilized.
    pub fn resolve(override_dir: Option<&Path>) -> Result<Self> {
        let (config_dir, data_dir) = if let Some(custom) = override_dir {
            (custom.to_path_buf(), custom.to_path_buf())
        } else {
            let proj = ProjectDirs::from("dev", "clipbridge", "clipbridge")
                .context("could not determine OS standard application directories")?;
            (
                proj.config_dir().to_path_buf(),
                proj.data_dir().to_path_buf(),
            )
        };

        let config_file = config_dir.join(CONFIG_FILE_NAME);
        let history_db = data_dir.join(HISTORY_DB_NAME);

        Ok(Self {
            config_dir,
            config_file,
            data_dir,
            history_db,
        })
    }

    /// Ensures that both configuration and data directories exist on the filesystem.
    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.config_dir).with_context(|| {
            format!(
                "failed to create config directory '{}'",
                self.config_dir.display()
            )
        })?;
        std::fs::create_dir_all(&self.data_dir).with_context(|| {
            format!(
                "failed to create data directory '{}'",
                self.data_dir.display()
            )
        })?;
        Ok(())
    }
}

/// Determines a sensible default device name from environment variables or hostname.
pub fn default_device_name() -> String {
    if let Ok(name) = std::env::var("CLIPBRIDGE_DEVICE_NAME") {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(name) = std::env::var("HOSTNAME") {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(user) = std::env::var("USER") {
        let trimmed = user.trim();
        if !trimmed.is_empty() {
            return format!("{trimmed}-desktop");
        }
    }
    "clipbridge-device".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_save_load_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.json");

        let key = GroupKey::generate();
        let original = AppConfig::new(
            "test-device-01",
            "test-workstation",
            "ws://127.0.0.1:18080",
            &key,
        )
        .unwrap();

        original.save(&config_path).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&config_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let loaded = AppConfig::load(&config_path).unwrap();
        assert_eq!(loaded, original);
        assert_eq!(loaded.parse_group_key().unwrap(), key);
    }

    #[test]
    fn test_config_not_found_error_message() {
        let temp_dir = tempfile::tempdir().unwrap();
        let missing_path = temp_dir.path().join("nonexistent.json");

        let err = AppConfig::load(&missing_path).unwrap_err();
        assert!(err.to_string().contains("configuration file not found"));
    }

    #[test]
    fn test_app_paths_override() {
        let temp_dir = tempfile::tempdir().unwrap();
        let custom_path = temp_dir.path().join("custom_home");

        let paths = AppPaths::resolve(Some(&custom_path)).unwrap();
        assert_eq!(paths.config_dir, custom_path);
        assert_eq!(paths.data_dir, custom_path);
        assert_eq!(paths.config_file, custom_path.join(CONFIG_FILE_NAME));
        assert_eq!(paths.history_db, custom_path.join(HISTORY_DB_NAME));

        paths.ensure_dirs().unwrap();
        assert!(custom_path.exists());
    }

    #[test]
    fn test_invalid_relay_url_scheme() {
        let key = GroupKey::generate();
        let err =
            AppConfig::new("dev-1", "name", "http://insecure.dev", &key).unwrap_err();
        assert!(err.to_string().contains("relay URL must start with"));
    }
}
