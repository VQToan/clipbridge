package config_test

import (
	"testing"
	"time"

	"github.com/toanvo/clipbridge/server/internal/config"
)

func TestConfigDefaults(t *testing.T) {
	t.Setenv("CLIPBRIDGE_ADDR", "")
	t.Setenv("CLIPBRIDGE_REDIS_URL", "")
	t.Setenv("CLIPBRIDGE_MAILBOX_SIZE", "")
	t.Setenv("CLIPBRIDGE_MAILBOX_TTL", "")
	t.Setenv("CLIPBRIDGE_MAX_CONN_PER_GROUP", "")
	t.Setenv("CLIPBRIDGE_PUBLISH_RATE", "")
	t.Setenv("CLIPBRIDGE_PUBLISH_BURST", "")
	t.Setenv("CLIPBRIDGE_LOG_LEVEL", "")

	cfg, err := config.Load()
	if err != nil {
		t.Fatalf("config.Load() unexpected error: %v", err)
	}

	if cfg.Addr != ":8080" {
		t.Errorf("expected Addr :8080, got %q", cfg.Addr)
	}
	if cfg.MailboxSize != 50 {
		t.Errorf("expected MailboxSize 50, got %d", cfg.MailboxSize)
	}
	if cfg.MailboxTTL != 24*time.Hour {
		t.Errorf("expected MailboxTTL 24h, got %v", cfg.MailboxTTL)
	}
	if cfg.MaxConnPerGroup != 32 {
		t.Errorf("expected MaxConnPerGroup 32, got %d", cfg.MaxConnPerGroup)
	}
	if cfg.PublishRate != 5.0 {
		t.Errorf("expected PublishRate 5.0, got %f", cfg.PublishRate)
	}
	if cfg.PublishBurst != 20 {
		t.Errorf("expected PublishBurst 20, got %d", cfg.PublishBurst)
	}
	if cfg.LogLevel != "info" {
		t.Errorf("expected LogLevel info, got %q", cfg.LogLevel)
	}
}

func TestConfigCustomEnv(t *testing.T) {
	t.Setenv("CLIPBRIDGE_ADDR", ":9090")
	t.Setenv("CLIPBRIDGE_REDIS_URL", "redis://localhost:6379/1")
	t.Setenv("CLIPBRIDGE_MAILBOX_SIZE", "100")
	t.Setenv("CLIPBRIDGE_MAILBOX_TTL", "12h")
	t.Setenv("CLIPBRIDGE_MAX_CONN_PER_GROUP", "64")
	t.Setenv("CLIPBRIDGE_PUBLISH_RATE", "10.5")
	t.Setenv("CLIPBRIDGE_PUBLISH_BURST", "40")
	t.Setenv("CLIPBRIDGE_LOG_LEVEL", "debug")

	cfg, err := config.Load()
	if err != nil {
		t.Fatalf("config.Load() unexpected error: %v", err)
	}

	if cfg.Addr != ":9090" {
		t.Errorf("expected Addr :9090, got %q", cfg.Addr)
	}
	if cfg.RedisURL != "redis://localhost:6379/1" {
		t.Errorf("expected RedisURL, got %q", cfg.RedisURL)
	}
	if cfg.MailboxSize != 100 {
		t.Errorf("expected MailboxSize 100, got %d", cfg.MailboxSize)
	}
	if cfg.MailboxTTL != 12*time.Hour {
		t.Errorf("expected MailboxTTL 12h, got %v", cfg.MailboxTTL)
	}
	if cfg.MaxConnPerGroup != 64 {
		t.Errorf("expected MaxConnPerGroup 64, got %d", cfg.MaxConnPerGroup)
	}
	if cfg.PublishRate != 10.5 {
		t.Errorf("expected PublishRate 10.5, got %f", cfg.PublishRate)
	}
	if cfg.PublishBurst != 40 {
		t.Errorf("expected PublishBurst 40, got %d", cfg.PublishBurst)
	}
	if cfg.LogLevel != "debug" {
		t.Errorf("expected LogLevel debug, got %q", cfg.LogLevel)
	}
}

func TestConfigInvalid(t *testing.T) {
	tests := []struct {
		name string
		env  map[string]string
	}{
		{"invalid integer", map[string]string{"CLIPBRIDGE_MAILBOX_SIZE": "abc"}},
		{"negative size", map[string]string{"CLIPBRIDGE_MAILBOX_SIZE": "-1"}},
		{"zero max conn", map[string]string{"CLIPBRIDGE_MAX_CONN_PER_GROUP": "0"}},
		{"invalid duration", map[string]string{"CLIPBRIDGE_MAILBOX_TTL": "not-duration"}},
		{"negative duration", map[string]string{"CLIPBRIDGE_MAILBOX_TTL": "-5m"}},
		{"invalid rate", map[string]string{"CLIPBRIDGE_PUBLISH_RATE": "not-float"}},
		{"zero rate", map[string]string{"CLIPBRIDGE_PUBLISH_RATE": "0"}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			for k, v := range tt.env {
				t.Setenv(k, v)
			}
			_, err := config.Load()
			if err == nil {
				t.Errorf("expected error for %s, got nil", tt.name)
			}
		})
	}
}
