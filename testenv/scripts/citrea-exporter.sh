#!/usr/bin/env bash
# Citrea Metrics Exporter — polls Citrea RPC + rollup contract, serves Prometheus metrics
set -euo pipefail

CITREA_RPC="${CITREA_RPC:-http://citrea:12345}"
PORT=9101
METRICS_DIR=/tmp/metrics
METRICS_FILE=$METRICS_DIR/index.html
mkdir -p "$METRICS_DIR"

# Read deployed addresses
source /deploy-output/env.sh 2>/dev/null || true
ROLLUP="${ROLLUP_PROXY:-0xcf7ed3acca5a467e9e704c703e8d87f634fb0fc9}"
ERC20="${ERC20_ADDR:-0x5fbdb2315678afecb367f032d93f642f64180aa3}"
DEPLOYER="0xf39Fd6e51aad88F6F4ce6aB8827279cfffb92266"

echo "Citrea exporter: RPC=$CITREA_RPC ROLLUP=$ROLLUP ERC20=$ERC20"

hex2dec() {
    local hex="${1#0x}"
    hex="${hex#"${hex%%[!0]*}"}"
    [ -z "$hex" ] && echo "0" && return
    printf "%d\n" "0x$hex" 2>/dev/null || echo "0"
}

eth_call() {
    curl -sf "$CITREA_RPC" -X POST -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"eth_call\",\"params\":[{\"to\":\"$1\",\"data\":\"$2\"},\"latest\"],\"id\":1}" | jq -r '.result // "0x0"'
}

pad_addr() { printf "%064s" "${1#0x}" | tr ' ' '0'; }

collect() {
    local block_hex gas_hex block_json gu gl tc rh_hex tok_hex dep_hex

    block_hex=$(curl -sf "$CITREA_RPC" -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' | jq -r '.result // "0x0"')
    gas_hex=$(curl -sf "$CITREA_RPC" -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_gasPrice","params":[],"id":1}' | jq -r '.result // "0x0"')
    block_json=$(curl -sf "$CITREA_RPC" -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}')

    gu=$(echo "$block_json" | jq -r '.result.gasUsed // "0x0"')
    gl=$(echo "$block_json" | jq -r '.result.gasLimit // "0x0"')
    tc=$(echo "$block_json" | jq -r '.result.transactions | length')

    rh_hex=$(eth_call "$ROLLUP" "0xf44ff712")
    tok_hex=$(eth_call "$ERC20" "0x70a08231$(pad_addr "$ROLLUP")")
    dep_hex=$(curl -sf "$CITREA_RPC" -X POST -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getBalance\",\"params\":[\"$DEPLOYER\",\"latest\"],\"id\":1}" | jq -r '.result // "0x0"')

    cat > "$METRICS_FILE" << EOF
# HELP citrea_block_number Current Citrea L1 block number
# TYPE citrea_block_number gauge
citrea_block_number $(hex2dec "$block_hex")
# HELP citrea_gas_price_wei Current gas price in wei
# TYPE citrea_gas_price_wei gauge
citrea_gas_price_wei $(hex2dec "$gas_hex")
# HELP citrea_block_gas_used Gas used in latest block
# TYPE citrea_block_gas_used gauge
citrea_block_gas_used $(hex2dec "$gu")
# HELP citrea_block_gas_limit Gas limit of latest block
# TYPE citrea_block_gas_limit gauge
citrea_block_gas_limit $(hex2dec "$gl")
# HELP citrea_block_tx_count Transactions in latest block
# TYPE citrea_block_tx_count gauge
citrea_block_tx_count $tc
# HELP citrea_rollup_verified_height Last verified block height on rollup contract
# TYPE citrea_rollup_verified_height gauge
citrea_rollup_verified_height $(hex2dec "$rh_hex")
# HELP citrea_rollup_tokens_locked ERC20 tokens locked in rollup contract
# TYPE citrea_rollup_tokens_locked gauge
citrea_rollup_tokens_locked $(hex2dec "$tok_hex")
# HELP citrea_deployer_balance_wei Deployer account ETH balance
# TYPE citrea_deployer_balance_wei gauge
citrea_deployer_balance_wei $(hex2dec "$dep_hex")
EOF
}

# Start Python HTTP server in background
python3 -m http.server "$PORT" --directory "$METRICS_DIR" &
HTTP_PID=$!
echo "HTTP server started on :$PORT (PID $HTTP_PID)"

# Collection loop
while true; do
    collect 2>/dev/null || echo "Collection failed, retrying..."
    sleep 5
done
