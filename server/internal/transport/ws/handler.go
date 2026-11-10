// Package ws exposes the relay over WebSocket (protocol v1, see docs/protocol.md).
package ws

import (
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/oklog/ulid/v2"
	"golang.org/x/time/rate"

	"github.com/toanvo/clipbridge/server/internal/domain"
	"github.com/toanvo/clipbridge/server/internal/service"
)

const (
	headerGroup  = "X-ClipBridge-Group"
	headerDevice = "X-ClipBridge-Device"

	sendBufferSize = 64
	writeTimeout   = 10 * time.Second
	pingInterval   = 30 * time.Second
	authTimeout    = 5 * time.Second

	frameTypePublish = "publish"
	frameTypeDeliver = "deliver"
	frameTypeAck     = "ack"
	frameTypeError   = "error"
)

// Frame is the JSON message exchanged over the socket.
type Frame struct {
	Type     string           `json:"type"`
	Envelope *domain.Envelope `json:"envelope,omitempty"`
	ID       string           `json:"id,omitempty"`
	Code     string           `json:"code,omitempty"`
	Message  string           `json:"message,omitempty"`
}

// Handler upgrades HTTP requests to relay sessions.
type Handler struct {
	relay        *service.Relay
	log          *slog.Logger
	publishRate  rate.Limit
	publishBurst int
}

// NewHandler builds a WebSocket handler.
func NewHandler(relay *service.Relay, log *slog.Logger, publishRate float64, publishBurst int) *Handler {
	return &Handler{relay: relay, log: log, publishRate: rate.Limit(publishRate), publishBurst: publishBurst}
}

// ServeHTTP implements http.Handler.
func (h *Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	groupID := r.Header.Get(headerGroup)
	deviceID := r.Header.Get(headerDevice)
	token, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
	if !ok {
		http.Error(w, "missing bearer token", http.StatusUnauthorized)
		return
	}
	since := r.URL.Query().Get("since")
	if since != "" {
		if _, err := ulid.ParseStrict(since); err != nil {
			http.Error(w, "since must be a ULID", http.StatusBadRequest)
			return
		}
	}

	authCtx, cancelAuth := context.WithTimeout(r.Context(), authTimeout)
	err := h.relay.Authenticate(authCtx, groupID, deviceID, token)
	cancelAuth()
	if err != nil {
		if errors.Is(err, domain.ErrUnauthorized) {
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		h.log.ErrorContext(r.Context(), "authenticate failed", "err", err)
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}

	conn, err := websocket.Accept(w, r, nil)
	if err != nil {
		h.log.WarnContext(r.Context(), "websocket accept failed", "err", err)
		return
	}
	conn.SetReadLimit(domain.MaxFrameBytes)

	ctx, cancel := context.WithCancel(context.Background())
	sess := &session{
		conn:     conn,
		deviceID: deviceID,
		send:     make(chan []byte, sendBufferSize),
		ctx:      ctx,
		cancel:   cancel,
		log:      h.log.With("device", deviceID, "group", groupID[:8]),
	}
	defer sess.Close("session ended")

	if err := h.relay.Subscribe(groupID, sess); err != nil {
		conn.Close(websocket.StatusPolicyViolation, err.Error())
		return
	}
	defer h.relay.Unsubscribe(groupID, sess)

	go sess.writeLoop()

	// Subscribing before replay may duplicate an envelope; clients dedupe by envelope ID.
	backlog, err := h.relay.Backlog(ctx, groupID, deviceID, since)
	if err != nil {
		sess.log.Error("load backlog failed", "err", err)
	}
	for i := range backlog {
		if !sess.enqueueBlocking(Frame{Type: frameTypeDeliver, Envelope: &backlog[i]}) {
			return
		}
	}
	sess.log.Info("session started", "backlog", len(backlog))

	h.readLoop(sess, groupID)
}

func (h *Handler) readLoop(sess *session, groupID string) {
	limiter := rate.NewLimiter(h.publishRate, h.publishBurst)
	for {
		typ, data, err := sess.conn.Read(sess.ctx)
		if err != nil {
			status := websocket.CloseStatus(err)
			if status != websocket.StatusNormalClosure && status != websocket.StatusGoingAway && sess.ctx.Err() == nil {
				sess.log.Info("read ended", "err", err)
			}
			return
		}
		if typ != websocket.MessageText {
			sess.enqueue(errorFrame("invalid_frame", "only text frames are supported", ""))
			continue
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil || f.Type != frameTypePublish || f.Envelope == nil {
			sess.enqueue(errorFrame("invalid_frame", "expected publish frame with envelope", ""))
			continue
		}
		if !limiter.Allow() {
			sess.enqueue(errorFrame("rate_limited", "publish rate exceeded", f.Envelope.ID))
			continue
		}
		if err := h.relay.Publish(sess.ctx, groupID, sess.deviceID, *f.Envelope); err != nil {
			if service.IsClientError(err) {
				sess.enqueue(errorFrame("invalid_envelope", err.Error(), f.Envelope.ID))
				continue
			}
			sess.log.Error("publish failed", "envelope", f.Envelope.ID, "err", err)
			sess.enqueue(errorFrame("internal", "publish failed", f.Envelope.ID))
			continue
		}
		sess.enqueue(Frame{Type: frameTypeAck, ID: f.Envelope.ID})
	}
}

func errorFrame(code, msg, id string) Frame {
	return Frame{Type: frameTypeError, Code: code, Message: msg, ID: id}
}

// session is a single WebSocket connection; it implements service.Subscriber.
type session struct {
	conn      *websocket.Conn
	deviceID  string
	send      chan []byte
	ctx       context.Context
	cancel    context.CancelFunc
	log       *slog.Logger
	closeOnce sync.Once
}

func (s *session) DeviceID() string { return s.deviceID }

func (s *session) Deliver(env domain.Envelope) bool {
	return s.enqueue(Frame{Type: frameTypeDeliver, Envelope: &env})
}

func (s *session) Close(reason string) {
	s.closeOnce.Do(func() {
		s.cancel()
		s.conn.Close(websocket.StatusNormalClosure, reason)
	})
}

// enqueue is non-blocking so one slow device can never stall fan-out to others.
func (s *session) enqueue(f Frame) bool {
	raw, err := json.Marshal(f)
	if err != nil {
		s.log.Error("marshal frame failed", "err", err)
		return false
	}
	select {
	case s.send <- raw:
		return true
	case <-s.ctx.Done():
		return false
	default:
		return false
	}
}

// enqueueBlocking is used for backlog replay where back-pressure is acceptable.
func (s *session) enqueueBlocking(f Frame) bool {
	raw, err := json.Marshal(f)
	if err != nil {
		s.log.Error("marshal frame failed", "err", err)
		return false
	}
	select {
	case s.send <- raw:
		return true
	case <-s.ctx.Done():
		return false
	}
}

func (s *session) writeLoop() {
	ticker := time.NewTicker(pingInterval)
	defer ticker.Stop()
	defer s.Close("writer stopped")
	for {
		select {
		case <-s.ctx.Done():
			return
		case raw := <-s.send:
			wctx, cancel := context.WithTimeout(s.ctx, writeTimeout)
			err := s.conn.Write(wctx, websocket.MessageText, raw)
			cancel()
			if err != nil {
				s.log.Info("write failed", "err", err)
				return
			}
		case <-ticker.C:
			pctx, cancel := context.WithTimeout(s.ctx, writeTimeout)
			err := s.conn.Ping(pctx)
			cancel()
			if err != nil {
				s.log.Info("ping failed", "err", err)
				return
			}
		}
	}
}
