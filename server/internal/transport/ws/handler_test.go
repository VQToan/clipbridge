package ws_test

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/oklog/ulid/v2"

	"github.com/toanvo/clipbridge/server/internal/domain"
	"github.com/toanvo/clipbridge/server/internal/repository/memory"
	"github.com/toanvo/clipbridge/server/internal/service"
	"github.com/toanvo/clipbridge/server/internal/transport/httpapi"
	"github.com/toanvo/clipbridge/server/internal/transport/ws"
)

const (
	group = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	token = "secret-token"
)

func newServer(t *testing.T) *httptest.Server {
	t.Helper()
	log := slog.New(slog.NewTextHandler(io.Discard, nil))
	store := memory.New(10)
	relay := service.NewRelay(store, store, service.Options{MailboxTTL: time.Hour, MaxConnPerGroup: 4}, log)
	srv := httptest.NewServer(httpapi.NewRouter(ws.NewHandler(relay, log, 100, 100), nil, log))
	t.Cleanup(srv.Close)
	return srv
}

func dial(t *testing.T, srv *httptest.Server, device, tok, since string) (*websocket.Conn, *http.Response, error) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	u := "ws" + strings.TrimPrefix(srv.URL, "http") + "/v1/ws"
	if since != "" {
		u += "?since=" + since
	}
	h := http.Header{}
	h.Set("Authorization", "Bearer "+tok)
	h.Set("X-ClipBridge-Group", group)
	h.Set("X-ClipBridge-Device", device)
	return websocket.Dial(ctx, u, &websocket.DialOptions{HTTPHeader: h})
}

func mustDial(t *testing.T, srv *httptest.Server, device, since string) *websocket.Conn {
	t.Helper()
	c, _, err := dial(t, srv, device, token, since)
	if err != nil {
		t.Fatalf("dial %s: %v", device, err)
	}
	t.Cleanup(func() { c.CloseNow() })
	return c
}

func envelope(sender string) domain.Envelope {
	return domain.Envelope{
		V: 1, ID: ulid.Make().String(), GroupID: group, Sender: sender, CreatedAt: time.Now().UnixMilli(),
		Nonce:      base64.StdEncoding.EncodeToString(make([]byte, 24)),
		Ciphertext: base64.StdEncoding.EncodeToString([]byte("opaque")),
	}
}

func send(t *testing.T, c *websocket.Conn, f ws.Frame) {
	t.Helper()
	raw, _ := json.Marshal(f)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := c.Write(ctx, websocket.MessageText, raw); err != nil {
		t.Fatal(err)
	}
}

func recv(t *testing.T, c *websocket.Conn) ws.Frame {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	_, raw, err := c.Read(ctx)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	var f ws.Frame
	if err := json.Unmarshal(raw, &f); err != nil {
		t.Fatal(err)
	}
	return f
}

func TestPublishFanOutAndAck(t *testing.T) {
	srv := newServer(t)
	a := mustDial(t, srv, "dev-a", "")
	b := mustDial(t, srv, "dev-b", "")

	env := envelope("dev-a")
	send(t, a, ws.Frame{Type: "publish", Envelope: &env})

	if ack := recv(t, a); ack.Type != "ack" || ack.ID != env.ID {
		t.Fatalf("expected ack for %s, got %+v", env.ID, ack)
	}
	got := recv(t, b)
	if got.Type != "deliver" || got.Envelope == nil || got.Envelope.ID != env.ID {
		t.Fatalf("expected deliver, got %+v", got)
	}
}

func TestBacklogReplayRespectsSinceAndSender(t *testing.T) {
	srv := newServer(t)
	a := mustDial(t, srv, "dev-a", "")
	first, second := envelope("dev-a"), envelope("dev-a")
	for _, e := range []domain.Envelope{first, second} {
		send(t, a, ws.Frame{Type: "publish", Envelope: &e})
		recv(t, a) // ack
	}

	b := mustDial(t, srv, "dev-b", first.ID)
	got := recv(t, b)
	if got.Envelope == nil || got.Envelope.ID != second.ID {
		t.Fatalf("expected only second envelope, got %+v", got)
	}

	// Sender must not receive its own envelopes on reconnect.
	a2 := mustDial(t, srv, "dev-a", "")
	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()
	if _, _, err := a2.Read(ctx); err == nil {
		t.Fatal("sender received its own backlog")
	}
}

func TestRejectsSpoofedSenderAndWrongToken(t *testing.T) {
	srv := newServer(t)
	a := mustDial(t, srv, "dev-a", "")
	spoof := envelope("dev-b")
	send(t, a, ws.Frame{Type: "publish", Envelope: &spoof})
	if f := recv(t, a); f.Type != "error" || f.Code != "invalid_envelope" {
		t.Fatalf("expected invalid_envelope, got %+v", f)
	}

	_, resp, err := dial(t, srv, "dev-x", "wrong-token", "")
	if err == nil || resp == nil || resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("expected 401, got err=%v resp=%v", err, resp)
	}
}
