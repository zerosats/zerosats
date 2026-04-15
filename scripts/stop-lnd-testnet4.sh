#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PID_FILE="$ROOT_DIR/.local/lnd/lnd.pid"

if [[ ! -f "$PID_FILE" ]]; then
  echo "No PID file at $PID_FILE"
  exit 0
fi

pid="$(cat "$PID_FILE")"
if [[ -z "$pid" ]]; then
  echo "PID file empty; removing"
  rm -f "$PID_FILE"
  exit 0
fi

if kill -0 "$pid" >/dev/null 2>&1; then
  kill "$pid"
  sleep 1
  if kill -0 "$pid" >/dev/null 2>&1; then
    echo "lnd still running, sending SIGKILL"
    kill -9 "$pid"
  fi
  echo "Stopped lnd PID $pid"
else
  echo "Process $pid is not running"
fi

rm -f "$PID_FILE"
