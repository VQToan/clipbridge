package redisstore

import (
	"context"
	"testing"
	"time"

	"github.com/alicebob/miniredis/v2"
	"github.com/redis/go-redis/v9"

	"github.com/toanvo/clipbridge/server/internal/domain"
	"github.com/toanvo/clipbridge/server/internal/service"
)

func newStore(t *testing.T, maxItems int) (*Store, *miniredis.Miniredis) {
	t.Helper()
	mr := miniredis.RunT(t)
	rdb := redis.NewClient(&redis.Options{Addr: mr.Addr()})
	t.Cleanup(func() { _ = rdb.Close() })
	return New(rdb, maxItems, time.Hour), mr
}

func TestRegisterOrGetIsTOFU(t *testing.T) {
	s, _ := newStore(t, 10)
	ctx := context.Background()
	first, err := s.RegisterOrGet(ctx, "g", "hash-1")
	if err != nil || first != "hash-1" {
		t.Fatalf("first register = %q, %v", first, err)
	}
	second, err := s.RegisterOrGet(ctx, "g", "hash-2")
	if err != nil || second != "hash-1" {
		t.Fatalf("second register must return original hash, got %q, %v", second, err)
	}
}

func TestMailboxTrimAndOrder(t *testing.T) {
	s, mr := newStore(t, 3)
	ctx := context.Background()
	for _, id := range []string{"a", "b", "c", "d"} {
		if err := s.Append(ctx, "g", service.StoredEnvelope{ReceivedAt: time.Now(), Envelope: domain.Envelope{ID: id}}); err != nil {
			t.Fatal(err)
		}
	}
	got, err := s.List(ctx, "g")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 3 || got[0].Envelope.ID != "b" || got[2].Envelope.ID != "d" {
		t.Fatalf("unexpected mailbox: %+v", got)
	}
	if ttl := mr.TTL(mailboxKeyPrefix + "g"); ttl != time.Hour {
		t.Fatalf("expected TTL 1h, got %v", ttl)
	}
}
