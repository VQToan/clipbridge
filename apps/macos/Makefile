.PHONY: build run clean

build:
	cd ../../core && cargo build -p clip-ffi
	DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift build -c release

dev:
	cd ../../core && cargo build -p clip-ffi
	DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift build

run: dev
	DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift run

clean:
	DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift package clean
