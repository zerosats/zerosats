#!/usr/bin/env bash

set -euo pipefail

# Compile the program
NARGO=${NARGO:-nargo}
$NARGO compile --workspace

# REPO_ROOT=/workspace/ciphera
#REPO_ROOT=$(git rev-parse --show-toplevel)
REPO_ROOT=/workspace/ciphera
BACKEND=${BACKEND:-bb}

# Clean target
rm -r $REPO_ROOT/noir/target

# Compile the program
nargo compile --workspace

# Create the fixtures directory if it doesn't exist
mkdir -p $REPO_ROOT/fixtures/programs

# Copy the compiled programs to the fixtures directory
cp -r $REPO_ROOT/noir/target/* $REPO_ROOT/fixtures/programs/

# Create the keys directory if it doesn't exist
mkdir -p $REPO_ROOT/fixtures/keys

# Get all program names from the workspace - the ordering of these is important,
# as the hash from utxo is used in agg_utxo, and agg_utxo used in agg_agg
PROGRAMS=("signature" "points" "utxo" "agg_utxo" "agg_agg") # "migrate")

# Define which programs should use the recursive flag
RECURSIVE_PROGRAMS=("agg_agg" "agg_utxo" "utxo")

# Function to check if a program should use recursive flag
is_recursive() {
  local program_name="$1"
  for p in "${RECURSIVE_PROGRAMS[@]}"; do
    if [[ "$p" == "$program_name" ]]; then
      return 0  # True in bash
    fi
  done
  return 1  # False in bash
}

# Generate verification keys for each program
# Generate verification keys for each program
for NAME in "${PROGRAMS[@]}"; do
  oracle_hash_args=()
  if [ "$NAME" == "agg_agg" ]; then
    oracle_hash_args=("--oracle_hash" "keccak")
  fi

  echo "================"
  echo "$(echo "$NAME" | tr '[:lower:]' '[:upper:]')"
  echo "================"

  echo "Generating verification key for $NAME..."
  $BACKEND write_vk ${oracle_hash_args[@]} --scheme ultra_honk -b $REPO_ROOT/fixtures/programs/${NAME}.json -o $REPO_ROOT/fixtures/keys/ \
    && python3 -c 'import sys, json; d=sys.stdin.buffer.read(); print(json.dumps([f"0x{d[i:i+32].hex()}" for i in range(0, len(d), 32)], indent=2))' < $REPO_ROOT/fixtures/keys/vk > $REPO_ROOT/fixtures/keys/vk_fields.json \
    && mv $REPO_ROOT/fixtures/keys/{vk,${NAME}_key} && mv $REPO_ROOT/fixtures/keys/{vk_fields.json,${NAME}_key_fields.json} \
    && rm $REPO_ROOT/fixtures/keys/vk_hash

  # Print verification key hash as u256 and hex
  echo "Verification key hash for $NAME:"
  VK_HASH_OUTPUT=$(cd $REPO_ROOT && cargo run --bin vk_hash -- $REPO_ROOT/fixtures/keys/${NAME}_key_fields.json)
  echo "$VK_HASH_OUTPUT" | sed 's/^/  /'
  echo ""

  # Update agg_utxo/src/main.nr with the UTXO verification key hash
  if [ "$NAME" == "utxo" ]; then
    UTXO_VK_HASH=$(echo "$VK_HASH_OUTPUT" | grep "u256:" | cut -d' ' -f2)
    echo "Updating agg_utxo/src/main.nr with UTXO verification key hash: $UTXO_VK_HASH"
    sed -i.bak "s/global UTXO_VERIFICATION_KEY_HASH: Field = [0-9]*;/global UTXO_VERIFICATION_KEY_HASH: Field = $UTXO_VK_HASH;/" $REPO_ROOT/noir/agg_utxo/src/main.nr
    rm $REPO_ROOT/noir/agg_utxo/src/main.nr.bak
  fi

  # Update agg_agg/src/main.nr with the agg_utxo verification key hash
  if [ "$NAME" == "agg_utxo" ]; then
    AGG_UTXO_VK_HASH=$(echo "$VK_HASH_OUTPUT" | grep "u256:" | cut -d' ' -f2)
    echo "Updating agg_agg/src/main.nr with agg_utxo verification key hash: $AGG_UTXO_VK_HASH"
    sed -i.bak "s/global AGG_UTXO_VERIFICATION_KEY_HASH: Field = [0-9]*;/global AGG_UTXO_VERIFICATION_KEY_HASH: Field = $AGG_UTXO_VK_HASH;/" $REPO_ROOT/noir/agg_agg/src/main.nr
    rm $REPO_ROOT/noir/agg_agg/src/main.nr.bak
  fi

  if [ "$NAME" == "agg_agg" ]; then
    AGG_AGG_VK_HASH=$(echo "$VK_HASH_OUTPUT" | grep "u256:" | cut -d' ' -f2)
    AGG_AGG_VK_HASH_HEX=$(echo "$VK_HASH_OUTPUT" | grep "hex:" | cut -d' ' -f2)
    echo "Updating agg_agg verification key hash: $AGG_AGG_VK_HASH"
    echo "Updating citrea/scripts/deploy.ts final verification key hash: $AGG_AGG_VK_HASH"

    sed -i.bak "s/const AGG_AGG_VERIFICATION_KEY_HASH = \".*\";/const AGG_AGG_VERIFICATION_KEY_HASH = \"$AGG_AGG_VK_HASH\";/" $REPO_ROOT/citrea/scripts/deploy.ts
    rm $REPO_ROOT/citrea/scripts/deploy.ts.bak

    sed -i.bak "s/pub const AGG_AGG_VERIFICATION_KEY_HASH.*;/pub const AGG_AGG_VERIFICATION_KEY_HASH: &str = \"$AGG_AGG_VK_HASH_HEX\";/" $REPO_ROOT/pkg/contracts/src/rollup.rs
    rm $REPO_ROOT/pkg/contracts/src/rollup.rs.bak

    $BACKEND write_solidity_verifier --scheme ultra_honk -k $REPO_ROOT/fixtures/keys/${NAME}_key -o $REPO_ROOT/citrea/noir/${NAME}.sol
    if [[ "$(uname)" == "Darwin" ]]; then
      SOLC=$REPO_ROOT/fixtures/binaries/solc-v0.8.29-macos
    else
      SOLC=$REPO_ROOT/fixtures/binaries/solc-v0.8.29-linux
    fi


    SOLC_INPUT=$(mktemp)
    cat <<EOF > "$SOLC_INPUT"
{
  "language": "Solidity",
  "sources": {
    "agg_agg.sol": {
      "urls": ["citrea/noir/$NAME.sol"]
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

    SOLC_OUTPUT=$(mktemp)
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
    printf '%s' "$HONK_BYTECODE" > "$REPO_ROOT/citrea/contracts/noir/${NAME}_HonkVerifier.bin"

    LIB_BYTECODE=$(jq -r ".contracts[\"$SOURCE_KEY\"][\"ZKTranscriptLib\"].evm.bytecode.object" "$SOLC_OUTPUT")
    if [[ "$LIB_BYTECODE" == "null" || -z "$LIB_BYTECODE" ]]; then
      echo "Failed to extract ZKTranscriptLib bytecode from solc output" >&2
      exit 1
    fi
    printf '%s' "$LIB_BYTECODE" > "$REPO_ROOT/citrea/contracts/noir/${NAME}_ZKTranscriptLib.bin"

    jq ".contracts[\"$SOURCE_KEY\"][\"HonkVerifier\"].evm.bytecode.linkReferences" "$SOLC_OUTPUT" > "$REPO_ROOT/citrea/contracts/noir/${NAME}_HonkVerifier.linkrefs.json"

    rm "$SOLC_INPUT" "$SOLC_OUTPUT"
  fi

done

echo "Successfully copied compiled programs to fixtures/keys/programs"
