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
CITREA_BASE_IMAGE="satsbridge/ciphera:citrea"
DOCKER_IMAGE="satsbridge/ciphera:dev"
BB_VERSION="${BB_VERSION:-1.0.0-nightly.20250723}"
DOCKER_PLATFORM="${DOCKER_PLATFORM:-linux/amd64}"

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

resolve_citrea_binary_name() {
    local platform="$1"
    local name url
    local -a candidates=()

    case "$platform" in
        linux-amd64) candidates=("citrea-${CITREA_VERSION}-linux-amd64") ;;
        linux-arm64) candidates=("citrea-${CITREA_VERSION}-linux-arm64") ;;
        osx-arm64) candidates=("citrea-${CITREA_VERSION}-osx-arm64") ;;
        osx-amd64)
            candidates=(
                "citrea-${CITREA_VERSION}-osx-amd64"
                "citrea-${CITREA_VERSION}-osx-x86_64"
                "citrea-${CITREA_VERSION}-darwin-amd64"
                "citrea-${CITREA_VERSION}-darwin-x86_64"
            )
            ;;
        *)
            return 1
            ;;
    esac

    for name in "${candidates[@]}"; do
        url="https://github.com/chainwayxyz/citrea/releases/download/${CITREA_VERSION}/${name}"
        if curl -fsSLI "$url" >/dev/null 2>&1; then
            echo "$name"
            return 0
        fi
    done

    return 1
}

if [[ -n "${CIPHERA_TEST_CITREA_BIN:-}" && ! -x "${CIPHERA_TEST_CITREA_BIN}" ]]; then
    echo "CIPHERA_TEST_CITREA_BIN is set but not executable: ${CIPHERA_TEST_CITREA_BIN}"
    exit 1
fi

if [[ "$USE_DOCKER" -eq 0 && -z "${CIPHERA_TEST_CITREA_BIN:-}" ]]; then
    PLATFORM="$(detect_platform)"
    if ! resolve_citrea_binary_name "$PLATFORM" >/dev/null; then
        if command -v docker >/dev/null 2>&1; then
            echo "No Citrea binary published for platform=$PLATFORM version=$CITREA_VERSION; switching to Docker mode."
            USE_DOCKER=1
        else
            echo "No Citrea binary published for platform=$PLATFORM version=$CITREA_VERSION."
            echo "Use --docker or set CIPHERA_TEST_CITREA_BIN to a working local binary."
            exit 1
        fi
    fi
fi

# =============================================================================
# Docker mode: build images and run tests inside the container
# =============================================================================
if [[ "$USE_DOCKER" -eq 1 ]]; then
    echo "=== Docker mode ==="

    echo "Building Citrea base image..."
    docker build \
        --platform "$DOCKER_PLATFORM" \
        --build-arg CITREA_VERSION="$CITREA_VERSION" \
        -f "$REPO_ROOT/citrea.dockerfile" \
        -t "$CITREA_BASE_IMAGE" \
        "$REPO_ROOT"

    # Build dev image
    echo "Building dev image..."
    docker build \
        --platform "$DOCKER_PLATFORM" \
        --build-arg CITREA_BASE_IMAGE="$CITREA_BASE_IMAGE" \
        --build-arg BB_VERSION="$BB_VERSION" \
        -f "$REPO_ROOT/dev.dockerfile" \
        -t "$DOCKER_IMAGE" \
        "$REPO_ROOT"

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
        DOCKER_ENV+=(-e "CIPHERA_TEST_FILTER=$TEST_FILTER")
        TEST_CMD+=" && printf 'Running integration test: %s\n' \"\$CIPHERA_TEST_FILTER\" && cargo test -p node --test e2e \"\$CIPHERA_TEST_FILTER\" -- --ignored --test-threads=1"
    else
        TEST_CMD+=" && echo 'Running full-stack integration tests...' && cargo test -p node --test e2e -- --ignored --test-threads=1"
    fi

    echo "Running tests in Docker..."
    docker run --rm --platform "$DOCKER_PLATFORM" "${DOCKER_ENV[@]}" "$DOCKER_IMAGE" -c "$TEST_CMD"
    exit $?
fi

# =============================================================================
# Local mode: download Citrea binary, set up env, run tests natively
# =============================================================================

# --- Download Citrea binary + resources ---
setup_citrea() {
    local binary_path="${CIPHERA_TEST_CITREA_BIN:-$CITREA_DIR/bin/citrea}"
    local configs_root="${CIPHERA_TEST_CITREA_CONFIGS_ROOT:-$CITREA_DIR/resources/configs}"
    local genesis_root="${CIPHERA_TEST_CITREA_GENESIS_ROOT:-$CITREA_DIR/resources/genesis}"

    if [[ -x "$binary_path" && -d "$configs_root/mock" && -d "$genesis_root/mock" ]]; then
        echo "Citrea $CITREA_VERSION already set up."
        return
    fi

    mkdir -p "$configs_root/mock" "$genesis_root/mock"

    if [[ -z "${CIPHERA_TEST_CITREA_BIN:-}" ]]; then
        local platform binary_name url
        mkdir -p "$CITREA_DIR/bin"
        platform="$(detect_platform)"
        binary_name="$(resolve_citrea_binary_name "$platform")" || {
            echo "No Citrea binary found for platform=$platform version=$CITREA_VERSION"
            echo "Use --docker or set CIPHERA_TEST_CITREA_BIN to a working local binary."
            exit 1
        }
        url="https://github.com/chainwayxyz/citrea/releases/download/${CITREA_VERSION}/${binary_name}"

        echo "Downloading Citrea $CITREA_VERSION for $platform..."
        curl -fSL "$url" -o "$CITREA_DIR/bin/citrea"
        chmod +x "$CITREA_DIR/bin/citrea"
    fi

    # Pull mock resources from the release
    echo "Fetching mock configs and genesis..."

    local base_url="https://raw.githubusercontent.com/chainwayxyz/citrea/${CITREA_VERSION}/resources"
    for f in sequencer_rollup_config.toml sequencer_config.toml; do
        curl -fSL "$base_url/configs/mock/$f" -o "$configs_root/mock/$f"
    done
    for f in accounts.json evm.json l2_block_rule_enforcer.json; do
        curl -fSL "$base_url/genesis/mock/$f" -o "$genesis_root/mock/$f"
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

export CIPHERA_TEST_CITREA_BIN="${CIPHERA_TEST_CITREA_BIN:-$CITREA_DIR/bin/citrea}"
export CIPHERA_TEST_CITREA_CONFIGS_ROOT="${CIPHERA_TEST_CITREA_CONFIGS_ROOT:-$CITREA_DIR/resources/configs}"
export CIPHERA_TEST_CITREA_GENESIS_ROOT="${CIPHERA_TEST_CITREA_GENESIS_ROOT:-$CITREA_DIR/resources/genesis}"

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
