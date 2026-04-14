#!/usr/bin/env bash
set -euo pipefail

# Tiny lncli helper for testnet4 workflows.
# No external dependencies (no jq, no Python).
#
# Defaults:
# - lncli binary: lncli
# - network flag: testnet (for testnet4-enabled LND builds)
#
# Optional env overrides:
#   LNCLI_BIN=lncli
#   LNCLI_NETWORK=testnet
#   LNCLI_RPCSERVER=127.0.0.1:10009
#   LNCLI_MACAROON_PATH=/path/to/admin.macaroon
#   LNCLI_TLSCERT_PATH=/path/to/tls.cert

LNCLI_BIN="${LNCLI_BIN:-lncli}"
LNCLI_NETWORK="${LNCLI_NETWORK:-testnet}"
LNCLI_RPCSERVER="${LNCLI_RPCSERVER:-}"
LNCLI_MACAROON_PATH="${LNCLI_MACAROON_PATH:-}"
LNCLI_TLSCERT_PATH="${LNCLI_TLSCERT_PATH:-}"

die() {
  echo "error: $*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  ./scripts/ln-testnet4.sh doctor
  ./scripts/ln-testnet4.sh info
  ./scripts/ln-testnet4.sh balances
  ./scripts/ln-testnet4.sh channels
  ./scripts/ln-testnet4.sh decode <invoice>
  ./scripts/ln-testnet4.sh pay <invoice> [extra lncli payinvoice args...]
  ./scripts/ln-testnet4.sh pay-pwa <pwa_json_file> [extra lncli payinvoice args...]
  ./scripts/ln-testnet4.sh track <payment_hash>
  ./scripts/ln-testnet4.sh list-payments
  ./scripts/ln-testnet4.sh invoice <amount_sats> [memo]
  ./scripts/ln-testnet4.sh peers
  ./scripts/ln-testnet4.sh add-peer <pubkey@host:port>

Examples:
  ./scripts/ln-testnet4.sh doctor
  ./scripts/ln-testnet4.sh decode 'lntb100u1...'
  ./scripts/ln-testnet4.sh pay 'lntb100u1...'
  ./scripts/ln-testnet4.sh pay 'lntb100u1...' --timeout_seconds=120 --fee_limit_sat=200
  ./scripts/ln-testnet4.sh pay-pwa ./pwa-output.json
  ./scripts/ln-testnet4.sh invoice 10000 'test invoice'

Notes:
  - This script uses lncli --network=testnet by default.
  - For testnet4, ensure your LND node itself is configured for testnet4.
EOF
}

ln() {
  local cmd=("$LNCLI_BIN" "--network=$LNCLI_NETWORK")
  [[ -n "$LNCLI_RPCSERVER" ]] && cmd+=("--rpcserver=$LNCLI_RPCSERVER")
  [[ -n "$LNCLI_MACAROON_PATH" ]] && cmd+=("--macaroonpath=$LNCLI_MACAROON_PATH")
  [[ -n "$LNCLI_TLSCERT_PATH" ]] && cmd+=("--tlscertpath=$LNCLI_TLSCERT_PATH")
  "${cmd[@]}" "$@"
}

require_lncli() {
  command -v "$LNCLI_BIN" >/dev/null 2>&1 || die "lncli binary not found: $LNCLI_BIN"
}

extract_json_string_field() {
  # Minimal JSON string extractor without jq.
  # Prints first match for key "field": "value".
  local field="$1"
  sed -n "s/.*\"$field\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" | head -n1
}

extract_invoice_from_pwa_json() {
  local file="$1"
  [[ -f "$file" ]] || die "file not found: $file"
  local compact
  compact="$(tr -d '\n' < "$file")"

  local invoice
  invoice="$(printf '%s' "$compact" | extract_json_string_field "invoice")"
  if [[ -z "$invoice" ]]; then
    invoice="$(printf '%s' "$compact" | extract_json_string_field "addressOrInvoice")"
  fi
  [[ -n "$invoice" ]] || die "could not find invoice/addressOrInvoice in $file"
  printf '%s\n' "$invoice"
}

print_invoice_summary() {
  local invoice="$1"
  echo "Decoding invoice..."
  ln --output json decodepayreq "$invoice"
}

doctor() {
  require_lncli
  echo "lncli: $(command -v "$LNCLI_BIN")"
  echo "network: $LNCLI_NETWORK"
  ln getinfo
}

cmd="${1:-}"
[[ -n "$cmd" ]] || { usage; exit 1; }
shift || true

case "$cmd" in
  -h|--help|help)
    usage
    ;;
  doctor)
    doctor
    ;;
  info)
    require_lncli
    ln getinfo
    ;;
  balances)
    require_lncli
    echo "== wallet balance =="
    ln walletbalance
    echo
    echo "== channel balance =="
    ln channelbalance
    ;;
  channels)
    require_lncli
    ln listchannels
    ;;
  peers)
    require_lncli
    ln listpeers
    ;;
  add-peer)
    require_lncli
    target="${1:-}"
    [[ -n "$target" ]] || die "usage: add-peer <pubkey@host:port>"
    ln connect "$target"
    ;;
  decode)
    require_lncli
    invoice="${1:-}"
    [[ -n "$invoice" ]] || die "usage: decode <invoice>"
    print_invoice_summary "$invoice"
    ;;
  pay)
    require_lncli
    invoice="${1:-}"
    [[ -n "$invoice" ]] || die "usage: pay <invoice> [extra lncli payinvoice args...]"
    shift || true
    print_invoice_summary "$invoice"
    echo
    echo "Paying invoice..."
    ln payinvoice --pay_req="$invoice" "$@"
    ;;
  pay-pwa)
    require_lncli
    file="${1:-}"
    [[ -n "$file" ]] || die "usage: pay-pwa <pwa_json_file> [extra lncli payinvoice args...]"
    shift || true
    invoice="$(extract_invoice_from_pwa_json "$file")"
    print_invoice_summary "$invoice"
    echo
    echo "Paying invoice extracted from $file ..."
    ln payinvoice --pay_req="$invoice" "$@"
    ;;
  track)
    require_lncli
    payment_hash="${1:-}"
    [[ -n "$payment_hash" ]] || die "usage: track <payment_hash>"
    ln trackpayment "$payment_hash"
    ;;
  list-payments)
    require_lncli
    ln listpayments
    ;;
  invoice)
    require_lncli
    amt="${1:-}"
    memo="${2:-}"
    [[ -n "$amt" ]] || die "usage: invoice <amount_sats> [memo]"
    [[ "$amt" =~ ^[0-9]+$ ]] || die "amount must be integer sats"
    if [[ -n "$memo" ]]; then
      ln addinvoice --amt="$amt" --memo="$memo"
    else
      ln addinvoice --amt="$amt"
    fi
    ;;
  *)
    usage
    die "unknown command: $cmd"
    ;;
esac

