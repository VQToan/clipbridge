use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::keys::GroupKey;

/// Pairing payload version.
pub const PAIRING_VERSION: u8 = 1;

/// URI scheme used for pairing QR codes and deep links.
pub const PAIRING_SCHEME: &str = "clipbridge";

/// Expected URI authority host for pairing links.
pub const PAIRING_HOST: &str = "pair";

/// Information required to bootstrap connection and pairing with a device group.
///
/// Encoded inside a `clipbridge://pair?d=<base64url(json)>` URI for QR display or scanning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingInfo {
    /// Pairing protocol version (currently 1).
    pub v: u8,
    /// WebSocket relay endpoint (`ws://...` or `wss://...`).
    pub relay_url: String,
    /// URL-safe unpadded base64 representation of the shared 32-byte group key.
    pub group_key: String,
}

impl PairingInfo {
    /// Creates a new `PairingInfo` instance.
    pub fn new(relay_url: impl Into<String>, group_key: &GroupKey) -> Self {
        Self {
            v: PAIRING_VERSION,
            relay_url: relay_url.into(),
            group_key: group_key.to_base64url(),
        }
    }

    /// Serializes pairing information to a deep link URI (`clipbridge://pair?d=...`).
    pub fn to_uri(&self) -> String {
        let json_bytes = serde_json::to_vec(self).unwrap_or_default();
        let encoded = URL_SAFE_NO_PAD.encode(json_bytes);
        format!("{PAIRING_SCHEME}://{PAIRING_HOST}?d={encoded}")
    }

    /// Parses and validates pairing info from a URI string.
    ///
    /// Validates the scheme, target host, payload version, relay protocol scheme (`ws://` or `wss://`),
    /// and ensures the group key is well-formed.
    pub fn from_uri(uri_str: &str) -> Result<Self> {
        let parsed = url::Url::parse(uri_str)
            .map_err(|e| CoreError::InvalidPairingUri(format!("malformed URL: {e}")))?;

        if parsed.scheme() != PAIRING_SCHEME {
            return Err(CoreError::InvalidPairingUri(format!(
                "invalid scheme '{}', expected '{PAIRING_SCHEME}'",
                parsed.scheme()
            )));
        }

        if parsed.host_str() != Some(PAIRING_HOST) {
            return Err(CoreError::InvalidPairingUri(format!(
                "invalid host '{:?}', expected '{PAIRING_HOST}'",
                parsed.host_str()
            )));
        }

        let encoded = parsed
            .query_pairs()
            .find(|(k, _)| k == "d")
            .map(|(_, v)| v.into_owned())
            .ok_or_else(|| {
                CoreError::InvalidPairingUri("missing 'd' query parameter in pairing URI".into())
            })?;

        let json_bytes = URL_SAFE_NO_PAD.decode(encoded.as_bytes()).map_err(|e| {
            CoreError::InvalidPairingUri(format!("invalid base64url data parameter: {e}"))
        })?;

        let info: PairingInfo = serde_json::from_slice(&json_bytes)
            .map_err(|e| CoreError::InvalidPairingUri(format!("invalid JSON in pairing data: {e}")))?;

        if info.v != PAIRING_VERSION {
            return Err(CoreError::InvalidPairingUri(format!(
                "unsupported pairing version: {}",
                info.v
            )));
        }

        if !info.relay_url.starts_with("ws://") && !info.relay_url.starts_with("wss://") {
            return Err(CoreError::InvalidPairingUri(format!(
                "relay_url must start with ws:// or wss://, got '{}'",
                info.relay_url
            )));
        }

        // Validate key format immediately
        let _ = info.group_key()?;

        Ok(info)
    }

    /// Decodes the `GroupKey` stored in this pairing configuration.
    pub fn group_key(&self) -> Result<GroupKey> {
        GroupKey::from_base64url(&self.group_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pairing_uri_roundtrip() {
        let key = GroupKey::generate();
        let info = PairingInfo::new("wss://relay.clipbridge.dev:443", &key);
        let uri = info.to_uri();

        assert!(uri.starts_with("clipbridge://pair?d="));

        let parsed = PairingInfo::from_uri(&uri).unwrap();
        assert_eq!(parsed, info);
        assert_eq!(parsed.group_key().unwrap(), key);
    }

    #[test]
    fn test_invalid_scheme() {
        let uri = "https://pair?d=ey==";
        let err = PairingInfo::from_uri(uri).unwrap_err();
        match err {
            CoreError::InvalidPairingUri(msg) => assert!(msg.contains("invalid scheme")),
            other => panic!("expected InvalidPairingUri, got: {other:?}"),
        }
    }

    #[test]
    fn test_missing_data_query() {
        let uri = "clipbridge://pair";
        let err = PairingInfo::from_uri(uri).unwrap_err();
        match err {
            CoreError::InvalidPairingUri(msg) => assert!(msg.contains("missing 'd'")),
            other => panic!("expected InvalidPairingUri, got: {other:?}"),
        }
    }

    #[test]
    fn test_invalid_relay_protocol() {
        let key = GroupKey::generate();
        let mut info = PairingInfo::new("http://relay.clipbridge.dev", &key);
        // Force an invalid relay URL into the serialized payload
        info.relay_url = "http://insecure.dev".to_string();
        let uri = info.to_uri();

        let err = PairingInfo::from_uri(&uri).unwrap_err();
        match err {
            CoreError::InvalidPairingUri(msg) => assert!(msg.contains("ws:// or wss://")),
            other => panic!("expected InvalidPairingUri, got: {other:?}"),
        }
    }
}
