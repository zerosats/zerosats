use dep::std;
use poseidon::poseidon2;

// agg_agg accepts a heterogeneous batch: each of its two child slots can be
// either an `agg_utxo` proof (original `utxo` leaves) or an `agg_escrow` proof
// (alternative `escrow` leaves). The two child circuits have identical
// public-input shape, proof size, and aggregation structure -- the only
// difference is which leaf VK each one accepts -- so they are
// interchangeable inside `agg_agg` once we let the prover declare per-slot
// which child VK is being used.
//
// The on-chain public inputs (`old_root`, `new_root`, `commit_hash`,
// `messages`) are unchanged, so the Solidity HonkVerifier ABI is untouched.
global AGG_UTXO_VERIFICATION_KEY_HASH: Field = 3798507185556691269237895706526353312852291328634099462512602675102578351493;
global agg_escrow_VERIFICATION_KEY_HASH: Field = 9504338426323889057755558003739951053112252896403686341602500331479886491501;

struct UtxoAggProof {
    proof: [Field; 508],
    old_root: Field,
    new_root: Field,
    commit_hash: Field,
}

fn main(
    verification_keys: [[Field; 115]; 2],
    verification_key_hashes: [Field; 2],
    utxo_agg_proofs: [UtxoAggProof; 2],
    old_root: pub Field,
    new_root: pub Field,
    commit_hash: pub Field,
    // 6 proofs, 6 per proof
    messages: pub [Field; 30],
) {
    // Each slot's declared VK hash must be one of the two known
    // 1-level-aggregator VKs. Asserting membership rather than equality
    // is what makes a heterogeneous batch legal; a homogeneous batch
    // (the existing prover path) passes the same hash in both slots and
    // also satisfies this check.
    for i in 0..2 {
        let h = verification_key_hashes[i];
        let is_a = h == AGG_UTXO_VERIFICATION_KEY_HASH;
        let is_b = h == agg_escrow_VERIFICATION_KEY_HASH;
        assert(is_a | is_b, "vk_hash must be agg_utxo or agg_escrow");
    }

    let mut root = old_root;
    let mut first_public_inputs: [Field; 18] = [0; 18];

    for i in 0..2 {
        let proof = utxo_agg_proofs[i];
        let public_inputs = [
            messages[15 * i],
            messages[15 * i + 1],
            messages[15 * i + 2],
            messages[15 * i + 3],
            messages[15 * i + 4],
            messages[15 * i + 5],
            messages[15 * i + 6],
            messages[15 * i + 7],
            messages[15 * i + 8],
            messages[15 * i + 9],
            messages[15 * i + 10],
            messages[15 * i + 11],
            messages[15 * i + 12],
            messages[15 * i + 13],
            messages[15 * i + 14],
            proof.old_root,
            proof.new_root,
            proof.commit_hash,
        ];

        if i == 0 {
            first_public_inputs = public_inputs;
        }

        // Padding slot fallback (`proof.old_root == 0`) must reuse slot 0's
        // verification key alongside slot 0's proof bytes and public
        // inputs, otherwise verify_proof_with_type would mix a slot-1 VK
        // with slot-0 proof bytes and fail.
        let is_padding = proof.old_root == 0;
        std::verify_proof_with_type(
            if is_padding {
                verification_keys[0]
            } else {
                verification_keys[i]
            },
            if is_padding {
                utxo_agg_proofs[0].proof
            } else {
                proof.proof
            },
            if is_padding {
                first_public_inputs
            } else {
                public_inputs
            },
            if is_padding {
                verification_key_hashes[0]
            } else {
                verification_key_hashes[i]
            },
            7,
        );

        if !is_padding {
            assert(proof.old_root == root, "proof old_root must match previous root");
            root = proof.new_root;
        }
    }

    assert(
        commit_hash
            == poseidon2::Poseidon2::hash(
                [utxo_agg_proofs[0].commit_hash, utxo_agg_proofs[1].commit_hash],
                2,
            ),
    );
    assert(new_root == root, "Roots must match");
}
