#!/usr/bin/env bash
set -euo pipefail

CITREA_RPC="${CITREA_RPC:-http://citrea:12345}"

echo "=== Contract deployer (REAL Honk verifier) ==="
echo "Waiting for Citrea at ${CITREA_RPC}..."

for i in $(seq 1 60); do
    if curl -sf "$CITREA_RPC" -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        > /dev/null 2>&1; then
        echo "Citrea is up (attempt $i)."
        break
    fi
    sleep 1
done

sleep 3

echo "Deploying contracts..."
cd /app/citrea

export TESTING_URL="$CITREA_RPC"
export SECRET_KEY="ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
# DEV_USE_NOOP_VERIFIER intentionally NOT exported -> real Honk verifier

OUTPUT=$(npx hardhat run scripts/deploy.ts --network citreaDevnet 2>&1)
echo "$OUTPUT"

DEPLOY_JSON=$(echo "$OUTPUT" | grep "DEPLOY_OUTPUT=" | sed 's/DEPLOY_OUTPUT=//')
ROLLUP_PROXY=$(echo "$DEPLOY_JSON" | jq -r '.rollupProxy')
ERC20_ADDR=$(echo "$DEPLOY_JSON" | jq -r '.erc20')
VERIFIER_ADDR=$(echo "$DEPLOY_JSON" | jq -r '.verifier')

echo "Rollup proxy:  $ROLLUP_PROXY"
echo "ERC20:         $ERC20_ADDR"
echo "Verifier:      $VERIFIER_ADDR (real Honk)"

mkdir -p /deploy-output
cat > /deploy-output/env.sh << EOF
export ROLLUP_PROXY="$ROLLUP_PROXY"
export ERC20_ADDR="$ERC20_ADDR"
export VERIFIER_ADDR="$VERIFIER_ADDR"
export CITREA_RPC="$CITREA_RPC"
EOF

echo "$DEPLOY_JSON" > /deploy-output/addresses.json
echo "=== Deployment complete ==="
