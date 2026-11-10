package domain

import (
	"encoding/base64"
	"errors"
	"strings"
	"testing"

	"github.com/oklog/ulid/v2"
)

const (
	testGroup  = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
	testDevice = "device-a"
)

func validEnvelope() Envelope {
	return Envelope{
		V:          ProtocolVersion,
		ID:         ulid.Make().String(),
		GroupID:    testGroup,
		Sender:     testDevice,
		CreatedAt:  1,
		Nonce:      base64.StdEncoding.EncodeToString(make([]byte, xchachaNonceSize)),
		Ciphertext: base64.StdEncoding.EncodeToString([]byte("ciphertext")),
	}
}

func TestEnvelopeValidate(t *testing.T) {
	tests := []struct {
		name    string
		mutate  func(*Envelope)
		wantErr bool
	}{
		{"valid", func(*Envelope) {}, false},
		{"bad version", func(e *Envelope) { e.V = 2 }, true},
		{"bad id", func(e *Envelope) { e.ID = "not-a-ulid" }, true},
		{"group mismatch", func(e *Envelope) { e.GroupID = strings.Repeat("f", 64) }, true},
		{"sender spoof", func(e *Envelope) { e.Sender = "device-b" }, true},
		{"zero created_at", func(e *Envelope) { e.CreatedAt = 0 }, true},
		{"short nonce", func(e *Envelope) { e.Nonce = base64.StdEncoding.EncodeToString([]byte("short")) }, true},
		{"empty ciphertext", func(e *Envelope) { e.Ciphertext = "" }, true},
		{"non-base64 ciphertext", func(e *Envelope) { e.Ciphertext = "%%%" }, true},
		{"oversized ciphertext", func(e *Envelope) {
			e.Ciphertext = strings.Repeat("A", MaxCiphertextBytes/3*4+8)
		}, true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			env := validEnvelope()
			tt.mutate(&env)
			err := env.Validate(testGroup, testDevice)
			if tt.wantErr != (err != nil) {
				t.Fatalf("Validate() err = %v, wantErr %v", err, tt.wantErr)
			}
			if err != nil && !errors.Is(err, ErrInvalidEnvelope) {
				t.Fatalf("expected ErrInvalidEnvelope, got %v", err)
			}
		})
	}
}

func TestIDValidators(t *testing.T) {
	if !ValidGroupID(testGroup) || ValidGroupID("ABC") {
		t.Fatal("group id validation incorrect")
	}
	if !ValidDeviceID("dev_1-x") || ValidDeviceID("") || ValidDeviceID("has space") {
		t.Fatal("device id validation incorrect")
	}
}
