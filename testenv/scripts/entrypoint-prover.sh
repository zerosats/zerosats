#!/usr/bin/env bash
set -euo pipefail

echo "=== Ciphera PROVER startup ==="
source /deploy-output/env.sh
echo "Using rollup contract: $ROLLUP_PROXY"

sed "s|PLACEHOLDER|${ROLLUP_PROXY}|g" /app/testenv/config/prover.toml > /tmp/prover.toml

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
    --rollup-contract-addr "$ROLLUP_PROXY"
