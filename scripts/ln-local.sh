#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LNCLI_BIN="$ROOT_DIR/bin/lncli"
LND_DIR="$ROOT_DIR/.local/lnd"

[[ -x "$LNCLI_BIN" ]] || {
  echo "lncli binary not found at $LNCLI_BIN" >&2
  echo "Run: ./scripts/install-lnd-testnet4.sh" >&2
  exit 1
}

"$LNCLI_BIN" \
  --lnddir="$LND_DIR" \
  --network=testnet4 \
  --rpcserver=127.0.0.1:10009 \
  "$@"
