#!/bin/bash

# ============================================================================
# Ciphera Wallet E2E Testing Script
# ============================================================================
# This script tests various wallet flows:
#   1. Same amounts flow (tokens equal throughout)
#   2. Variation of amounts (testing different note denominations)
#
# Usage:
#   export SECRET="your_secret_key"
#   export BURN_ADDRESS="0xyour_address"
#   bash ciphera_wallet_test.sh
#
# ============================================================================

set -e  # Exit on error

# Configuration
GETH_RPC="${GETH_RPC:-https://rpc.testnet.citrea.xyz}"
HOST="${HOST:-ciphera.satsbridge.com}"
PORT="${PORT:-443}"
CLI_BIN="./target/debug/ciphera-cli"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_section() {
    echo -e "\n${BLUE}=================================================================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}=================================================================================${NC}\n"
}

# Validate environment
validate_env() {
    log_info "Validating environment..."

    if [ -z "$SECRET" ]; then
        log_error "SECRET environment variable not set"
        exit 1
    fi

    if [ -z "$BURN_ADDRESS" ]; then
        log_error "BURN_ADDRESS environment variable not set"
        exit 1
    fi

    log_success "Environment validated"
}

# Check if command exists
check_command() {
    if ! command -v cargo &> /dev/null; then
        log_error "cargo not found in PATH"
        exit 1
    fi
}

# Clean up wallet files from previous runs
cleanup_wallets() {
    log_info "Cleaning up wallet files from previous runs..."
    rm -f alice.json bob.json charlie.json david.json
    rm -f alice-note.json bob-note.json charlie-note.json david-note.json
    log_success "Cleanup complete"
}


create_wallets() {
    local wallets=("alice" "bob" "charlie" "david")
    local failed_wallets=()

    for wallet in "${wallets[@]}"; do
        log_section "CREATING ${wallet^^} WALLET"

        if $CLI_BIN create --name "$wallet"; then
            log_success "${wallet^} wallet created"
        else
            log_error "Failed to create ${wallet^} wallet"
            failed_wallets+=("$wallet")
        fi
    done

    [ ${#failed_wallets[@]} -gt 0 ] && return 1
    log_success "✓ All wallets created successfully"
    return 0
}

# ============================================================================
# TEST 1: SAME AMOUNTS FLOW
# ============================================================================
# Test where all transfers are the same amount
# Flow: alice (mint 1000) -> bob (spend 1000) -> charlie (spend 1000)
# ============================================================================

test_same_amounts_flow() {
    log_section "TEST 1: SAME AMOUNTS FLOW"

    local MINT_AMOUNT="1000"
    local TRANSFER_AMOUNT="1000"

    log_info "Scenario: Alice mints $MINT_AMOUNT, transfers to Bob, Bob to Charlie, all same amounts"

    # Step 1: Mint tokens for alice
    log_info "[1/5] Minting $MINT_AMOUNT tokens for alice..."
    if $CLI_BIN mint \
        --geth-rpc "$GETH_RPC" \
        --secret "$SECRET" \
        --amount "$MINT_AMOUNT" \
        --host "$HOST" \
        --port "$PORT" \
        --name alice; then
        log_success "Alice minted $MINT_AMOUNT tokens"
    else
        log_error "Failed to mint tokens for alice"
        return 1
    fi

    # Step 2: Alice spends to bob
    log_info "[2/5] Alice spending $TRANSFER_AMOUNT to Bob..."
    if $CLI_BIN spend \
        --host "$HOST" \
        --port "$PORT" \
        --amount "$TRANSFER_AMOUNT" \
        --name alice; then
        log_success "Alice created spend note"
        [ -f alice-note.json ] && log_success "Spend note created: alice-note.json"
    else
        log_error "Failed to create spend note from alice"
        return 1
    fi

    # Step 3: Bob receives from alice
    log_info "[3/5] Bob receiving $TRANSFER_AMOUNT from Alice..."
    if $CLI_BIN receive \
        --host "$HOST" \
        --port "$PORT" \
        --name bob \
        --note alice-note.json; then
        log_success "Bob received $TRANSFER_AMOUNT from Alice"
    else
        log_error "Failed to receive note for bob"
        return 1
    fi

    # Step 4: Bob spends to charlie
    log_info "[4/5] Bob spending $TRANSFER_AMOUNT to Charlie..."
    if $CLI_BIN spend \
        --host "$HOST" \
        --port "$PORT" \
        --amount "$TRANSFER_AMOUNT" \
        --name bob; then
        log_success "Bob created spend note"
        [ -f bob-note.json ] && log_success "Spend note created: bob-note.json"
    else
        log_error "Failed to create spend note from bob"
        return 1
    fi

    # Step 5: Charlie receives from bob
    log_info "[5/5] Charlie receiving $TRANSFER_AMOUNT from Bob..."
    if $CLI_BIN receive \
        --host "$HOST" \
        --port "$PORT" \
        --name charlie \
        --note bob-note.json; then
        log_success "Charlie received $TRANSFER_AMOUNT from Bob"
    else
        log_error "Failed to receive note for charlie"
        return 1
    fi

    log_success "✓ SAME AMOUNTS FLOW COMPLETED"
}

# ============================================================================
# TEST 2: VARYING AMOUNTS FLOW (Scenario A)
# ============================================================================
# Test where amounts vary:
# alice mints 5000 -> alice spends 2000 to bob -> bob spends 1500 to charlie
# ============================================================================

test_varying_amounts_flow_a() {
    log_section "TEST 2A: VARYING AMOUNTS FLOW (Decreasing)"

    local MINT_AMOUNT="5000"
    local TRANSFER_1="2000"
    local TRANSFER_2="1500"

    log_info "Scenario: Alice mints $MINT_AMOUNT, spends $TRANSFER_1 to Bob, Bob spends $TRANSFER_2 to Charlie"

    # Step 1: Mint tokens for alice
    log_info "[1/5] Minting $MINT_AMOUNT tokens for alice..."
    if $CLI_BIN mint \
        --geth-rpc "$GETH_RPC" \
        --secret "$SECRET" \
        --amount "$MINT_AMOUNT" \
        --host "$HOST" \
        --port "$PORT" \
        --name alice; then
        log_success "Alice minted $MINT_AMOUNT tokens"
    else
        log_error "Failed to mint tokens for alice"
        return 1
    fi

    # Step 2: Alice spends different amount to bob
    log_info "[2/5] Alice spending $TRANSFER_1 to Bob (from $MINT_AMOUNT available)..."
    if $CLI_BIN spend \
        --host "$HOST" \
        --port "$PORT" \
        --amount "$TRANSFER_1" \
        --name alice; then
        log_success "Alice created spend note for $TRANSFER_1"
    else
        log_error "Failed to create spend note from alice"
        return 1
    fi

    # Step 3: Bob receives from alice
    log_info "[3/5] Bob receiving $TRANSFER_1 from Alice..."
    if $CLI_BIN receive \
        --host "$HOST" \
        --port "$PORT" \
        --name bob \
        --note alice-note.json; then
        log_success "Bob received $TRANSFER_1 from Alice"
    else
        log_error "Failed to receive note for bob"
        return 1
    fi

    # Step 4: Bob spends different amount to charlie
    log_info "[4/5] Bob spending $TRANSFER_2 to Charlie (from $TRANSFER_1 available)..."
    if $CLI_BIN spend \
        --host "$HOST" \
        --port "$PORT" \
        --amount "$TRANSFER_2" \
        --name bob; then
        log_success "Bob created spend note for $TRANSFER_2"
    else
        log_error "Failed to create spend note from bob"
        return 1
    fi

    # Step 5: Charlie receives from bob
    log_info "[5/5] Charlie receiving $TRANSFER_2 from Bob..."
    if $CLI_BIN receive \
        --host "$HOST" \
        --port "$PORT" \
        --name charlie \
        --note bob-note.json; then
        log_success "Charlie received $TRANSFER_2 from Bob"
    else
        log_error "Failed to receive note for charlie"
        return 1
    fi

    log_success "✓ VARYING AMOUNTS FLOW (Decreasing) COMPLETED"
}

# ============================================================================
# TEST 3: MULTI-TRANSFER WITH CONSOLIDATION
# ============================================================================
# Test that creates multiple notes and consolidates them
# ============================================================================

test_multi_transfer_consolidation() {
    log_section "TEST 3: MULTI-TRANSFER WITH CONSOLIDATION"

    local INITIAL_MINT="1200"
    local TRANSFER_1="1000"
    local TRANSFER_2="500"
    local TRANSFER_3="800"
    local FINAL_SPEND="2000"

    log_info "Scenario: Alice receives multiple transfers and consolidates"
    log_info "  - Alice mints $INITIAL_MINT"
    log_info "  - Bob transfers $TRANSFER_1 to Alice"
    log_info "  - Charlie transfers $TRANSFER_2 to Alice"
    log_info "  - David transfers $TRANSFER_3 to Alice"
    log_info "  - Alice spends consolidated $FINAL_SPEND"

    # Setup: Mint for multiple wallets
    log_info "[1/8] Setting up wallets..."
    for wallet in bob charlie david; do
        if $CLI_BIN mint \
            --geth-rpc "$GETH_RPC" \
            --secret "$SECRET" \
            --amount "$TRANSFER_1" \
            --host "$HOST" \
            --port "$PORT" \
            --name "$wallet" &> /dev/null; then
            log_success "Minted tokens for $wallet"
        else
            log_error "Failed to mint for $wallet"
            return 1
        fi
    done

    # Alice initial mint
    log_info "[2/8] Alice minting $INITIAL_MINT..."
    if $CLI_BIN mint \
        --geth-rpc "$GETH_RPC" \
        --secret "$SECRET" \
        --amount "$INITIAL_MINT" \
        --host "$HOST" \
        --port "$PORT" \
        --name alice; then
        log_success "Alice minted $INITIAL_MINT"
    else
        log_error "Failed to mint for alice"
        return 1
    fi

    # Multiple transfers to alice
    local transfers=("bob:$TRANSFER_1" "charlie:$TRANSFER_2" "david:$TRANSFER_3")
    local step=3

    for transfer in "${transfers[@]}"; do
        IFS=':' read -r sender amount <<< "$transfer"
        log_info "[$step/8] $sender spending $amount to Alice..."

        if $CLI_BIN spend \
            --host "$HOST" \
            --port "$PORT" \
            --amount "$amount" \
            --name "$sender" &> /dev/null; then

            if $CLI_BIN receive \
                --host "$HOST" \
                --port "$PORT" \
                --name alice \
                --note "${sender}-note.json" &> /dev/null; then
                log_success "Alice received $amount from $sender"
            else
                log_error "Alice failed to receive from $sender"
                return 1
            fi
        else
            log_error "Failed to spend from $sender"
            return 1
        fi

        ((step++))
    done

    # Alice consolidates and spends
    log_info "[7/8] Alice spending consolidated $FINAL_SPEND..."
    if $CLI_BIN spend \
        --host "$HOST" \
        --port "$PORT" \
        --amount "$FINAL_SPEND" \
        --name alice; then
        log_success "Alice created consolidated spend note"
    else
        log_error "Failed to create spend from consolidated balance"
        return 1
    fi

    log_success "✓ MULTI-TRANSFER CONSOLIDATION COMPLETED"
}

# ============================================================================
# TEST 5: BURN FLOW
# ============================================================================
# Test burning tokens (converting back to main chain)
# ============================================================================

test_burn_flow() {
    log_section "TEST 4: BURN FLOW"

    local BURN_AMOUNT="800"

    log_info "Scenario: Alice burns $BURN_AMOUNT"
    log_info "Burn address: $BURN_ADDRESS"

    # Use existing alice wallet
    log_info "[1/2] Burning $BURN_AMOUNT tokens..."
    if $CLI_BIN burn \
        --amount "$BURN_AMOUNT" \
        --host "$HOST" \
        --port "$PORT" \
        --name alice \
        --address "$BURN_ADDRESS"; then
        log_success "Burned $BURN_AMOUNT tokens"
    else
        log_error "Failed to burn tokens"
        return 1
    fi

    # Check contract state
    log_info "[2/2] Checking contract state..."
    if $CLI_BIN contract \
        --geth-rpc "$GETH_RPC"; then
        log_success "Contract state verified"
    else
        log_error "Failed to check contract state"
        return 1
    fi

    log_success "✓ BURN FLOW COMPLETED"
}

# ============================================================================
# Main Test Runner
# ============================================================================

run_all_tests() {
    log_section "CIPHERA WALLET E2E TEST SUITE"

    local tests_passed=0
    local tests_failed=0

    validate_env
    check_command

    cleanup_wallets
    create_wallets

    # Run Test 1: Same Amounts
    test_same_amounts_flow

    cleanup_wallets
    create_wallets

    test_varying_amounts_flow_a

    cleanup_wallets
    create_wallets

    test_multi_transfer_consolidation

    test_burn_flow
}

# Run tests
run_all_tests
exit $?