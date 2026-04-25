BINARY := metadata-checker
TARGET_DIR := target

# Supported targets
TARGETS := \
	aarch64-apple-darwin \
	x86_64-unknown-linux-musl \
	aarch64-unknown-linux-musl \
	x86_64-pc-windows-gnu

.PHONY: all build release clean test help

all: release

build:
	cargo build --release

release: $(TARGETS)

aarch64-apple-darwin:
	@echo "Building for macOS ARM64..."
	cargo build --release --target $@
	mkdir -p $(TARGET_DIR)/release/$@
	cp $(TARGET_DIR)/$@/release/$(BINARY) $(TARGET_DIR)/release/$@/$(BINARY)

x86_64-unknown-linux-musl:
	@echo "Building for Linux AMD64 (musl)..."
	cargo zigbuild --release --target $@
	mkdir -p $(TARGET_DIR)/release/$@
	cp $(TARGET_DIR)/$@/release/$(BINARY) $(TARGET_DIR)/release/$@/$(BINARY)

aarch64-unknown-linux-musl:
	@echo "Building for Linux ARM64 (musl)..."
	cargo zigbuild --release --target $@
	mkdir -p $(TARGET_DIR)/release/$@
	cp $(TARGET_DIR)/$@/release/$(BINARY) $(TARGET_DIR)/release/$@/$(BINARY)

x86_64-pc-windows-gnu:
	@echo "Building for Windows AMD64..."
	cargo zigbuild --release --target $@
	mkdir -p $(TARGET_DIR)/release/$@
	cp $(TARGET_DIR)/$@/release/$(BINARY).exe $(TARGET_DIR)/release/$@/$(BINARY).exe

test:
	cargo test

clean:
	cargo clean

help:
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  all                        Build release binaries for all targets"
	@echo "  build                      Build for current host"
	@echo "  release                    Build for all cross targets"
	@echo "  test                       Run tests"
	@echo "  clean                      Clean build artifacts"
	@echo ""
	@echo "Per-target builds:"
	@echo "  aarch64-apple-darwin       macOS ARM64"
	@echo "  x86_64-unknown-linux-musl  Linux AMD64"
	@echo "  aarch64-unknown-linux-musl Linux ARM64"
	@echo "  x86_64-pc-windows-gnu      Windows AMD64"
