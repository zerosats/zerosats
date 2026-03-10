#!/usr/bin/env bash
set -euo pipefail

# Test runner for Ciphera tests
# Runs the non-ignored node e2e tests first, then the ignored full-stack integration tests.
#
# Usage:
#   ./scripts/test.sh                              # run all node e2e tests
#   ./scripts/test.sh burn_tx                      # run a specific ignored integration test
#   ./scripts/test.sh --verbose burn_tx            # with Citrea + deploy logs
#   ./scripts/test.sh --integration-only           # skip fast tests, only run ignored
#   ./scripts/test.sh --docker                     # run everything inside Docker
#   ./scripts/test.sh --docker burn_tx             # run a specific test in Docker

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CITREA_VERSION="${CITREA_VERSION:-v2.1.0}"
CITREA_DIR="$REPO_ROOT/.citrea/$CITREA_VERSION"
DOCKER_IMAGE="satsbridge/ciphera:dev"

# --- Parse args ---
VERBOSE=0
TEST_FILTER=""
INTEGRATION_ONLY=0
USE_DOCKER=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --verbose|-v)
            VERBOSE=1
            shift
            ;;
        --integration-only)
            INTEGRATION_ONLY=1
            shift
            ;;
        --docker)
            USE_DOCKER=1
            shift
            ;;
        *)
            TEST_FILTER="$1"
            shift
            ;;
    esac
done

# =============================================================================
# Docker mode: build images and run tests inside the container
# =============================================================================
if [[ "$USE_DOCKER" -eq 1 ]]; then
    echo "=== Docker mode ==="

    # Build citrea base image if needed
    if ! docker image inspect satsbridge/ciphera:citrea &>/dev/null; then
        echo "Building Citrea base image..."
        docker build -f "$REPO_ROOT/citrea.dockerfile" -t satsbridge/ciphera:citrea "$REPO_ROOT"
    fi

    # Build dev image
    echo "Building dev image..."
    docker build -f "$REPO_ROOT/dev.dockerfile" -t "$DOCKER_IMAGE" "$REPO_ROOT"

    # Assemble the cargo test command to run inside the container
    DOCKER_ENV=()
    if [[ "$VERBOSE" -eq 1 ]]; then
        DOCKER_ENV+=(-e LOG_CITREA_OUTPUT=1 -e LOG_HARDHAT_DEPLOY_OUTPUT=1 -e LOG_NODE_OUTPUT=1)
    fi

    TEST_CMD="cd /app"

    # Phase 1: non-ignored node e2e tests
    if [[ "$INTEGRATION_ONLY" -eq 0 && -z "$TEST_FILTER" ]]; then
        TEST_CMD+=" && echo 'Running non-ignored node e2e tests...' && cargo test -p node --test e2e -- --test-threads=1"
    fi

    # Phase 2: integration tests
    if [[ -n "$TEST_FILTER" ]]; then
        TEST_CMD+=" && echo 'Running integration test: $TEST_FILTER' && cargo test -p node --test e2e $TEST_FILTER -- --ignored --test-threads=1"
    else
        TEST_CMD+=" && echo 'Running full-stack integration tests...' && cargo test -p node --test e2e -- --ignored --test-threads=1"
    fi

    echo "Running tests in Docker..."
    docker run --rm "${DOCKER_ENV[@]}" "$DOCKER_IMAGE" -c "$TEST_CMD"
    exit $?
fi

# =============================================================================
# Local mode: download Citrea binary, set up env, run tests natively
# =============================================================================

# --- Detect platform ---
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin) os="osx" ;;
        Linux)  os="linux" ;;
        *)      echo "Unsupported OS: $os"; exit 1 ;;
    esac

    case "$arch" in
        arm64|aarch64) arch="arm64" ;;
        x86_64)        arch="amd64" ;;
        *)             echo "Unsupported arch: $arch"; exit 1 ;;
    esac

    echo "${os}-${arch}"
}

# --- Download Citrea binary + resources ---
setup_citrea() {
    if [[ -x "$CITREA_DIR/bin/citrea" && -d "$CITREA_DIR/resources/configs/mock" ]]; then
        echo "Citrea $CITREA_VERSION already set up at $CITREA_DIR"
        return
    fi

    local platform
    platform="$(detect_platform)"

    # Citrea publishes raw binaries, not tarballs
    local binary_name="citrea-${CITREA_VERSION}-${platform}"
    local url="https://github.com/chainwayxyz/citrea/releases/download/${CITREA_VERSION}/${binary_name}"

    echo "Downloading Citrea $CITREA_VERSION for $platform..."
    mkdir -p "$CITREA_DIR/bin"
    curl -fSL "$url" -o "$CITREA_DIR/bin/citrea"
    chmod +x "$CITREA_DIR/bin/citrea"

    # Pull mock resources from the release
    echo "Fetching mock configs and genesis..."
    mkdir -p "$CITREA_DIR/resources/configs/mock" "$CITREA_DIR/resources/genesis/mock"

    local base_url="https://raw.githubusercontent.com/chainwayxyz/citrea/${CITREA_VERSION}/resources"
    for f in sequencer_rollup_config.toml sequencer_config.toml; do
        curl -fSL "$base_url/configs/mock/$f" -o "$CITREA_DIR/resources/configs/mock/$f"
    done
    for f in accounts.json evm.json l2_block_rule_enforcer.json; do
        curl -fSL "$base_url/genesis/mock/$f" -o "$CITREA_DIR/resources/genesis/mock/$f"
    done

    echo "Citrea $CITREA_VERSION ready."
}

# --- Compile Solidity contracts ---
compile_contracts() {
    local citrea_dir="$REPO_ROOT/ciphera/citrea"
    if [[ ! -d "$citrea_dir/node_modules" ]]; then
        echo "Installing Hardhat dependencies..."
        (cd "$citrea_dir" && npm ci --silent)
    fi

    # Always compile — artifacts may be tracked but stale after .sol changes
    echo "Compiling Solidity contracts..."
    (cd "$citrea_dir" && npx hardhat compile 2>&1 | grep -v "^WARNING:")
}

# --- Clean stale Citrea state ---
clean_stale_dbs() {
    local dbs_dir="$REPO_ROOT/ciphera/citrea/resources/dbs"
    if [[ -d "$dbs_dir" ]]; then
        echo "Cleaning stale Citrea databases..."
        rm -rf "$dbs_dir"
    fi
}

# --- Main (local) ---
setup_citrea
compile_contracts
clean_stale_dbs

export CIPHERA_TEST_CITREA_BIN="$CITREA_DIR/bin/citrea"
export CIPHERA_TEST_CITREA_CONFIGS_ROOT="$CITREA_DIR/resources/configs"
export CIPHERA_TEST_CITREA_GENESIS_ROOT="$CITREA_DIR/resources/genesis"

if [[ "$VERBOSE" -eq 1 ]]; then
    export LOG_CITREA_OUTPUT=1
    export LOG_HARDHAT_DEPLOY_OUTPUT=1
    export LOG_NODE_OUTPUT=1
fi

cd "$REPO_ROOT/ciphera"

# --- Phase 1: Non-ignored node e2e tests ---
if [[ "$INTEGRATION_ONLY" -eq 0 && -z "$TEST_FILTER" ]]; then
    echo "Running non-ignored node e2e tests..."
    cargo test -p node --test e2e -- --test-threads=1
    echo ""
fi

# --- Phase 2: Full-stack integration tests (Citrea + contracts) ---
if [[ -n "$TEST_FILTER" ]]; then
    echo "Running integration test: $TEST_FILTER"
    cargo test -p node --test e2e "$TEST_FILTER" -- --ignored --test-threads=1
else
    echo "Running full-stack integration tests..."
    cargo test -p node --test e2e -- --ignored --test-threads=1
fi
