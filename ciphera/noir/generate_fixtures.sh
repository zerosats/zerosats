#!/usr/bin/env bash

set -euo pipefail

# Compile the program
NARGO=${NARGO:-nargo}

# REPO_ROOT=/workspace/ciphera
REPO_ROOT=$(git rev-parse --show-toplevel)
BACKEND=${BACKEND:-bb}

# Clean target
rm -r $REPO_ROOT/noir/target

# Compile the program
$NARGO compile --workspace

# Create the fixtures directory if it doesn't exist
mkdir -p $REPO_ROOT/fixtures/programs

# Copy the compiled programs to the fixtures directory
cp -r $REPO_ROOT/noir/target/* $REPO_ROOT/fixtures/programs/

# Create the keys directory if it doesn't exist
mkdir -p $REPO_ROOT/fixtures/keys

# Get all program names from the workspace - the ordering of these is important,
# as the hash from utxo is used in agg_utxo, and agg_utxo used in agg_agg
PROGRAMS=("utxo" "agg_utxo" "agg_agg" "signature" "points" ) # "migrate")

# Define which programs need noir-recursive verifier target
RECURSIVE_PROGRAMS=("agg_utxo" "utxo")

# Function to get the verifier target for a program
get_verifier_target() {
  local program_name="$1"
  if [[ "$program_name" == "agg_agg" ]]; then
    echo "evm"
    return
  fi
  for p in "${RECURSIVE_PROGRAMS[@]}"; do
    if [[ "$p" == "$program_name" ]]; then
      echo "noir-recursive"
      return
    fi
  done
  echo ""
}

# Generate verification keys for each program
for NAME in "${PROGRAMS[@]}"; do
  TARGET=$(get_verifier_target "$NAME")
  target_args=()
  if [ -n "$TARGET" ]; then
    target_args=("--verifier_target" "$TARGET")
  fi

  echo "================"
  echo "$(echo "$NAME" | tr '[:lower:]' '[:upper:]')"
  echo "================"

  echo "Generating verification key for $NAME (target: ${TARGET:-default})"
  $BACKEND write_vk ${target_args[@]+"${target_args[@]}"} -b $REPO_ROOT/fixtures/programs/${NAME}.json -o $REPO_ROOT/fixtures/keys/ \
    && mv $REPO_ROOT/fixtures/keys/{vk,${NAME}_key} && mv $REPO_ROOT/fixtures/keys/{vk_hash,${NAME}_vk_hash}

  # Print verification key hash
  echo "Verification key hash for $NAME:"
  VK_HASH_HEX=$(xxd -p -c 64 $REPO_ROOT/fixtures/keys/${NAME}_vk_hash)
  echo "  hex: 0x$VK_HASH_HEX"

  # Convert hash to decimal for Noir constants
  VK_HASH_DECIMAL=$(python3 -c "print(int('$VK_HASH_HEX', 16))")
  echo "  u256: $VK_HASH_DECIMAL"
  echo ""

  # Update agg_utxo/src/main.nr with the UTXO verification key hash
  if [ "$NAME" == "utxo" ]; then
    echo "Updating agg_utxo/src/main.nr with UTXO verification key hash: $VK_HASH_DECIMAL"
    sed -i.bak "s/global UTXO_VERIFICATION_KEY_HASH: Field = [0-9]*;/global UTXO_VERIFICATION_KEY_HASH: Field = $VK_HASH_DECIMAL;/" $REPO_ROOT/noir/agg_utxo/src/main.nr
    rm $REPO_ROOT/noir/agg_utxo/src/main.nr.bak

    # Recompile agg_utxo after hash update
    echo "Recompiling agg_utxo with updated VK hash..."
    (cd $REPO_ROOT/noir && $NARGO compile --package agg_utxo)
    cp $REPO_ROOT/noir/target/agg_utxo.json $REPO_ROOT/fixtures/programs/
  fi

  # Update agg_agg/src/main.nr with the agg_utxo verification key hash
  if [ "$NAME" == "agg_utxo" ]; then
    echo "Updating agg_agg/src/main.nr with agg_utxo verification key hash: $VK_HASH_DECIMAL"
    sed -i.bak "s/global AGG_UTXO_VERIFICATION_KEY_HASH: Field = [0-9]*;/global AGG_UTXO_VERIFICATION_KEY_HASH: Field = $VK_HASH_DECIMAL;/" $REPO_ROOT/noir/agg_agg/src/main.nr
    rm $REPO_ROOT/noir/agg_agg/src/main.nr.bak

    # Recompile agg_agg after hash update
    echo "Recompiling agg_agg with updated VK hash..."
    (cd $REPO_ROOT/noir && $NARGO compile --package agg_agg)
    cp $REPO_ROOT/noir/target/agg_agg.json $REPO_ROOT/fixtures/programs/
  fi

  # Generate Solidity verifier
  sol_target_args=()
  if [[ "$NAME" == "agg_agg" ]]; then
    sol_target_args=("--verifier_target" "evm")
  fi
  $BACKEND write_solidity_verifier ${sol_target_args[@]+"${sol_target_args[@]}"} -k $REPO_ROOT/fixtures/keys/${NAME}_key -o $REPO_ROOT/citrea/noir/${NAME}.sol
  sed -i.bak 's/external pure/internal pure/g' $REPO_ROOT/citrea/noir/${NAME}.sol
  rm $REPO_ROOT/citrea/noir/${NAME}.sol.bak
  if [[ "$(uname)" == "Darwin" ]]; then
    SOLC=$REPO_ROOT/fixtures/binaries/solc-v0.8.29-macos
  else
    SOLC=$REPO_ROOT/fixtures/binaries/solc-v0.8.29-linux
  fi
  $SOLC --combined-json bin --revert-strings strip --optimize --optimize-runs 1 $REPO_ROOT/citrea/noir/$NAME.sol | jq -r ".contracts[\"$REPO_ROOT/citrea/noir/$NAME.sol:HonkVerifier\"].bin" > $REPO_ROOT/citrea/contracts/noir/${NAME}_HonkVerifier.bin
done

# Propagate agg_agg VK hash to deployment scripts
# Extract VK_HASH from the generated Solidity verifier (macOS + GNU compatible)
AGG_AGG_SOL_VK_HASH=$(sed -n 's/.*VK_HASH = \(0x[0-9a-fA-F]*\).*/\1/p' $REPO_ROOT/citrea/noir/agg_agg.sol)
if [ -n "$AGG_AGG_SOL_VK_HASH" ]; then
    echo "Propagating agg_agg VK hash ($AGG_AGG_SOL_VK_HASH) to deployment scripts..."

    # Update deploy.ts
    sed -i.bak "s|\"0x[0-9a-fA-F]\{64\}\";|\"$AGG_AGG_SOL_VK_HASH\";|" $REPO_ROOT/citrea/scripts/deploy.ts
    rm -f $REPO_ROOT/citrea/scripts/deploy.ts.bak

    # Update deploy-devnet.ts
    sed -i.bak "s|\"0x[0-9a-fA-F]\{64\}\";|\"$AGG_AGG_SOL_VK_HASH\";|" $REPO_ROOT/citrea/scripts/deploy-devnet.ts
    rm -f $REPO_ROOT/citrea/scripts/deploy-devnet.ts.bak

    # Update rollup.rs
    sed -i.bak "s|\"0x[0-9a-fA-F]\{64\}\"|\"$AGG_AGG_SOL_VK_HASH\"|" $REPO_ROOT/pkg/contracts/src/rollup.rs
    rm -f $REPO_ROOT/pkg/contracts/src/rollup.rs.bak

    echo "VK hash propagated successfully."
else
    echo "WARNING: Could not extract VK_HASH from agg_agg.sol"
fi

echo "Successfully generated fixtures for all programs"
