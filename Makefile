BINARY := metadata-checker
TARGET_DIR := target
BENCH_TARGET_DIR ?= target/criterion

# Supported targets
TARGETS := \
	aarch64-apple-darwin \
	x86_64-unknown-linux-musl \
	aarch64-unknown-linux-musl \
	x86_64-pc-windows-gnu

.PHONY: all build release clean test perf-real perf-real-mutation perf-real-runtime perf-real-query perf-m51-profile perf-fixture perf-cli-cold perf-redb perf-stdio perf-telemetry perf-session perf-browser-offscreen-samples perf-browser-offscreen perf-browser-offscreen-bencher-bmf perf-browser-offscreen-ci help

BROWSER_OFFSCREEN_FIXTURE_DIR := tests/fixtures/browser-offscreen-real-project
BROWSER_OFFSCREEN_PROJECT_DIR ?= /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi
BROWSER_OFFSCREEN_PROJECT_NAME ?= xiaoshouyi
M51_PROFILE_PROJECT_DIR ?= /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi
M51_PROFILE_OUTPUT ?= target/m51-profile/page-logic-profile.json
M51_PROFILE_SAMPLE_COUNT ?= 3

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

perf-real:
	node tools/m50-real-project-perf.mjs

perf-real-mutation:
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/rebuild cargo bench --bench rebuild_bench

perf-real-runtime:
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/runtime cargo bench --bench runtime_bench

perf-real-query:
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/query-matrix cargo bench --bench query_matrix_bench

perf-m51-profile:
	cargo run --bin m51_profile_report -- \
		--project-dir "$(M51_PROFILE_PROJECT_DIR)" \
		--output "$(M51_PROFILE_OUTPUT)" \
		--sample-count "$(M51_PROFILE_SAMPLE_COUNT)"

perf-fixture:
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/parse cargo bench --bench parse_bench
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/query-micro cargo bench --bench query_micro_bench

perf-cli-cold:
	node tools/cli-cold-start-perf.mjs

perf-redb:
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/redb cargo bench --bench redb_persistence_bench

perf-stdio:
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/stdio cargo bench --bench stdio_boundary_bench

perf-telemetry:
	cargo build --profile release-fast --features telemetry
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/telemetry cargo bench --bench telemetry_overhead_bench

perf-session:
	CARGO_TARGET_DIR=$(BENCH_TARGET_DIR)/session cargo bench --bench session_sync_bench

perf-browser-offscreen-samples:
	cargo run --bin generate_browser_offscreen_fixtures -- \
		--project-dir "$(BROWSER_OFFSCREEN_PROJECT_DIR)" \
		--output-dir "$(BROWSER_OFFSCREEN_FIXTURE_DIR)" \
		--project-name "$(BROWSER_OFFSCREEN_PROJECT_NAME)"
	node browser/bench/validate-offscreen-manifest.mjs --fixture-dir "$(BROWSER_OFFSCREEN_FIXTURE_DIR)"

perf-browser-offscreen:
	node browser/bench/offscreen-local-graph-replay.mjs

perf-browser-offscreen-bencher-bmf:
	node browser/bench/offscreen-summary-to-bencher-bmf.mjs

perf-browser-offscreen-ci:
	node browser/bench/validate-offscreen-manifest.mjs --fixture-dir "$(BROWSER_OFFSCREEN_FIXTURE_DIR)"
	node browser/bench/offscreen-local-graph-replay.mjs --smoke --fixture-dir "$(BROWSER_OFFSCREEN_FIXTURE_DIR)"
	node browser/bench/write-ci-perf-index.mjs --output-dir target/browser-offscreen-bench --ci-tier local-smoke
	node browser/bench/offscreen-summary-to-bencher-bmf.mjs --stats p50

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
	@echo "  perf-real                  Run M50 real-project performance benchmark"
	@echo "  perf-real-mutation         Run real-project rebuild Criterion benchmark"
	@echo "  perf-real-runtime          Run real-project runtime lifecycle Criterion benchmark"
	@echo "  perf-real-query            Run real-project query matrix Criterion benchmark"
	@echo "  perf-m51-profile           Write M51 page logic stage profile JSON"
	@echo "  perf-fixture               Run fixture-based parse and query micro benchmarks"
	@echo "  perf-cli-cold              Run CLI cold-start hyperfine + trace boundary runner"
	@echo "  perf-redb                  Run redb persistence/open/lock Criterion benchmark"
	@echo "  perf-stdio                 Run stdio protocol boundary Criterion benchmark"
	@echo "  perf-telemetry             Run OpenTelemetry overhead Criterion benchmark"
	@echo "  perf-session               Run remote/session sync Criterion benchmark"
	@echo "  perf-browser-offscreen-samples  Generate and validate browser offscreen fixtures"
	@echo "  perf-browser-offscreen     Run full browser offscreen WASM replay bench"
	@echo "  perf-browser-offscreen-bencher-bmf  Convert browser offscreen summary to Bencher BMF JSON"
	@echo "  perf-browser-offscreen-ci  Run CI smoke browser offscreen bench"
	@echo "  clean                      Clean build artifacts"
	@echo ""
	@echo "Per-target builds:"
	@echo "  aarch64-apple-darwin       macOS ARM64"
	@echo "  x86_64-unknown-linux-musl  Linux AMD64"
	@echo "  aarch64-unknown-linux-musl Linux ARM64"
	@echo "  x86_64-pc-windows-gnu      Windows AMD64"
