// Package domain contains the relay's core types and validation rules.
// The relay never decrypts envelopes; it only validates routing metadata.
package domain

import (
	"encoding/base64"
	"errors"
	"fmt"
	"regexp"

	"github.com/oklog/ulid/v2"
)

// ProtocolVersion is the only envelope version this relay accepts.
const ProtocolVersion = 1

const (
	// xchachaNonceSize is the nonce length of XChaCha20-Poly1305.
	xchachaNonceSize = 24
	// MaxCiphertextBytes caps decoded ciphertext (10 MiB payload + AEAD tag + CBOR overhead).
	MaxCiphertextBytes = 10*1024*1024 + 64*1024
	// MaxFrameBytes caps a single WebSocket message (base64 expands by 4/3).
	MaxFrameBytes = 16 * 1024 * 1024
)

var (
	ErrInvalidEnvelope = errors.New("invalid envelope")
	ErrUnauthorized    = errors.New("unauthorized")
	ErrGroupFull       = errors.New("group connection limit reached")

	groupIDPattern  = regexp.MustCompile(`^[0-9a-f]{64}$`)
	deviceIDPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)
)

// Envelope is an end-to-end encrypted clipboard item as seen by the relay.
type Envelope struct {
	V          int    `json:"v"`
	ID         string `json:"id"`
	GroupID    string `json:"group_id"`
	Sender     string `json:"sender"`
	CreatedAt  int64  `json:"created_at"`
	Nonce      string `json:"nonce"`
	Ciphertext string `json:"ciphertext"`
}

// ValidGroupID reports whether id is a well-formed group identifier.
func ValidGroupID(id string) bool { return groupIDPattern.MatchString(id) }

// ValidDeviceID reports whether id is a well-formed device identifier.
func ValidDeviceID(id string) bool { return deviceIDPattern.MatchString(id) }

// Validate checks the envelope against the authenticated connection identity.
// Binding group and sender to the connection prevents a member from spoofing another device.
func (e *Envelope) Validate(groupID, deviceID string) error {
	if e.V != ProtocolVersion {
		return fmt.Errorf("%w: unsupported version %d", ErrInvalidEnvelope, e.V)
	}
	if _, err := ulid.ParseStrict(e.ID); err != nil {
		return fmt.Errorf("%w: id is not a ULID", ErrInvalidEnvelope)
	}
	if e.GroupID != groupID {
		return fmt.Errorf("%w: group mismatch", ErrInvalidEnvelope)
	}
	if e.Sender != deviceID {
		return fmt.Errorf("%w: sender mismatch", ErrInvalidEnvelope)
	}
	if e.CreatedAt <= 0 {
		return fmt.Errorf("%w: created_at must be positive", ErrInvalidEnvelope)
	}
	nonce, err := base64.StdEncoding.DecodeString(e.Nonce)
	if err != nil || len(nonce) != xchachaNonceSize {
		return fmt.Errorf("%w: nonce must be %d base64 bytes", ErrInvalidEnvelope, xchachaNonceSize)
	}
	if base64.StdEncoding.DecodedLen(len(e.Ciphertext)) > MaxCiphertextBytes {
		return fmt.Errorf("%w: ciphertext too large", ErrInvalidEnvelope)
	}
	if _, err := base64.StdEncoding.DecodeString(e.Ciphertext); err != nil || e.Ciphertext == "" {
		return fmt.Errorf("%w: ciphertext must be non-empty base64", ErrInvalidEnvelope)
	}
	return nil
}
