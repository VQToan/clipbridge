package service_test

import (
	"context"
	"encoding/base64"
	"errors"
	"io"
	"log/slog"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/oklog/ulid/v2"

	"github.com/toanvo/clipbridge/server/internal/domain"
	"github.com/toanvo/clipbridge/server/internal/repository/memory"
	"github.com/toanvo/clipbridge/server/internal/service"
)

const (
	testGroupA = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	testGroupB = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	testDeviceA = "device-a"
	testDeviceB = "device-b"
	testDeviceC = "device-c"
	testToken   = "valid-token-secret"
)

type mockSubscriber struct {
	mu          sync.Mutex
	id          string
	deliverRet  bool
	delivered   []domain.Envelope
	closed      bool
	closeReason string
}

func newMockSubscriber(id string, deliverRet bool) *mockSubscriber {
	return &mockSubscriber{id: id, deliverRet: deliverRet}
}

func (s *mockSubscriber) DeviceID() string {
	return s.id
}

func (s *mockSubscriber) Deliver(env domain.Envelope) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.delivered = append(s.delivered, env)
	return s.deliverRet
}

func (s *mockSubscriber) Close(reason string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.closed = true
	s.closeReason = reason
}

func (s *mockSubscriber) isClosed() (bool, string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.closed, s.closeReason
}

func (s *mockSubscriber) getDelivered() []domain.Envelope {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]domain.Envelope(nil), s.delivered...)
}

func testEnvelope(groupID, sender string) domain.Envelope {
	return domain.Envelope{
		V:          domain.ProtocolVersion,
		ID:         ulid.Make().String(),
		GroupID:    groupID,
		Sender:     sender,
		CreatedAt:  time.Now().UnixMilli(),
		Nonce:      base64.StdEncoding.EncodeToString(make([]byte, 24)),
		Ciphertext: base64.StdEncoding.EncodeToString([]byte("test-ciphertext")),
	}
}

func newTestRelay(t *testing.T, opts service.Options) (*service.Relay, *memory.Store) {
	t.Helper()
	log := slog.New(slog.NewTextHandler(io.Discard, nil))
	store := memory.New(50)
	return service.NewRelay(store, store, opts, log), store
}

func TestTOFUAuthentication(t *testing.T) {
	ctx := context.Background()
	r, _ := newTestRelay(t, service.Options{
		MailboxTTL:      time.Hour,
		MaxConnPerGroup: 32,
	})

	// 1. Initial connection with unknown group registers token (TOFU).
	if err := r.Authenticate(ctx, testGroupA, testDeviceA, testToken); err != nil {
		t.Fatalf("first registration failed: %v", err)
	}

	// 2. Subsequent connection presenting the exact same token succeeds.
	if err := r.Authenticate(ctx, testGroupA, testDeviceB, testToken); err != nil {
		t.Fatalf("authentication with same token failed: %v", err)
	}

	// 3. Subsequent connection presenting a different token is rejected with ErrUnauthorized.
	err := r.Authenticate(ctx, testGroupA, testDeviceC, "wrong-token")
	if !errors.Is(err, domain.ErrUnauthorized) {
		t.Fatalf("expected ErrUnauthorized for wrong token, got: %v", err)
	}

	// 4. Invalid group ID formats rejected.
	invalidGroups := []string{
		"",
		"not-hex",
		strings.Repeat("a", 63), // too short
		strings.Repeat("a", 65), // too long
		"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", // uppercase
	}
	for _, g := range invalidGroups {
		if err := r.Authenticate(ctx, g, testDeviceA, testToken); !errors.Is(err, domain.ErrUnauthorized) {
			t.Errorf("expected ErrUnauthorized for invalid group %q, got: %v", g, err)
		}
	}

	// 5. Invalid device ID formats rejected.
	invalidDevices := []string{
		"",
		strings.Repeat("d", 65), // max length is 64
		"device with spaces",
		"device@invalid#char!",
	}
	for _, d := range invalidDevices {
		if err := r.Authenticate(ctx, testGroupA, d, testToken); !errors.Is(err, domain.ErrUnauthorized) {
			t.Errorf("expected ErrUnauthorized for invalid device %q, got: %v", d, err)
		}
	}

	// 6. Empty token rejected.
	if err := r.Authenticate(ctx, testGroupA, testDeviceA, ""); !errors.Is(err, domain.ErrUnauthorized) {
		t.Fatalf("expected ErrUnauthorized for empty token, got: %v", err)
	}

	// 7. A separate group can register independently with its own token.
	if err := r.Authenticate(ctx, testGroupB, testDeviceA, "token-for-b"); err != nil {
		t.Fatalf("registration of separate group failed: %v", err)
	}
	if err := r.Authenticate(ctx, testGroupB, testDeviceB, testToken); !errors.Is(err, domain.ErrUnauthorized) {
		t.Fatalf("expected ErrUnauthorized for group B with group A token, got: %v", err)
	}
}

func TestMaxConnPerGroup(t *testing.T) {
	r, _ := newTestRelay(t, service.Options{
		MailboxTTL:      time.Hour,
		MaxConnPerGroup: 2,
	})

	sub1 := newMockSubscriber(testDeviceA, true)
	sub2 := newMockSubscriber(testDeviceB, true)
	sub3 := newMockSubscriber(testDeviceC, true)

	// Subscribe up to limit (2).
	if err := r.Subscribe(testGroupA, sub1); err != nil {
		t.Fatalf("subscribe 1 failed: %v", err)
	}
	if err := r.Subscribe(testGroupA, sub2); err != nil {
		t.Fatalf("subscribe 2 failed: %v", err)
	}

	// 3rd subscriber exceeds limit.
	err := r.Subscribe(testGroupA, sub3)
	if !errors.Is(err, domain.ErrGroupFull) {
		t.Fatalf("expected ErrGroupFull, got: %v", err)
	}

	// Unsubscribe one member, then re-try subscriber 3.
	r.Unsubscribe(testGroupA, sub1)
	if err := r.Subscribe(testGroupA, sub3); err != nil {
		t.Fatalf("subscribe 3 after slot freed failed: %v", err)
	}

	// Unsubscribe remaining members until group map entry cleans up.
	r.Unsubscribe(testGroupA, sub2)
	r.Unsubscribe(testGroupA, sub3)

	// Unsubscribing non-existent subscriber is a no-op and does not panic.
	r.Unsubscribe(testGroupA, sub1)
	r.Unsubscribe("non-existent-group", sub1)
}

func TestBacklogTTLFiltering(t *testing.T) {
	ctx := context.Background()
	r, _ := newTestRelay(t, service.Options{
		MailboxTTL:      1 * time.Hour,
		MaxConnPerGroup: 10,
	})

	now := time.Date(2026, 11, 10, 10, 0, 0, 0, time.UTC)
	r.SetNowForTest(func() time.Time { return now })

	// Env 1 published at 10:00.
	env1 := testEnvelope(testGroupA, testDeviceA)
	if err := r.Publish(ctx, testGroupA, testDeviceA, env1); err != nil {
		t.Fatalf("publish env1 failed: %v", err)
	}

	// Advance time by 30 minutes to 10:30.
	now = now.Add(30 * time.Minute)
	env2 := testEnvelope(testGroupA, testDeviceA)
	if err := r.Publish(ctx, testGroupA, testDeviceA, env2); err != nil {
		t.Fatalf("publish env2 failed: %v", err)
	}

	// Advance time by 45 minutes to 11:15 (TTL is 1h, so cutoff is 10:15).
	// env1 (received 10:00) is now expired (< 10:15).
	// env2 (received 10:30) is still valid (> 10:15).
	now = now.Add(45 * time.Minute)
	env3 := testEnvelope(testGroupA, testDeviceA)
	if err := r.Publish(ctx, testGroupA, testDeviceA, env3); err != nil {
		t.Fatalf("publish env3 failed: %v", err)
	}

	// Env 4 published by device-b at 11:15.
	env4 := testEnvelope(testGroupA, testDeviceB)
	if err := r.Publish(ctx, testGroupA, testDeviceB, env4); err != nil {
		t.Fatalf("publish env4 failed: %v", err)
	}

	// Request backlog for device-b without since ID.
	// Expected:
	// - env1: omitted (expired TTL)
	// - env2: included
	// - env3: included
	// - env4: omitted (sent by device-b itself)
	backlog, err := r.Backlog(ctx, testGroupA, testDeviceB, "")
	if err != nil {
		t.Fatalf("Backlog failed: %v", err)
	}
	if len(backlog) != 2 {
		t.Fatalf("expected 2 envelopes in backlog, got %d: %+v", len(backlog), backlog)
	}
	if backlog[0].ID != env2.ID || backlog[1].ID != env3.ID {
		t.Fatalf("expected [env2, env3], got [%s, %s]", backlog[0].ID, backlog[1].ID)
	}

	// Request backlog for device-b with since = env2.ID.
	// Expected: only env3.
	backlogSince, err := r.Backlog(ctx, testGroupA, testDeviceB, env2.ID)
	if err != nil {
		t.Fatalf("Backlog with since failed: %v", err)
	}
	if len(backlogSince) != 1 || backlogSince[0].ID != env3.ID {
		t.Fatalf("expected [env3], got %+v", backlogSince)
	}
}

func TestSlowSubscriberGetsClosed(t *testing.T) {
	ctx := context.Background()
	r, _ := newTestRelay(t, service.Options{
		MailboxTTL:      time.Hour,
		MaxConnPerGroup: 10,
	})

	fastSub := newMockSubscriber(testDeviceB, true)
	slowSub := newMockSubscriber(testDeviceC, false) // Deliver returns false (buffer full)
	senderSub := newMockSubscriber(testDeviceA, true)

	if err := r.Subscribe(testGroupA, fastSub); err != nil {
		t.Fatalf("subscribe fastSub: %v", err)
	}
	if err := r.Subscribe(testGroupA, slowSub); err != nil {
		t.Fatalf("subscribe slowSub: %v", err)
	}
	if err := r.Subscribe(testGroupA, senderSub); err != nil {
		t.Fatalf("subscribe senderSub: %v", err)
	}

	env := testEnvelope(testGroupA, testDeviceA)
	if err := r.Publish(ctx, testGroupA, testDeviceA, env); err != nil {
		t.Fatalf("publish: %v", err)
	}

	// 1. Fast subscriber received the envelope and was not closed.
	fastDelivered := fastSub.getDelivered()
	if len(fastDelivered) != 1 || fastDelivered[0].ID != env.ID {
		t.Fatalf("expected fast subscriber to receive envelope, got: %+v", fastDelivered)
	}
	if closed, _ := fastSub.isClosed(); closed {
		t.Fatal("fast subscriber was unexpectedly closed")
	}

	// 2. Slow subscriber had Close() called with reason "slow consumer".
	closed, reason := slowSub.isClosed()
	if !closed {
		t.Fatal("expected slow subscriber to be closed")
	}
	if reason != "slow consumer" {
		t.Fatalf("expected close reason 'slow consumer', got %q", reason)
	}

	// 3. Sender should not receive its own published envelope.
	senderDelivered := senderSub.getDelivered()
	if len(senderDelivered) != 0 {
		t.Fatalf("sender received its own envelope: %+v", senderDelivered)
	}
}
