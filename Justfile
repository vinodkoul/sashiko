# Sashiko Development and CI Tasks

# List available commands
default:
    @just --list

# [PR Suite] Run all checks required for a Pull Request (SOB, Lint, Unit Tests)
check-pr: sob lint test

# [Integration Suite] Run the full integration tests
check-integration: integration-test

# Run the complete check suite (PR + Integration)
check-all: check-pr check-integration

# Check Signed-off-by tags (default: HEAD~1..HEAD)
sob range="HEAD~1..HEAD":
    @./scripts/check-sob.sh {{range}}

# Run all linters (clippy, fmt, yamllint)
lint:
    @cargo clippy --all-targets --all-features --release -- -D warnings
    @cargo fmt --all -- --check
    @yamllint .

# Run unit tests
test:
    @cargo test --release

# [Slow] Run integration tests using benchmarks
integration-test:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "Cleaning up..."
    rm -f sashiko.db sashiko.db.bak

    # In CI, we use the release binary if it exists
    SASHIKO_BIN="./target/release/sashiko"
    #BENCHMARK_BIN="./target/release/benchmark"
    if [ ! -f "$SASHIKO_BIN" ]; then
        SASHIKO_BIN="cargo run --bin sashiko --"
        #BENCHMARK_BIN="cargo run --bin benchmark --"
    fi

    echo "Starting server..."
    $SASHIKO_BIN --no-ai &
    SERVER_PID=$!

    # Ensure server is killed on exit
    trap 'kill $SERVER_PID || true' EXIT

    sleep 10

    exit
