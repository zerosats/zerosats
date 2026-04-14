#!/usr/bin/env bash
set -euo pipefail

# Tiny LNBits helper for testnet4 workflows.
# No external deps (curl only).
#
# Env:
#   LNBITS_URL=https://lnbits-testnet4.atomiqlabs.org:12345
#   LNBITS_ADMIN_KEY=...
#   LNBITS_INVOICE_KEY=...
#
# Optional:
#   LNBITS_WALLET_ID=...

LNBITS_URL="${LNBITS_URL:-https://lnbits-testnet4.atomiqlabs.org:12345}"
LNBITS_ADMIN_KEY="${LNBITS_ADMIN_KEY:-}"
LNBITS_INVOICE_KEY="${LNBITS_INVOICE_KEY:-}"
LNBITS_WALLET_ID="${LNBITS_WALLET_ID:-}"

die() {
  echo "error: $*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  ./scripts/lnbits-testnet4.sh doctor
  ./scripts/lnbits-testnet4.sh wallet [admin|invoice]
  ./scripts/lnbits-testnet4.sh create-invoice <amount_sats> [memo]
  ./scripts/lnbits-testnet4.sh pay <bolt11>
  ./scripts/lnbits-testnet4.sh pay-pwa <json_file_with_invoice_or_addressOrInvoice>
  ./scripts/lnbits-testnet4.sh status <checking_id_or_payment_hash>
  ./scripts/lnbits-testnet4.sh list [limit]
  ./scripts/lnbits-testnet4.sh decode <bolt11>
  ./scripts/lnbits-testnet4.sh balance

Examples:
  export LNBITS_ADMIN_KEY='...'
  export LNBITS_INVOICE_KEY='...'
  ./scripts/lnbits-testnet4.sh wallet admin
  ./scripts/lnbits-testnet4.sh create-invoice 1000 "test"
  ./scripts/lnbits-testnet4.sh pay 'lntb...'
  ./scripts/lnbits-testnet4.sh status '<checking_id_or_hash>'
EOF
}

require_curl() {
  command -v curl >/dev/null 2>&1 || die "curl not found"
}

require_admin_key() {
  [[ -n "$LNBITS_ADMIN_KEY" ]] || die "LNBITS_ADMIN_KEY is required"
}

require_invoice_key() {
  [[ -n "$LNBITS_INVOICE_KEY" ]] || die "LNBITS_INVOICE_KEY is required"
}

api_get() {
  local key="$1"
  local path="$2"
  curl -sS "$LNBITS_URL$path" \
    -H "X-Api-Key: $key"
}

api_post_json() {
  local key="$1"
  local path="$2"
  local body="$3"
  curl -sS -X POST "$LNBITS_URL$path" \
    -H "Content-Type: application/json" \
    -H "X-Api-Key: $key" \
    -d "$body"
}

extract_json_string_field() {
  local field="$1"
  sed -n "s/.*\"$field\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" | head -n1
}

extract_invoice_from_file() {
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

cmd="${1:-}"
[[ -n "$cmd" ]] || { usage; exit 1; }
shift || true

require_curl

case "$cmd" in
  -h|--help|help)
    usage
    ;;
  doctor)
    echo "url: $LNBITS_URL"
    if [[ -n "$LNBITS_ADMIN_KEY" ]]; then
      echo "admin key: set"
    else
      echo "admin key: missing"
    fi
    if [[ -n "$LNBITS_INVOICE_KEY" ]]; then
      echo "invoice key: set"
    else
      echo "invoice key: missing"
    fi
    ;;
  wallet)
    mode="${1:-admin}"
    if [[ "$mode" == "admin" ]]; then
      require_admin_key
      api_get "$LNBITS_ADMIN_KEY" "/api/v1/wallet"
    elif [[ "$mode" == "invoice" ]]; then
      require_invoice_key
      api_get "$LNBITS_INVOICE_KEY" "/api/v1/wallet"
    else
      die "wallet mode must be admin or invoice"
    fi
    ;;
  balance)
    require_admin_key
    api_get "$LNBITS_ADMIN_KEY" "/api/v1/wallet"
    ;;
  create-invoice)
    require_invoice_key
    amount="${1:-}"
    memo="${2:-}"
    [[ -n "$amount" ]] || die "usage: create-invoice <amount_sats> [memo]"
    [[ "$amount" =~ ^[0-9]+$ ]] || die "amount must be integer sats"
    if [[ -n "$memo" ]]; then
      api_post_json "$LNBITS_INVOICE_KEY" "/api/v1/payments" \
        "{\"out\":false,\"amount\":$amount,\"memo\":\"$memo\"}"
    else
      api_post_json "$LNBITS_INVOICE_KEY" "/api/v1/payments" \
        "{\"out\":false,\"amount\":$amount}"
    fi
    ;;
  decode)
    require_invoice_key
    bolt11="${1:-}"
    [[ -n "$bolt11" ]] || die "usage: decode <bolt11>"
    api_get "$LNBITS_INVOICE_KEY" "/api/v1/payments/decode/$bolt11"
    ;;
  pay)
    require_admin_key
    bolt11="${1:-}"
    [[ -n "$bolt11" ]] || die "usage: pay <bolt11>"
    api_post_json "$LNBITS_ADMIN_KEY" "/api/v1/payments" \
      "{\"out\":true,\"bolt11\":\"$bolt11\"}"
    ;;
  pay-pwa)
    require_admin_key
    file="${1:-}"
    [[ -n "$file" ]] || die "usage: pay-pwa <json_file_with_invoice_or_addressOrInvoice>"
    invoice="$(extract_invoice_from_file "$file")"
    api_post_json "$LNBITS_ADMIN_KEY" "/api/v1/payments" \
      "{\"out\":true,\"bolt11\":\"$invoice\"}"
    ;;
  status)
    require_admin_key
    checking_id="${1:-}"
    [[ -n "$checking_id" ]] || die "usage: status <checking_id_or_payment_hash>"
    api_get "$LNBITS_ADMIN_KEY" "/api/v1/payments/$checking_id"
    ;;
  list)
    require_admin_key
    limit="${1:-20}"
    [[ "$limit" =~ ^[0-9]+$ ]] || die "limit must be integer"
    api_get "$LNBITS_ADMIN_KEY" "/api/v1/payments?limit=$limit"
    ;;
  *)
    usage
    die "unknown command: $cmd"
    ;;
esac
