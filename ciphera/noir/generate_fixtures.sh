#!/usr/bin/env bash

set -euo pipefail

NARGO=${NARGO:-nargo}

# Detect repo root: in Docker, ciphera/ contents are at the git root.
# Locally, they're under a ciphera/ subdirectory.
GIT_ROOT=$(git rev-parse --show-toplevel)
if [ -d "$GIT_ROOT/noir" ]; then
  REPO_ROOT="$GIT_ROOT"
elif [ -d "$GIT_ROOT/ciphera/noir" ]; then
  REPO_ROOT="$GIT_ROOT/ciphera"
else
  echo "ERROR: Cannot find noir/ directory under $GIT_ROOT or $GIT_ROOT/ciphera/"
  exit 1
fi
BACKEND=${BACKEND:-bb}

# Clean target so every artifact in this run is fresh.
rm -rf "$REPO_ROOT/noir/target"

mkdir -p "$REPO_ROOT/fixtures/programs"
mkdir -p "$REPO_ROOT/fixtures/keys"

# ----------------------------------------------------------------------
# Helpers
# ----------------------------------------------------------------------

# Replace a Noir global Field constant's value in-place.
# Usage: update_noir_hash <file> <global_name> <new_value>
#
# Uses -0777 (slurp the whole file) so the substitution survives a
# `nargo fmt`-induced newline between `=` and the literal -- the
# fixed-length lookbehind form silently no-ops on wrapped declarations
# (same failure mode as the Rust/TS perl calls below).
update_noir_hash() {
  local file="$1" var="$2" hash="$3"
  perl -i -0777 -pe "s/(global ${var}:\s*Field\s*=\s*)\d+(\s*;)/\${1}${hash}\${2}/" "$file"
}

# Extract the u256 decimal hash from vk_hash output.
extract_u256() { awk '/^u256:/ { print $2 }'; }

# Extract the hex hash from vk_hash output.
extract_hex() { awk '/^hex:/ { print $2 }'; }

# Compile a single Noir package and copy the resulting artifact into the
# fixtures/programs directory.
compile_package() {
  local name="$1"
  echo "================"
  echo "COMPILE $name"
  echo "================"
  $NARGO compile --package "$name"
  cp "$REPO_ROOT/noir/target/${name}.json" "$REPO_ROOT/fixtures/programs/${name}.json"
}

# Generate the verification key for a compiled package and rename it to
# the canonical `<name>_key` filename. Extra arguments are forwarded to
# `bb write_vk` (e.g. `--oracle_hash keccak` for the on-chain proof).
write_vk_for() {
  local name="$1"; shift
  echo "================"
  echo "WRITE_VK $name"
  echo "================"
  $BACKEND write_vk --verifier_type standalone "$@" --scheme ultra_honk \
    -b "$REPO_ROOT/fixtures/programs/${name}.json" \
    -o "$REPO_ROOT/fixtures/keys/"
  mv "$REPO_ROOT/fixtures/keys/vk" "$REPO_ROOT/fixtures/keys/${name}_key"
  rm "$REPO_ROOT/fixtures/keys/vk_hash"
}

# Compute the u256 + hex hash of a verification key.
vk_hash_for() {
  local name="$1"
  (cd "$REPO_ROOT" && cargo run --quiet --bin vk_hash -- \
    "$REPO_ROOT/fixtures/keys/${name}_key")
}

# Portable thousand-separator: locales with grouping aren't guaranteed
# inside the build image, so do it with sed. Used by both the circuits
# table and the badge generation step downstream.
fmt_thousands() {
  echo "$1" | sed -E ':a;s/([0-9])([0-9]{3})(\b|,)/\1,\2\3/;ta'
}

# Per-circuit complexity stats, populated as each circuit is compiled.
# ACIR_OPS  : Noir-level ACIR opcode count    (what `nargo compile` produces)
# HONK_GATES: UltraHonk circuit size in gates (proving-time complexity)
# VK_BYTES  : size of the on-disk verification key, in bytes
declare -A ACIR_OPS
declare -A HONK_GATES
declare -A VK_BYTES

# Capture static complexity for a compiled package. The same `bb gates`
# invocation must use the oracle hash the verifier expects, otherwise
# the gate count for agg_agg (keccak) differs from the figure printed
# in the on-chain verifier path.
record_circuit_stats() {
  local name="$1"; shift
  local stats
  stats=$($BACKEND gates --scheme ultra_honk "$@" \
    -b "$REPO_ROOT/fixtures/programs/${name}.json" 2>/dev/null \
    | jq -r '.functions[0] | "\(.acir_opcodes) \(.circuit_size)"')
  if [[ -z "$stats" || "$stats" == "null null" ]]; then
    echo "WARN: could not extract gate stats for $name" >&2
    ACIR_OPS[$name]="?"
    HONK_GATES[$name]="?"
  else
    ACIR_OPS[$name]="${stats%% *}"
    HONK_GATES[$name]="${stats##* }"
  fi
  VK_BYTES[$name]=$(wc -c < "$REPO_ROOT/fixtures/keys/${name}_key" | tr -d ' ')
}

# ----------------------------------------------------------------------
# Stage 1 — leaf circuits and the inner-most recursion target (utxo).
#
# None of these source files contain a propagated VK-hash constant, so
# they can all be compiled before any hash is known.
# ----------------------------------------------------------------------

LEAF_PROGRAMS=("signature" "signature32" "signature32sha" "utxo")
for name in "${LEAF_PROGRAMS[@]}"; do
  compile_package "$name"
  write_vk_for "$name"
  record_circuit_stats "$name"
done

# ----------------------------------------------------------------------
# Stage 2 — propagate utxo's VK hash into agg_utxo's source BEFORE
# compiling agg_utxo. Doing this in the old order meant agg_utxo.json
# was always one run behind on UTXO_VERIFICATION_KEY_HASH.
# ----------------------------------------------------------------------

UTXO_VK_OUT=$(vk_hash_for utxo)
echo "Verification key hash for utxo:"
echo "$UTXO_VK_OUT" | sed 's/^/  /'
UTXO_VK_HASH=$(echo "$UTXO_VK_OUT" | extract_u256)
echo "Updating noir/agg_utxo/src/main.nr UTXO_VERIFICATION_KEY_HASH=$UTXO_VK_HASH"
update_noir_hash "$REPO_ROOT/noir/agg_utxo/src/main.nr" \
  UTXO_VERIFICATION_KEY_HASH "$UTXO_VK_HASH"

compile_package agg_utxo
write_vk_for agg_utxo
record_circuit_stats agg_utxo

# ----------------------------------------------------------------------
# Stage 3 — propagate agg_utxo's VK hash into agg_agg's source BEFORE
# compiling agg_agg.
# ----------------------------------------------------------------------

AGG_UTXO_VK_OUT=$(vk_hash_for agg_utxo)
echo "Verification key hash for agg_utxo:"
echo "$AGG_UTXO_VK_OUT" | sed 's/^/  /'
AGG_UTXO_VK_HASH=$(echo "$AGG_UTXO_VK_OUT" | extract_u256)
echo "Updating noir/agg_agg/src/main.nr AGG_UTXO_VERIFICATION_KEY_HASH=$AGG_UTXO_VK_HASH"
update_noir_hash "$REPO_ROOT/noir/agg_agg/src/main.nr" \
  AGG_UTXO_VERIFICATION_KEY_HASH "$AGG_UTXO_VK_HASH"

compile_package agg_agg
# agg_agg is verified on-chain by Solidity, so use the keccak oracle.
write_vk_for agg_agg --oracle_hash keccak
record_circuit_stats agg_agg --oracle_hash keccak

# ----------------------------------------------------------------------
# Stage 4 — propagate agg_agg's VK hash into the Rust + TypeScript
# consumers, then build the Solidity verifier.
# ----------------------------------------------------------------------

AGG_AGG_VK_OUT=$(vk_hash_for agg_agg)
echo "Verification key hash for agg_agg:"
echo "$AGG_AGG_VK_OUT" | sed 's/^/  /'
AGG_AGG_VK_HASH_HEX=$(echo "$AGG_AGG_VK_OUT" | extract_hex)

echo "Updating citrea/scripts/deploy.ts AGG_AGG_VERIFICATION_KEY_HASH=$AGG_AGG_VK_HASH_HEX"
echo "Updating pkg/contracts/src/rollup.rs AGG_AGG_VERIFICATION_KEY_HASH=$AGG_AGG_VK_HASH_HEX"

# Use -0777 (slurp the whole file) so the substitution survives a
# rustfmt/prettier-induced newline between `=` and `"`. The lookbehind
# form fails silently on multi-line declarations -- exactly the failure
# mode that left pkg/contracts/src/rollup.rs frozen at a stale hash.
perl -i -0777 -pe "s/(const AGG_AGG_VERIFICATION_KEY_HASH\s*=\s*\")[^\"]*(\")/\${1}${AGG_AGG_VK_HASH_HEX}\${2}/" \
  "$REPO_ROOT/citrea/scripts/deploy.ts"
perl -i -0777 -pe "s/(AGG_AGG_VERIFICATION_KEY_HASH:\s*&str\s*=\s*\")[^\"]*(\")/\${1}${AGG_AGG_VK_HASH_HEX}\${2}/" \
  "$REPO_ROOT/pkg/contracts/src/rollup.rs"

$BACKEND write_solidity_verifier --scheme ultra_honk \
  -k "$REPO_ROOT/fixtures/keys/agg_agg_key" \
  -o "$REPO_ROOT/citrea/noir/agg_agg.sol"

if [[ "$(uname)" == "Darwin" ]]; then
  SOLC="$REPO_ROOT/fixtures/binaries/solc-v0.8.29-macos"
else
  SOLC="$REPO_ROOT/fixtures/binaries/solc-v0.8.29-linux"
fi

SOLC_INPUT=$(mktemp)
SOLC_OUTPUT=$(mktemp)
trap 'rm -f "$SOLC_INPUT" "$SOLC_OUTPUT"' EXIT

cat <<EOF > "$SOLC_INPUT"
{
  "language": "Solidity",
  "sources": {
    "agg_agg.sol": {
      "urls": ["citrea/noir/agg_agg.sol"]
    }
  },
  "settings": {
    "optimizer": { "enabled": true, "runs": 0 },
    "debug": { "revertStrings": "strip" },
    "outputSelection": {
      "*": {
        "*": ["evm.bytecode", "evm.deployedBytecode"],
        "": ["id"]
      }
    }
  }
}
EOF

(cd "$REPO_ROOT" && $SOLC --standard-json "$SOLC_INPUT") > "$SOLC_OUTPUT"

SOURCE_KEY=$(jq -r '.contracts | keys[0]' "$SOLC_OUTPUT")
if [[ "$SOURCE_KEY" == "null" ]]; then
  echo "Failed to determine source key from solc output" >&2
  exit 1
fi

HONK_BYTECODE=$(jq -r ".contracts[\"$SOURCE_KEY\"][\"HonkVerifier\"].evm.bytecode.object" "$SOLC_OUTPUT")
if [[ "$HONK_BYTECODE" == "null" || -z "$HONK_BYTECODE" ]]; then
  echo "Failed to extract HonkVerifier bytecode from solc output" >&2
  exit 1
fi
printf '%s' "$HONK_BYTECODE" > "$REPO_ROOT/citrea/contracts/noir/agg_agg_HonkVerifier.bin"

LIB_BYTECODE=$(jq -r ".contracts[\"$SOURCE_KEY\"][\"ZKTranscriptLib\"].evm.bytecode.object" "$SOLC_OUTPUT")
if [[ "$LIB_BYTECODE" == "null" || -z "$LIB_BYTECODE" ]]; then
  echo "Failed to extract ZKTranscriptLib bytecode from solc output" >&2
  exit 1
fi
printf '%s' "$LIB_BYTECODE" > "$REPO_ROOT/citrea/contracts/noir/agg_agg_ZKTranscriptLib.bin"

jq ".contracts[\"$SOURCE_KEY\"][\"HonkVerifier\"].evm.bytecode.linkReferences" "$SOLC_OUTPUT" \
  > "$REPO_ROOT/citrea/contracts/noir/agg_agg_HonkVerifier.linkrefs.json"

# ----------------------------------------------------------------------
# Stage 5 — refresh the Circuits block in the root README so the table
# always reflects the freshly-minted verification keys. The block lives
# between `<!-- circuits:start -->` and `<!-- circuits:end -->` markers
# so it can be rewritten atomically without disturbing surrounding prose.
# ----------------------------------------------------------------------

declare -A CIRCUIT_ROLE=(
  [signature]="Signature leaf"
  [signature32]="Signature leaf (32-byte message)"
  [signature32sha]="Signature leaf (sha256 message)"
  [utxo]="UTXO leaf (recursion base)"
  [agg_utxo]="1-level aggregator"
  [agg_agg]="Top-level aggregator (on-chain verifier)"
)
declare -A CIRCUIT_ORACLE=(
  [signature]="poseidon"
  [signature32]="poseidon"
  [signature32sha]="poseidon"
  [utxo]="poseidon"
  [agg_utxo]="poseidon"
  [agg_agg]="keccak"
)

# Reuse hashes that earlier stages already computed; only fetch fresh
# ones for the leaf signature circuits that were not previously needed
# for VK-hash propagation.
declare -A CIRCUIT_VK_OUT=(
  [utxo]="$UTXO_VK_OUT"
  [agg_utxo]="$AGG_UTXO_VK_OUT"
  [agg_agg]="$AGG_AGG_VK_OUT"
)

CIRCUIT_ORDER=(signature signature32 signature32sha utxo agg_utxo agg_agg)

README_BLOCK=$(mktemp)
trap 'rm -f "$SOLC_INPUT" "$SOLC_OUTPUT" "$README_BLOCK"' EXIT

GENERATED_AT=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
{
  echo "<!-- circuits:start -->"
  echo ""
  echo "_Auto-generated by \`noir/generate_fixtures.sh\` — last refreshed ${GENERATED_AT}._"
  echo ""
  echo "| Circuit | Role | Oracle | ACIR opcodes | UltraHonk gates | VK bytes | Verification key (hex) |"
  echo "|---------|------|--------|-------------:|----------------:|---------:|------------------------|"
  for name in "${CIRCUIT_ORDER[@]}"; do
    vk_out="${CIRCUIT_VK_OUT[$name]:-}"
    if [[ -z "$vk_out" ]]; then
      vk_out=$(vk_hash_for "$name")
    fi
    hex_hash=$(echo "$vk_out" | extract_hex)
    short="${hex_hash:0:10}…${hex_hash: -8}"
    acir_fmt=$(fmt_thousands "${ACIR_OPS[$name]:-?}")
    gates_fmt=$(fmt_thousands "${HONK_GATES[$name]:-?}")
    vk_fmt=$(fmt_thousands "${VK_BYTES[$name]:-?}")
    echo "| \`$name\` | ${CIRCUIT_ROLE[$name]} | ${CIRCUIT_ORACLE[$name]} | ${acir_fmt} | ${gates_fmt} | ${vk_fmt} | \`$short\` |"
  done
  echo ""
  echo "**ACIR opcodes** are the Noir-frontend instruction count; **UltraHonk gates** is the prover-time circuit size after black-box expansion. Wall-clock prove/verify timings are machine-dependent — see \`cargo bench -p zk-circuits\` for canonical numbers on tracked hardware."
  echo ""
  echo "<!-- circuits:end -->"
} > "$README_BLOCK"

BLOCK_FILE="$README_BLOCK" perl -i -0777 -pe '
  open(my $fh, "<", $ENV{BLOCK_FILE}) or die "cannot open block file: $!";
  local $/; my $block = <$fh>; close $fh;
  chomp $block;
  unless (s|<!-- circuits:start -->.*?<!-- circuits:end -->|$block|s) {
    die "circuits markers not found in README; expected <!-- circuits:start --> ... <!-- circuits:end -->";
  }
' "$REPO_ROOT/README.md"

echo "📝 Updated Circuits section in $REPO_ROOT/README.md"

# Refresh the dynamic complexity badges. shields.io path syntax: spaces
# render with %20, underscores must be doubled, and we encode the
# thousand-separator comma as %2C so the badge text reads naturally.
BADGE_BLOCK=$(mktemp)
trap 'rm -f "$SOLC_INPUT" "$SOLC_OUTPUT" "$README_BLOCK" "$BADGE_BLOCK"' EXIT

badge_value() {
  fmt_thousands "$1" | sed 's/,/%2C/g'
}

ACIR_BADGE_VAL=$(badge_value "${ACIR_OPS[utxo]:-0}")
GATES_BADGE_VAL=$(badge_value "${HONK_GATES[utxo]:-0}")
VK_BADGE_VAL=$(badge_value "${VK_BYTES[utxo]:-0}")

{
  echo "<!-- badges:start -->"
  echo "![Utxo ACIR](https://img.shields.io/badge/utxo%20ACIR-${ACIR_BADGE_VAL}-0066ff)"
  echo "![Utxo gates](https://img.shields.io/badge/utxo%20gates-${GATES_BADGE_VAL}-1f6feb)"
  echo "![Utxo VK bytes](https://img.shields.io/badge/utxo%20VK%20bytes-${VK_BADGE_VAL}-7c3aed)"
  echo "<!-- badges:end -->"
} > "$BADGE_BLOCK"

BLOCK_FILE="$BADGE_BLOCK" perl -i -0777 -pe '
  open(my $fh, "<", $ENV{BLOCK_FILE}) or die "cannot open badge block file: $!";
  local $/; my $block = <$fh>; close $fh;
  chomp $block;
  unless (s|<!-- badges:start -->.*?<!-- badges:end -->|$block|s) {
    die "badges markers not found in README; expected <!-- badges:start --> ... <!-- badges:end -->";
  }
' "$REPO_ROOT/README.md"

echo "🏷️  Updated complexity badges in $REPO_ROOT/README.md"

echo "Successfully generated programs, keys, and Solidity verifier"
