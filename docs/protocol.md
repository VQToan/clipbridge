# ClipBridge Relay Protocol — v1

The relay is **zero-knowledge**: it routes and temporarily stores encrypted envelopes. It never sees the group key, clipboard content, MIME type or size of the plaintext (only ciphertext length).

## 1. Key derivation (client side, `clip-core`)

All keys are derived from a random 32-byte **group key** shared during pairing, using BLAKE3 `derive_key`:

| Purpose | Context string | Encoding sent to server |
|---|---|---|
| Group ID | `clipbridge 2026-11 group-id v1` | 64 lowercase hex chars |
| Relay auth token | `clipbridge 2026-11 relay-auth v1` | base64url (no padding), 43 chars |
| Envelope encryption key | `clipbridge 2026-11 envelope-enc v1` | never leaves device |

The server stores only `SHA-256(token)` per group. The first connection for an unknown group registers it (trust-on-first-use); every later connection must present the same token.

## 2. Connection

```
GET /v1/ws?since=<ULID, optional>
Authorization: Bearer <relay auth token>
X-ClipBridge-Group: <group id>
X-ClipBridge-Device: <device id, 1-64 chars [A-Za-z0-9_-]>
```

After upgrade the server immediately replays mailbox envelopes for the group that are newer than `since` and were not sent by this device.

## 3. Frames (JSON text messages)

Client → Server

```json
{ "type": "publish", "envelope": { ...Envelope } }
```

Server → Client

```json
{ "type": "deliver", "envelope": { ...Envelope } }
{ "type": "ack", "id": "<envelope id>" }
{ "type": "error", "code": "invalid_envelope", "message": "...", "id": "<optional>" }
```

Error codes: `invalid_frame`, `invalid_envelope`, `rate_limited`, `internal`.

## 4. Envelope

```json
{
  "v": 1,
  "id": "01JC...ULID",
  "group_id": "<64 hex>",
  "sender": "<device id>",
  "created_at": 1762750000000,
  "nonce": "<base64, 24 bytes>",
  "ciphertext": "<base64>"
}
```

- Cipher: **XChaCha20-Poly1305**, key = envelope encryption key.
- AAD: `"cb1|" + id + "|" + group_id + "|" + sender + "|" + created_at` — binds header fields to ciphertext, so the relay cannot re-route or re-label envelopes undetected.
- Plaintext: CBOR-encoded `ClipPayload { kind, mime, data, hash }` (see `clip-core/src/model.rs`).

## 5. Limits

| Limit | Value |
|---|---|
| Max plaintext payload | 10 MiB |
| Max WebSocket frame | 16 MiB |
| Mailbox per group | 50 envelopes, TTL 24h (configurable) |
| Publish rate per connection | 5/s, burst 20 |
| Connections per group | 32 |
