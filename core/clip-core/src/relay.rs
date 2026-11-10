use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::envelope::Envelope;
use crate::error::{CoreError, Result};
use crate::keys::GroupKey;

/// Maximum WebSocket message size (16 MiB) per `protocol.md` section 5.
pub const MAX_WS_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Configuration for connecting to the ClipBridge WebSocket relay hub.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// WebSocket base endpoint (`ws://...` or `wss://...`).
    pub base_url: String,
    /// Identifier of the local device (`^[A-Za-z0-9_-]{1,64}$`).
    pub device_id: String,
    /// Shared symmetric group key.
    pub key: GroupKey,
    /// Optional ULID to request missed envelopes since this ID.
    pub since: Option<String>,
}

/// Events delivered by the relay over the WebSocket connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEvent {
    /// Inbound envelope published by another device in the group.
    Deliver(Envelope),
    /// Server acknowledgement that a published envelope was accepted.
    Ack(String),
    /// Server error notification.
    Error {
        /// Machine-readable error code (e.g. `invalid_frame`, `rate_limited`).
        code: String,
        /// Human-readable explanation.
        message: String,
        /// Envelope ID associated with the error, if any.
        id: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum OutgoingFrame<'a> {
    Publish { envelope: &'a Envelope },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum IncomingFrame {
    Deliver {
        envelope: Envelope,
    },
    Ack {
        id: String,
    },
    Error {
        code: String,
        message: String,
        #[serde(default)]
        id: Option<String>,
    },
}

type Stream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Sink = futures_util::stream::SplitSink<Stream, Message>;
type Source = futures_util::stream::SplitStream<Stream>;

/// The sending half of a split relay connection.
pub struct RelaySender {
    sink: Sink,
}

impl RelaySender {
    /// Publishes an encrypted envelope to the group via the relay.
    pub async fn publish(&mut self, env: &Envelope) -> Result<()> {
        let frame = OutgoingFrame::Publish { envelope: env };
        let json = serde_json::to_string(&frame)
            .map_err(|e| CoreError::Serialization(e.to_string()))?;

        self.sink
            .send(Message::Text(json.into()))
            .await
            .map_err(|e| CoreError::Relay(format!("failed to send publish frame: {e}")))?;

        Ok(())
    }
}

/// The receiving half of a split relay connection.
pub struct RelayReceiver {
    source: Source,
}

impl RelayReceiver {
    /// Awaits the next event from the relay server.
    ///
    /// Automatically replies to WebSocket pings. Returns `Ok(None)` when the connection is closed.
    pub async fn next_event(&mut self) -> Result<Option<RelayEvent>> {
        while let Some(msg_result) = self.source.next().await {
            let msg = msg_result
                .map_err(|e| CoreError::Relay(format!("websocket read error: {e}")))?;

            match msg {
                Message::Text(text) => {
                    let frame: IncomingFrame = serde_json::from_str(&text)
                        .map_err(|e| CoreError::Decode(format!("failed to parse frame JSON: {e}")))?;

                    let event = match frame {
                        IncomingFrame::Deliver { envelope } => RelayEvent::Deliver(envelope),
                        IncomingFrame::Ack { id } => RelayEvent::Ack(id),
                        IncomingFrame::Error { code, message, id } => {
                            RelayEvent::Error { code, message, id }
                        }
                    };
                    return Ok(Some(event));
                }
                Message::Close(_) => {
                    return Ok(None);
                }
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {
                    // Tungstenite automatically responds to pings with pongs.
                    // Binary or extraneous frames are ignored per protocol specification.
                    continue;
                }
            }
        }

        Ok(None)
    }
}

/// Active WebSocket connection to the ClipBridge relay server.
pub struct RelayConnection {
    sender: RelaySender,
    receiver: RelayReceiver,
}

impl RelayConnection {
    /// Connects and authenticates to the relay server.
    ///
    /// Formats the target URL with optional `since` query parameter and sets
    /// required handshake headers (`Authorization`, `X-ClipBridge-Group`, `X-ClipBridge-Device`).
    pub async fn connect(cfg: &RelayConfig) -> Result<Self> {
        let base = cfg.base_url.trim_end_matches('/');
        let url_str = match &cfg.since {
            Some(since) => format!("{base}/v1/ws?since={since}"),
            None => format!("{base}/v1/ws"),
        };

        let mut request = url_str
            .into_client_request()
            .map_err(|e| CoreError::Relay(format!("invalid request URL: {e}")))?;

        let headers = request.headers_mut();
        headers.insert(
            "Authorization",
            format!("Bearer {}", cfg.key.relay_token())
                .parse()
                .map_err(|e| CoreError::Relay(format!("invalid Authorization header: {e}")))?,
        );
        headers.insert(
            "X-ClipBridge-Group",
            cfg.key
                .group_id()
                .parse()
                .map_err(|e| CoreError::Relay(format!("invalid X-ClipBridge-Group header: {e}")))?,
        );
        headers.insert(
            "X-ClipBridge-Device",
            cfg.device_id
                .parse()
                .map_err(|e| CoreError::Relay(format!("invalid X-ClipBridge-Device header: {e}")))?,
        );

        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(MAX_WS_FRAME_SIZE);
        ws_config.max_frame_size = Some(MAX_WS_FRAME_SIZE);

        let (stream, _resp) = tokio_tungstenite::connect_async_with_config(
            request,
            Some(ws_config),
            false,
        )
        .await
        .map_err(|e| CoreError::Relay(format!("failed to connect to relay: {e}")))?;

        let (sink, source) = stream.split();

        Ok(Self {
            sender: RelaySender { sink },
            receiver: RelayReceiver { source },
        })
    }

    /// Publishes an envelope over this connection.
    pub async fn publish(&mut self, env: &Envelope) -> Result<()> {
        self.sender.publish(env).await
    }

    /// Waits for the next event from the relay server.
    pub async fn next_event(&mut self) -> Result<Option<RelayEvent>> {
        self.receiver.next_event().await
    }

    /// Splits the connection into independent sender and receiver halves.
    pub fn split(self) -> (RelaySender, RelayReceiver) {
        (self.sender, self.receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipPayload;
    use tokio::net::TcpListener;

    #[test]
    fn test_incoming_frame_deserialization() {
        let ack_json = r#"{"type":"ack","id":"01JC0000000000000000000001"}"#;
        let frame: IncomingFrame = serde_json::from_str(ack_json).unwrap();
        match frame {
            IncomingFrame::Ack { id } => assert_eq!(id, "01JC0000000000000000000001"),
            _ => panic!("expected Ack"),
        }

        let err_json = r#"{"type":"error","code":"rate_limited","message":"slow down","id":"01JC0000000000000000000001"}"#;
        let frame: IncomingFrame = serde_json::from_str(err_json).unwrap();
        match frame {
            IncomingFrame::Error { code, message, id } => {
                assert_eq!(code, "rate_limited");
                assert_eq!(message, "slow down");
                assert_eq!(id, Some("01JC0000000000000000000001".to_string()));
            }
            _ => panic!("expected Error"),
        }
    }

    #[tokio::test]
    async fn test_relay_connection_mock_roundtrip() {
        // Start a mock WebSocket relay server on a random local port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

            // Read the publish frame sent by the client
            let msg = ws.next().await.unwrap().unwrap();
            let text = msg.into_text().unwrap();
            assert!(text.contains("\"type\":\"publish\""));

            // Reply with an ack
            ws.send(Message::Text(r#"{"type":"ack","id":"test-id-123"}"#.into()))
                .await
                .unwrap();
        });

        let key = GroupKey::generate();
        let cfg = RelayConfig {
            base_url: format!("ws://{addr}"),
            device_id: "test-device".to_string(),
            key: key.clone(),
            since: None,
        };

        let mut conn = RelayConnection::connect(&cfg).await.unwrap();

        let payload = ClipPayload::text("test relay transfer").unwrap();
        let env = Envelope::seal(&key, "test-device", &payload).unwrap();

        conn.publish(&env).await.unwrap();

        let event = conn.next_event().await.unwrap().expect("expected event");
        match event {
            RelayEvent::Ack(id) => assert_eq!(id, "test-id-123"),
            other => panic!("expected Ack, got: {other:?}"),
        }

        server_task.await.unwrap();
    }
}
