#!/usr/bin/env bash
set -euo pipefail

echo "=== Ciphera PROVER startup ==="
source /deploy-output/env.sh
echo "Using rollup contract: $ROLLUP_PROXY"

sed "s|PLACEHOLDER|${ROLLUP_PROXY}|g" /app/testenv/config/prover.toml > /tmp/prover.toml

# Resolve validator hostname to IP for libp2p multiaddr
VALIDATOR_IP=$(getent hosts ciphera-validator | awk '{print $1}')
echo "Validator IP: $VALIDATOR_IP"

cd /app
if [[ ! -f target/release/node ]]; then
    echo "Building Ciphera node..."
    cargo build --release -p node
fi

echo "Starting prover (real ZK proofs via Barretenberg)..."
exec ./target/release/node \
    -c /tmp/prover.toml \
    --mode prover \
    --evm-rpc-url "http://citrea:12345" \
    --rollup-contract-addr "$ROLLUP_PROXY" \
    --p2p-dial "/ip4/${VALIDATOR_IP}/tcp/5000"
