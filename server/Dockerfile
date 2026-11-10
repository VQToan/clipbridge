# Multi-stage Dockerfile for ClipBridge relay server
# Builder stage
FROM golang:1.26-alpine AS builder

WORKDIR /src

# Cache dependencies
COPY go.mod go.sum ./
RUN go mod download

# Copy source and build static binary
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o /bin/relay ./cmd/relay

# Final minimal runtime stage
FROM gcr.io/distroless/static-debian12:nonroot

WORKDIR /
COPY --from=builder /bin/relay /relay

EXPOSE 8080
USER nonroot:nonroot

ENTRYPOINT ["/relay"]
