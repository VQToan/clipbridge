// Command relay runs the ClipBridge zero-knowledge relay server.
package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/redis/go-redis/v9"

	"github.com/toanvo/clipbridge/server/internal/config"
	"github.com/toanvo/clipbridge/server/internal/repository/memory"
	"github.com/toanvo/clipbridge/server/internal/repository/redisstore"
	"github.com/toanvo/clipbridge/server/internal/service"
	"github.com/toanvo/clipbridge/server/internal/transport/httpapi"
	"github.com/toanvo/clipbridge/server/internal/transport/ws"
)

const readHeaderTimeout = 10 * time.Second

func main() {
	if err := run(); err != nil {
		slog.Error("relay exited", "err", err)
		os.Exit(1)
	}
}

func run() error {
	cfg, err := config.Load()
	if err != nil {
		return fmt.Errorf("load config: %w", err)
	}
	log := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: parseLevel(cfg.LogLevel)}))

	var (
		groups  service.GroupStore
		mailbox service.MailboxStore
		health  httpapi.HealthChecker
	)
	if cfg.RedisURL != "" {
		opts, err := redis.ParseURL(cfg.RedisURL)
		if err != nil {
			return fmt.Errorf("parse CLIPBRIDGE_REDIS_URL: %w", err)
		}
		rdb := redis.NewClient(opts)
		defer rdb.Close()
		store := redisstore.New(rdb, cfg.MailboxSize, cfg.MailboxTTL)
		groups, mailbox = store, store
		health = func(ctx context.Context) error { return rdb.Ping(ctx).Err() }
		log.Info("using redis store")
	} else {
		store := memory.New(cfg.MailboxSize)
		groups, mailbox = store, store
		log.Warn("CLIPBRIDGE_REDIS_URL not set: using in-memory store (data lost on restart)")
	}

	relay := service.NewRelay(groups, mailbox, service.Options{
		MailboxTTL:      cfg.MailboxTTL,
		MaxConnPerGroup: cfg.MaxConnPerGroup,
	}, log)
	wsHandler := ws.NewHandler(relay, log, cfg.PublishRate, cfg.PublishBurst)

	srv := &http.Server{
		Addr:              cfg.Addr,
		Handler:           httpapi.NewRouter(wsHandler, health, log),
		ReadHeaderTimeout: readHeaderTimeout,
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	errCh := make(chan error, 1)
	go func() {
		log.Info("relay listening", "addr", cfg.Addr)
		if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errCh <- err
		}
		close(errCh)
	}()

	select {
	case err := <-errCh:
		return fmt.Errorf("http server: %w", err)
	case <-ctx.Done():
	}

	log.Info("shutting down")
	shutdownCtx, cancel := context.WithTimeout(context.Background(), cfg.ShutdownTimeout)
	defer cancel()
	if err := srv.Shutdown(shutdownCtx); err != nil {
		return fmt.Errorf("graceful shutdown: %w", err)
	}
	return nil
}

func parseLevel(s string) slog.Level {
	switch strings.ToLower(s) {
	case "debug":
		return slog.LevelDebug
	case "warn":
		return slog.LevelWarn
	case "error":
		return slog.LevelError
	default:
		return slog.LevelInfo
	}
}
