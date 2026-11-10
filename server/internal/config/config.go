// Package config loads relay configuration from environment variables.
package config

import (
	"fmt"
	"os"
	"strconv"
	"time"
)

const (
	defaultAddr           = ":8080"
	defaultMailboxSize    = 50
	defaultMailboxTTL     = 24 * time.Hour
	defaultMaxConnPerGrp  = 32
	defaultPublishRate    = 5.0
	defaultPublishBurst   = 20
	defaultShutdownPeriod = 10 * time.Second
)

// Config holds runtime settings for the relay.
type Config struct {
	Addr             string
	RedisURL         string // empty => in-memory store (dev/test only)
	MailboxSize      int
	MailboxTTL       time.Duration
	MaxConnPerGroup  int
	PublishRate      float64
	PublishBurst     int
	ShutdownTimeout  time.Duration
	LogLevel         string
}

// Load reads configuration from CLIPBRIDGE_* environment variables.
func Load() (Config, error) {
	cfg := Config{
		Addr:            getEnv("CLIPBRIDGE_ADDR", defaultAddr),
		RedisURL:        os.Getenv("CLIPBRIDGE_REDIS_URL"),
		LogLevel:        getEnv("CLIPBRIDGE_LOG_LEVEL", "info"),
		ShutdownTimeout: defaultShutdownPeriod,
	}
	var err error
	if cfg.MailboxSize, err = getInt("CLIPBRIDGE_MAILBOX_SIZE", defaultMailboxSize); err != nil {
		return Config{}, err
	}
	if cfg.MaxConnPerGroup, err = getInt("CLIPBRIDGE_MAX_CONN_PER_GROUP", defaultMaxConnPerGrp); err != nil {
		return Config{}, err
	}
	if cfg.PublishBurst, err = getInt("CLIPBRIDGE_PUBLISH_BURST", defaultPublishBurst); err != nil {
		return Config{}, err
	}
	if cfg.MailboxTTL, err = getDuration("CLIPBRIDGE_MAILBOX_TTL", defaultMailboxTTL); err != nil {
		return Config{}, err
	}
	cfg.PublishRate = defaultPublishRate
	if v := os.Getenv("CLIPBRIDGE_PUBLISH_RATE"); v != "" {
		if cfg.PublishRate, err = strconv.ParseFloat(v, 64); err != nil || cfg.PublishRate <= 0 {
			return Config{}, fmt.Errorf("CLIPBRIDGE_PUBLISH_RATE: must be a positive number, got %q", v)
		}
	}
	if cfg.MailboxSize <= 0 || cfg.MaxConnPerGroup <= 0 || cfg.PublishBurst <= 0 {
		return Config{}, fmt.Errorf("mailbox size, max conn per group and publish burst must be positive")
	}
	return cfg, nil
}

func getEnv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func getInt(key string, fallback int) (int, error) {
	v := os.Getenv(key)
	if v == "" {
		return fallback, nil
	}
	n, err := strconv.Atoi(v)
	if err != nil {
		return 0, fmt.Errorf("%s: invalid integer %q: %w", key, v, err)
	}
	return n, nil
}

func getDuration(key string, fallback time.Duration) (time.Duration, error) {
	v := os.Getenv(key)
	if v == "" {
		return fallback, nil
	}
	d, err := time.ParseDuration(v)
	if err != nil || d <= 0 {
		return 0, fmt.Errorf("%s: invalid positive duration %q", key, v)
	}
	return d, nil
}
