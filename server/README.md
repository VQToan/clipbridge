# ClipBridge Relay Server

A high-performance, zero-knowledge Go relay server for **ClipBridge** clipboard synchronization.

The relay routes and temporarily stores end-to-end encrypted envelopes (protocol v1). It never inspects clipboard contents, MIME types, or plaintext payloads. Authentication uses Trust-On-First-Use (TOFU) based on `SHA-256(relay_auth_token)`.

## Quick Start

### Run Locally (In-Memory Mode)

```bash
# Starts the relay using an in-memory store on :8080
go run ./cmd/relay
```

### Run with Redis via Docker Compose

From the repository root:

```bash
# Starts Redis and the relay server with healthchecks
docker compose up --build
```

### Running Tests

```bash
# Run all tests with Go race detector enabled
go test -race ./...

# Or using the Makefile
make test
```

## Configuration

The server is configured via environment variables:

| Variable | Type | Default | Description |
|---|---|---|---|
| `CLIPBRIDGE_ADDR` | string | `:8080` | TCP host:port to bind the HTTP / WebSocket server |
| `CLIPBRIDGE_REDIS_URL` | string | `""` | Redis connection URL (e.g. `redis://localhost:6379/0`). If empty, uses an in-memory store (for local dev/tests; data lost on restart) |
| `CLIPBRIDGE_MAILBOX_SIZE` | int | `50` | Maximum number of envelopes stored in the mailbox per group |
| `CLIPBRIDGE_MAILBOX_TTL` | duration | `24h` | Expiration TTL for stored mailbox envelopes (e.g. `24h`, `1h`) |
| `CLIPBRIDGE_MAX_CONN_PER_GROUP` | int | `32` | Maximum concurrent active WebSocket connections per group |
| `CLIPBRIDGE_PUBLISH_RATE` | float | `5.0` | Sustained publish rate limit per connection (envelopes/second) |
| `CLIPBRIDGE_PUBLISH_BURST` | int | `20` | Maximum burst capacity for publishing envelopes per connection |
| `CLIPBRIDGE_LOG_LEVEL` | string | `info` | Structured log level (`debug`, `info`, `warn`, `error`) |

## HTTP & WebSocket Endpoints

- `GET /healthz`: Service health check. Returns `200 ok` when healthy; returns `503 unhealthy` if Redis ping fails.
- `GET /v1/ws`: WebSocket upgrade endpoint.
  - Required headers:
    - `Authorization: Bearer <token>`
    - `X-ClipBridge-Group: <64 hex characters>`
    - `X-ClipBridge-Device: <device-id>`
  - Optional query parameter:
    - `since=<ULID>` (replays envelopes received after the given ULID)
