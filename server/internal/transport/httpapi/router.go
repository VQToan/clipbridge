// Package httpapi wires HTTP routes for the relay.
package httpapi

import (
	"context"
	"log/slog"
	"net/http"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/go-chi/chi/v5/middleware"
)

// HealthChecker reports dependency health (e.g. Redis ping).
type HealthChecker func(ctx context.Context) error

const healthTimeout = 2 * time.Second

// NewRouter builds the HTTP router.
func NewRouter(wsHandler http.Handler, health HealthChecker, log *slog.Logger) http.Handler {
	r := chi.NewRouter()
	r.Use(middleware.RequestID)
	r.Use(middleware.RealIP)
	r.Use(middleware.Recoverer)

	r.Get("/healthz", func(w http.ResponseWriter, req *http.Request) {
		ctx, cancel := context.WithTimeout(req.Context(), healthTimeout)
		defer cancel()
		if health != nil {
			if err := health(ctx); err != nil {
				log.WarnContext(ctx, "health check failed", "err", err)
				http.Error(w, "unhealthy", http.StatusServiceUnavailable)
				return
			}
		}
		w.Header().Set("Content-Type", "text/plain")
		_, _ = w.Write([]byte("ok"))
	})
	r.Get("/v1/ws", wsHandler.ServeHTTP)
	return r
}
