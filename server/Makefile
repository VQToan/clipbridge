.PHONY: all build test vet run docker clean

BIN_DIR := bin
BINARY := $(BIN_DIR)/relay
IMAGE_TAG ?= clipbridge-relay:latest

all: vet test build

build:
	@mkdir -p $(BIN_DIR)
	go build -trimpath -ldflags "-s -w" -o $(BINARY) ./cmd/relay

test:
	go test -race -v ./...

vet:
	go vet ./...

run:
	go run ./cmd/relay

docker:
	docker build -t $(IMAGE_TAG) .

clean:
	rm -rf $(BIN_DIR)
