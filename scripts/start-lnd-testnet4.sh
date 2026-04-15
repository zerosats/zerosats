#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LND_BIN="$ROOT_DIR/bin/lnd"
LND_DIR="$ROOT_DIR/.local/lnd"
LOG_FILE="$LND_DIR/lnd.log"
PID_FILE="$LND_DIR/lnd.pid"

[[ -x "$LND_BIN" ]] || {
  echo "lnd binary not found at $LND_BIN" >&2
  echo "Run: ./scripts/install-lnd-testnet4.sh" >&2
  exit 1
}

mkdir -p "$LND_DIR"

if [[ -f "$PID_FILE" ]]; then
  pid="$(cat "$PID_FILE")"
  if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
    echo "lnd already running with PID $pid"
    exit 0
  fi
fi

nohup "$LND_BIN" \
  --lnddir="$LND_DIR" \
  --bitcoin.testnet4 \
  --bitcoin.node=neutrino \
  --rpclisten=127.0.0.1:10009 \
  --restlisten=127.0.0.1:8080 \
  --listen=127.0.0.1:9735 \
  --debuglevel=info \
  >"$LOG_FILE" 2>&1 &

echo $! >"$PID_FILE"
sleep 1

pid="$(cat "$PID_FILE")"
if kill -0 "$pid" >/dev/null 2>&1; then
  echo "lnd started (PID $pid)"
  echo "log: $LOG_FILE"
else
  echo "lnd failed to start; check log: $LOG_FILE" >&2
  exit 1
fi
