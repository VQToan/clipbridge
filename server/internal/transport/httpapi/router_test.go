package httpapi_test

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/toanvo/clipbridge/server/internal/transport/httpapi"
)

func TestRouterHealthz(t *testing.T) {
	log := slog.New(slog.NewTextHandler(io.Discard, nil))
	dummyWS := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {})

	t.Run("healthy nil checker", func(t *testing.T) {
		router := httpapi.NewRouter(dummyWS, nil, log)
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
		router.ServeHTTP(rec, req)

		if rec.Code != http.StatusOK {
			t.Fatalf("expected 200, got %d", rec.Code)
		}
		if body := rec.Body.String(); body != "ok" {
			t.Fatalf("expected 'ok', got %q", body)
		}
	})

	t.Run("healthy custom checker", func(t *testing.T) {
		checker := func(ctx context.Context) error { return nil }
		router := httpapi.NewRouter(dummyWS, checker, log)
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
		router.ServeHTTP(rec, req)

		if rec.Code != http.StatusOK {
			t.Fatalf("expected 200, got %d", rec.Code)
		}
	})

	t.Run("unhealthy checker", func(t *testing.T) {
		checker := func(ctx context.Context) error { return errors.New("redis down") }
		router := httpapi.NewRouter(dummyWS, checker, log)
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
		router.ServeHTTP(rec, req)

		if rec.Code != http.StatusServiceUnavailable {
			t.Fatalf("expected 503, got %d", rec.Code)
		}
	})
}
