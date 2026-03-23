#!/usr/bin/env bash
set -euo pipefail

echo "=== Ciphera VALIDATOR startup ==="
source /deploy-output/env.sh
echo "Using rollup contract: $ROLLUP_PROXY"

sed "s|PLACEHOLDER|${ROLLUP_PROXY}|g" /app/testenv/config/validator.toml > /tmp/validator.toml

cd /app
if [[ ! -f target/release/node ]]; then
    echo "Building Ciphera node..."
    cargo build --release -p node
fi

echo "Starting validator..."
exec ./target/release/node \
    -c /tmp/validator.toml \
    --mode validator \
    --evm-rpc-url "http://citrea:12345" \
    --rollup-contract-addr "$ROLLUP_PROXY"
