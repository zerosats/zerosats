// agg_escrow: 1-level aggregator that batches `escrow` leaf proofs. Structurally
// identical to `agg_utxo` -- same public-input shape (15 messages + 3 roots),
// same `[Field; 508]` proof bundle, same 3-proof fan-out -- so an `agg_escrow`
// proof can sit in the same `agg_agg` slot as an `agg_utxo` proof.
//
// The only behavioural difference is the hardcoded leaf VK hash: `agg_escrow`
// will *only* accept `escrow` leaf proofs. `agg_agg` then enforces that the
// proof in each of its two slots came from one of {agg_utxo, agg_escrow}.
use dep::std;
use common::is_one_of;
use poseidon::poseidon2;

global ESCROW_VERIFICATION_KEY_HASH: Field = 8115097813416055474203293494159333668492496971502701553388391990757819310029;

struct AggUtxoProofInput {
    proof: [Field; 508],
    utxo_kind: Field,
    input_merkle_paths: [[Field; 160]; 2],
    output_merkle_paths: [[Field; 160]; 2],
    input_commitments: [Field; 2],
    output_commitments: [Field; 2],
}

fn main(
    verification_key: [Field; 115],
    verification_key_hash: Field,
    proofs: [AggUtxoProofInput; 3],
    messages: pub [Field; 15],
    old_root: pub Field,
    new_root: pub Field,
    commit_hash: pub Field,
) {
    // Hardcode verification_key_hash for escrow proof. The propagation lives
    // in `noir/generate_fixtures.sh`, which writes ESCROW_VK_HASH after
    // compiling escrow and before compiling agg_escrow (mirroring how
    // agg_utxo gets UTXO_VK_HASH).
    assert(verification_key_hash == ESCROW_VERIFICATION_KEY_HASH, "only escrow proof allowed");

    let mut root = old_root;
    let mut utxo_hashes: [Field; 3] = [0, 0, 0];
    let mut messages_index = 0;

    let mut first_public_inputs: [Field; 9] = [0; 9];
    for i in 0..3 {
        let proof = proofs[i];
        let input_commitments = proof.input_commitments;
        let output_commitments = proof.output_commitments;

        let utxo_kind = proof.utxo_kind;
        let is_padding = utxo_kind == 0;
        let is_proof_kind_mint_or_burn = is_one_of(utxo_kind, [2, 3, 4]);
        let is_burn = is_one_of(utxo_kind, [3, 4]);

        let mut msg_note_kind = 0;
        let mut msg_value = 0;
        let mut msg_hash = 0;
        let mut msg_burn_addr = 0;

        if is_proof_kind_mint_or_burn {
            assert(messages[messages_index] == utxo_kind, "proof 'kind' must match message");
            msg_note_kind = messages[messages_index + 1];
            msg_value = messages[messages_index + 2];
            msg_hash = messages[messages_index + 3];
            messages_index += 4;
        }

        if is_burn {
            msg_burn_addr = messages[messages_index];
            messages_index += 1;
        }

        utxo_hashes[i] = poseidon2::Poseidon2::hash(
            [
                input_commitments[0],
                input_commitments[1],
                output_commitments[0],
                output_commitments[1],
            ],
            4,
        );

        let public_inputs = [
            input_commitments[0],
            input_commitments[1],
            output_commitments[0],
            output_commitments[1],
            utxo_kind,
            msg_note_kind,
            msg_value,
            msg_hash,
            msg_burn_addr,
        ];

        if (i == 0) {
            assert(!is_padding, "first proof cannot be a padding proof");
            first_public_inputs = public_inputs;
        }

        std::verify_proof_with_type(
            verification_key,
            if (is_padding) {
                proofs[0].proof
            } else {
                proof.proof
            },
            if (is_padding) {
                first_public_inputs
            } else {
                public_inputs
            },
            verification_key_hash,
            7,
        );

        if (!is_padding) {
            for j in 0..2 {
                let commitment = input_commitments[j];
                if commitment != 0 {
                    let bits: [u1; 254] = commitment.to_le_bits();
                    let merkle_path = proof.input_merkle_paths[j];
                    validate_inclusion(commitment, bits, merkle_path, root);
                    root = get_null_root(bits, merkle_path);
                }
            }
            for j in 0..2 {
                let commitment = output_commitments[j];
                if commitment != 0 {
                    let bits: [u1; 254] = commitment.to_le_bits();
                    let merkle_path = proof.output_merkle_paths[j];
                    validate_null(bits, merkle_path, root);
                    root = get_merkle_root(commitment, bits, merkle_path);
                }
            }
        }
    }

    for i in 0..messages.len() {
        if i >= messages_index {
            assert(messages[i] == 0, "Message is not zero");
        }
    }

    assert(root == new_root, "New root is not valid");
    assert(commit_hash == poseidon2::Poseidon2::hash(utxo_hashes, 3));
}

fn validate_inclusion(leaf: Field, bits: [u1; 254], merkle_path: [Field; 160], root: Field) {
    let merkle_root = get_merkle_root(leaf, bits, merkle_path);
    assert(merkle_root == root, "Merkle path root does not match");
}

fn validate_null(bits: [u1; 254], merkle_path: [Field; 160], root: Field) {
    let merkle_root = get_null_root(bits, merkle_path);
    assert(merkle_root == root, "Merkle path root does not match");
}

fn get_null_root(bits: [u1; 254], merkle_path: [Field; 160]) -> Field {
    get_merkle_root(0, bits, merkle_path)
}

fn get_merkle_root(leaf: Field, bits: [u1; 254], merkle_path: [Field; 160]) -> Field {
    let mut hash = leaf;
    for i in 0..160 {
        let dir = bits[i];
        let sibling = merkle_path[i];
        if dir == 0 {
            hash = poseidon2::Poseidon2::hash([hash, sibling], 2);
        } else {
            hash = poseidon2::Poseidon2::hash([sibling, hash], 2);
        }
    }
    hash
}
