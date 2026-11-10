// Package memory provides in-process stores for development and tests.
// Data is lost on restart; use the Redis store in production.
package memory

import (
	"context"
	"sync"

	"github.com/toanvo/clipbridge/server/internal/service"
)

// Store implements service.GroupStore and service.MailboxStore.
type Store struct {
	maxItems int

	mu      sync.Mutex
	groups  map[string]string
	mailbox map[string][]service.StoredEnvelope
}

// New creates a Store keeping at most maxItems envelopes per group.
func New(maxItems int) *Store {
	return &Store{
		maxItems: maxItems,
		groups:   make(map[string]string),
		mailbox:  make(map[string][]service.StoredEnvelope),
	}
}

// RegisterOrGet implements service.GroupStore.
func (s *Store) RegisterOrGet(_ context.Context, groupID, tokenHash string) (string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if existing, ok := s.groups[groupID]; ok {
		return existing, nil
	}
	s.groups[groupID] = tokenHash
	return tokenHash, nil
}

// Append implements service.MailboxStore.
func (s *Store) Append(_ context.Context, groupID string, env service.StoredEnvelope) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	items := append(s.mailbox[groupID], env)
	if len(items) > s.maxItems {
		items = items[len(items)-s.maxItems:]
	}
	s.mailbox[groupID] = items
	return nil
}

// List implements service.MailboxStore.
func (s *Store) List(_ context.Context, groupID string) ([]service.StoredEnvelope, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]service.StoredEnvelope(nil), s.mailbox[groupID]...), nil
}
