use dep::std;
use poseidon::poseidon2;

global AGG_UTXO_VERIFICATION_KEY_HASH: Field =
    13292478382929640414094914588468765641269481463031700247514057870573940873872;

struct UtxoAggProof {
    proof: [Field; 508],
    old_root: Field,
    new_root: Field,
    commit_hash: Field,
}

fn main(
    verification_key: [Field; 115],
    verification_key_hash: Field,
    utxo_agg_proofs: [UtxoAggProof; 2],
    old_root: pub Field,
    new_root: pub Field,
    commit_hash: pub Field,
    // 6 proofs, 6 per proof
    messages: pub [Field; 30],
) {
    // Hardcode verification_key_hash for agg_utxo proof
    assert(verification_key_hash == AGG_UTXO_VERIFICATION_KEY_HASH, "only agg_utxo proof allowed");

    // Pre-loop padding invariants. A padded child contributes nothing to
    // the outer message segment, commit_hash, or root, so every field
    // derived from that slot must be zero. Without this, the padded
    // segment of `messages` is exposed to the verifier as if it had been
    // proven, even though the recursive verifier re-uses slot 0's proof.
    for i in 0..2 {
        let is_padding = utxo_agg_proofs[i].old_root == 0;
        if i == 0 {
            assert(!is_padding, "first proof cannot be a padding proof");
        }
        if is_padding {
            for k in 0..15 {
                assert(messages[15 * i + k] == 0, "padded child messages must be zero");
            }
            assert(utxo_agg_proofs[i].commit_hash == 0, "padded child commit_hash must be zero");
            assert(utxo_agg_proofs[i].new_root == 0, "padded child new_root must be zero");
        }
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

        std::verify_proof_with_type(
            verification_key,
            if proof.old_root != 0 {
                proof.proof
            } else {
                utxo_agg_proofs[0].proof
            },
            if proof.old_root != 0 {
                public_inputs
            } else {
                first_public_inputs
            },
            verification_key_hash,
            7,
        );

        if proof.old_root != 0 {
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

#[test(should_fail_with = "first proof cannot be a padding proof")]
fn test_first_proof_cannot_be_padding() {
    // Slot 0 is the substitute proof for padded slots, so it must itself
    // be a real (non-padding) proof.
    let verification_key = [0; 115];
    let proof = UtxoAggProof { proof: [0; 508], old_root: 0, new_root: 0, commit_hash: 0 };
    let utxo_agg_proofs = [proof, proof];

    main(
        verification_key,
        AGG_UTXO_VERIFICATION_KEY_HASH,
        utxo_agg_proofs,
        0,
        0,
        0,
        [0; 30],
    );
}

#[test(should_fail_with = "padded child messages must be zero")]
fn test_padded_child_messages_must_be_zero() {
    // Padded slot 1 carries a forged burn (kind = 3) in
    // its message segment. The recursive verifier reuses slot 0's proof
    // so the segment would otherwise be unconstrained -- this assertion
    // must catch it.
    let verification_key = [0; 115];
    let real = UtxoAggProof { proof: [0; 508], old_root: 1, new_root: 2, commit_hash: 0 };
    let padded =
        UtxoAggProof { proof: [0; 508], old_root: 0, new_root: 0, commit_hash: 0 };
    let utxo_agg_proofs = [real, padded];

    let mut messages = [0; 30];
    messages[15] = 3; // forged burn kind in padded segment

    main(
        verification_key,
        AGG_UTXO_VERIFICATION_KEY_HASH,
        utxo_agg_proofs,
        1,
        2,
        0,
        messages,
    );
}

#[test(should_fail_with = "padded child commit_hash must be zero")]
fn test_padded_child_commit_hash_must_be_zero() {
    let verification_key = [0; 115];
    let real = UtxoAggProof { proof: [0; 508], old_root: 1, new_root: 2, commit_hash: 0 };
    // old_root == 0 marks padding, but commit_hash is nonzero.
    let padded =
        UtxoAggProof { proof: [0; 508], old_root: 0, new_root: 0, commit_hash: 999 };
    let utxo_agg_proofs = [real, padded];

    main(
        verification_key,
        AGG_UTXO_VERIFICATION_KEY_HASH,
        utxo_agg_proofs,
        1,
        2,
        0,
        [0; 30],
    );
}

#[test(should_fail_with = "padded child new_root must be zero")]
fn test_padded_child_new_root_must_be_zero() {
    let verification_key = [0; 115];
    let real = UtxoAggProof { proof: [0; 508], old_root: 1, new_root: 2, commit_hash: 0 };
    // old_root == 0 marks padding, but new_root is nonzero.
    let padded =
        UtxoAggProof { proof: [0; 508], old_root: 0, new_root: 7, commit_hash: 0 };
    let utxo_agg_proofs = [real, padded];

    main(
        verification_key,
        AGG_UTXO_VERIFICATION_KEY_HASH,
        utxo_agg_proofs,
        1,
        2,
        0,
        [0; 30],
    );
}
