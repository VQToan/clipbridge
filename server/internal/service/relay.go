// Package service implements the relay use cases: authentication, fan-out and mailbox replay.
package service

import (
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"
	"sync"
	"time"

	"github.com/toanvo/clipbridge/server/internal/domain"
)

// StoredEnvelope wraps an envelope with the server receive time so TTL is not
// dependent on client clocks.
type StoredEnvelope struct {
	ReceivedAt time.Time       `json:"received_at"`
	Envelope   domain.Envelope `json:"envelope"`
}

// GroupStore persists group credentials (hash of relay auth token).
type GroupStore interface {
	// RegisterOrGet stores tokenHash if the group is new and returns the stored hash.
	RegisterOrGet(ctx context.Context, groupID, tokenHash string) (string, error)
}

// MailboxStore keeps the most recent envelopes of a group for offline devices.
type MailboxStore interface {
	Append(ctx context.Context, groupID string, env StoredEnvelope) error
	// List returns envelopes in chronological order (oldest first).
	List(ctx context.Context, groupID string) ([]StoredEnvelope, error)
}

// Subscriber receives envelopes fanned out by the hub.
type Subscriber interface {
	DeviceID() string
	// Deliver must not block; it returns false if the subscriber cannot keep up.
	Deliver(env domain.Envelope) bool
	// Close terminates the subscriber (e.g. slow consumer).
	Close(reason string)
}

// Options tunes relay behaviour.
type Options struct {
	MailboxTTL      time.Duration
	MaxConnPerGroup int
}

// Relay is the application service shared by all transports.
type Relay struct {
	groups  GroupStore
	mailbox MailboxStore
	opts    Options
	log     *slog.Logger
	now     func() time.Time

	mu   sync.RWMutex
	subs map[string]map[Subscriber]struct{}
}

// NewRelay builds a Relay.
func NewRelay(groups GroupStore, mailbox MailboxStore, opts Options, log *slog.Logger) *Relay {
	return &Relay{
		groups:  groups,
		mailbox: mailbox,
		opts:    opts,
		log:     log,
		now:     time.Now,
		subs:    make(map[string]map[Subscriber]struct{}),
	}
}

// Authenticate verifies the bearer token for a group (trust-on-first-use).
func (r *Relay) Authenticate(ctx context.Context, groupID, deviceID, token string) error {
	if !domain.ValidGroupID(groupID) || !domain.ValidDeviceID(deviceID) || token == "" {
		return domain.ErrUnauthorized
	}
	sum := sha256.Sum256([]byte(token))
	presented := hex.EncodeToString(sum[:])
	stored, err := r.groups.RegisterOrGet(ctx, groupID, presented)
	if err != nil {
		return fmt.Errorf("load group credentials: %w", err)
	}
	if subtle.ConstantTimeCompare([]byte(stored), []byte(presented)) != 1 {
		return domain.ErrUnauthorized
	}
	return nil
}

// Subscribe registers a live subscriber for a group.
func (r *Relay) Subscribe(groupID string, s Subscriber) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	set, ok := r.subs[groupID]
	if !ok {
		set = make(map[Subscriber]struct{})
		r.subs[groupID] = set
	}
	if len(set) >= r.opts.MaxConnPerGroup {
		return domain.ErrGroupFull
	}
	set[s] = struct{}{}
	return nil
}

// Unsubscribe removes a subscriber; safe to call multiple times.
func (r *Relay) Unsubscribe(groupID string, s Subscriber) {
	r.mu.Lock()
	defer r.mu.Unlock()
	set, ok := r.subs[groupID]
	if !ok {
		return
	}
	delete(set, s)
	if len(set) == 0 {
		delete(r.subs, groupID)
	}
}

// Publish validates, persists and fans out an envelope to other devices of the group.
func (r *Relay) Publish(ctx context.Context, groupID, deviceID string, env domain.Envelope) error {
	if err := env.Validate(groupID, deviceID); err != nil {
		return err
	}
	stored := StoredEnvelope{ReceivedAt: r.now().UTC(), Envelope: env}
	if err := r.mailbox.Append(ctx, groupID, stored); err != nil {
		// Live delivery still proceeds; only offline replay is degraded.
		r.log.ErrorContext(ctx, "mailbox append failed", "group", shortID(groupID), "envelope", env.ID, "err", err)
	}

	r.mu.RLock()
	targets := make([]Subscriber, 0, len(r.subs[groupID]))
	for s := range r.subs[groupID] {
		if s.DeviceID() != deviceID {
			targets = append(targets, s)
		}
	}
	r.mu.RUnlock()

	for _, s := range targets {
		if !s.Deliver(env) {
			r.log.WarnContext(ctx, "dropping slow subscriber", "group", shortID(groupID), "device", s.DeviceID())
			s.Close("slow consumer")
		}
	}
	return nil
}

// Backlog returns mailbox envelopes newer than sinceID (ULIDs sort lexicographically),
// excluding the requester's own envelopes and expired entries.
func (r *Relay) Backlog(ctx context.Context, groupID, deviceID, sinceID string) ([]domain.Envelope, error) {
	items, err := r.mailbox.List(ctx, groupID)
	if err != nil {
		return nil, fmt.Errorf("list mailbox: %w", err)
	}
	cutoff := r.now().Add(-r.opts.MailboxTTL)
	out := make([]domain.Envelope, 0, len(items))
	for _, it := range items {
		if it.ReceivedAt.Before(cutoff) || it.Envelope.Sender == deviceID {
			continue
		}
		if sinceID != "" && it.Envelope.ID <= sinceID {
			continue
		}
		out = append(out, it.Envelope)
	}
	return out, nil
}

// IsClientError reports whether err should be surfaced to the client rather than logged as internal.
func IsClientError(err error) bool {
	return errors.Is(err, domain.ErrInvalidEnvelope)
}

// shortID keeps logs useful without leaking full group identifiers.
func shortID(id string) string {
	const n = 8
	if len(id) <= n {
		return id
	}
	return id[:n]
}
