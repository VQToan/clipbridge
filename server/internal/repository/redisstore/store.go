// Package redisstore implements relay persistence on Redis.
package redisstore

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"time"

	"github.com/redis/go-redis/v9"

	"github.com/toanvo/clipbridge/server/internal/service"
)

const (
	groupKeyPrefix   = "cb:group:"
	mailboxKeyPrefix = "cb:mbox:"
)

// Store implements service.GroupStore and service.MailboxStore.
type Store struct {
	rdb      *redis.Client
	maxItems int
	ttl      time.Duration
}

// New creates a Redis-backed store.
func New(rdb *redis.Client, maxItems int, ttl time.Duration) *Store {
	return &Store{rdb: rdb, maxItems: maxItems, ttl: ttl}
}

// RegisterOrGet implements service.GroupStore using SETNX for atomic TOFU registration.
func (s *Store) RegisterOrGet(ctx context.Context, groupID, tokenHash string) (string, error) {
	key := groupKeyPrefix + groupID
	if _, err := s.rdb.SetNX(ctx, key, tokenHash, 0).Result(); err != nil {
		return "", fmt.Errorf("redis setnx %s: %w", key, err)
	}
	stored, err := s.rdb.Get(ctx, key).Result()
	if err != nil {
		return "", fmt.Errorf("redis get %s: %w", key, err)
	}
	return stored, nil
}

// Append implements service.MailboxStore. Newest entries are kept at the list head.
func (s *Store) Append(ctx context.Context, groupID string, env service.StoredEnvelope) error {
	raw, err := json.Marshal(env)
	if err != nil {
		return fmt.Errorf("marshal envelope: %w", err)
	}
	key := mailboxKeyPrefix + groupID
	pipe := s.rdb.TxPipeline()
	pipe.LPush(ctx, key, raw)
	pipe.LTrim(ctx, key, 0, int64(s.maxItems-1))
	pipe.Expire(ctx, key, s.ttl)
	if _, err := pipe.Exec(ctx); err != nil {
		return fmt.Errorf("redis append %s: %w", key, err)
	}
	return nil
}

// List implements service.MailboxStore, returning oldest first.
func (s *Store) List(ctx context.Context, groupID string) ([]service.StoredEnvelope, error) {
	key := mailboxKeyPrefix + groupID
	raws, err := s.rdb.LRange(ctx, key, 0, -1).Result()
	if err != nil {
		return nil, fmt.Errorf("redis lrange %s: %w", key, err)
	}
	out := make([]service.StoredEnvelope, 0, len(raws))
	for _, raw := range raws {
		var env service.StoredEnvelope
		if err := json.Unmarshal([]byte(raw), &env); err != nil {
			// Skip corrupt entries instead of failing the whole replay.
			continue
		}
		out = append(out, env)
	}
	slices.Reverse(out)
	return out, nil
}
