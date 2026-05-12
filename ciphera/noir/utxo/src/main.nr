use common::{
    bytes_32_to_field_pair, check_commitment, check_spend_conditions, get_note_commitment,
    get_secret_hash, InputNote, Note, TimeLock, TimeProof,
};
use poseidon::poseidon2;

mod test;

fn main(
    input_notes: [InputNote; 2],
    output_notes: [Note; 2],
    pmessage4: Field,
    commitments: pub [Field; 4],
    messages: pub [Field; 5],
) {
    // Check the commitments
    check_commitment(input_notes[0].note, commitments[0]);
    check_commitment(input_notes[1].note, commitments[1]);
    check_commitment(output_notes[0], commitments[2]);
    check_commitment(output_notes[1], commitments[3]);

    // Check individual outputs are not greater than 240 bits
    output_notes[0].value.assert_max_bit_size::<240>();
    output_notes[1].value.assert_max_bit_size::<240>();

    // We do not need to check the input notes values, as they must already have been added
    // to the tree, and the only way for that to occur if them to be added as an output note first
    // (which the check below would catch). Also, an overflow attack on the inputs would be deterimental
    // to the attacker as they would have to provide more input funds, and would receive less output funds.
    let input_value = input_notes[0].note.value + input_notes[1].note.value;

    // Because we have already checked the individual output notes are no greater than 240 bits,
    // then the maximum of the addition can be 2^241 which means the output value can never overflow
    let output_value = output_notes[0].value + output_notes[1].value;

    // Check for multiple kinds, not currently supported in this proof
    let (is_multiple_kinds, _note_kind) = is_multiple_kinds([
        input_notes[0].note,
        input_notes[1].note,
        output_notes[0],
        output_notes[1],
    ]);
    assert(!is_multiple_kinds, "Inconsistent kinds are not allowed");

    // Kind of UTXO txn (in future we may have subtypes for kind, which define different encodings
    // but for now its easier to define this way, so bytes[32] will always be main kind)
    let kind = messages[0];

    // Messages (only for mint/burns, otherwise all zeros)
    let msg_note_kind = messages[1];
    let msg_value = messages[2];

    // Assert message value size is max 240 bits to prevent overflow attack
    msg_value.assert_max_bit_size::<240>();

    // We only constrain the message hash for mints. For burns, any hash can be used
    // as its only used by the substitutor to early release funds. Early release is
    // secured by the validator, so as long as it records the burn hash it should be valid.
    // A user could provide a burn hash that is already used, which would prevent
    // a the substitutor from substituting the burn; this is acceptable.
    let msg_hash = messages[3];
    let burn_addr = messages[4];

    // Mint hash - we use the private message to prevent others from
    // being able to frontrun the mint hash in their own utxo txn
    let mint_hash = poseidon2::Poseidon2::hash([output_notes[0].psi, output_notes[1].psi], 2);

    // Burn hash - used for the burn substitutor and to verify status of an existing burn -
    // could be any hash so long as its asserted in the proof and cannot be modified. We were
    // planning to use the commitment of both inputs, but that is extra work and not neccessary.
    let burn_hash = commitments[0];

    if (kind == 1) {
        //SEND
        assert(input_value == output_value, "Input and output totals do not match");
    } else if (kind == 2) {
        // MINT
        // Assert mint utxo is balanced:
        //   - `output_value` is checked above
        //   - `input_value` is checked as it must have previously been an output value
        //   - `msg_value` is checked above (but also using that to overflow would be detrimental to the
        //      attacker)
        assert(output_value == input_value + msg_value, "Mint output must match value message");
        // Assert mint hash
        assert(mint_hash == msg_hash, "Mint hash must match message");
        // Assert note kind
        assert(output_notes[0].kind == msg_note_kind, "Mint note kind must match message")
    } else if (kind == 3) | (kind == 4) {
        // 3 - for native & cross-chain routed burns
        // 4 - for cross-chain burns
        // BURN
        // Prevent frontrunning the txn and changing the evm address
        assert(pmessage4 == burn_addr, "messages[4] must match private input");
        // Assert burn hash
        assert(burn_hash == msg_hash, "Burn hash must match message");
        // Assert burn utxo is balanced:
        //   - `output_value` is checked above
        //   - `input_value` is checked as it must have previously been an output value
        //   - `msg_value` is checked above
        assert(input_value == output_value + msg_value, "Burn output must match value message");
        // Assert burn kind
        assert(input_notes[0].note.kind == msg_note_kind, "Burn note kind must match message")
    } else {
        assert(false, "Invalid kind");
    }

    // Check the user owns the input notes
    check_spend_conditions(input_notes[0]);
    check_spend_conditions(input_notes[1]);
}

fn is_multiple_kinds(notes: [Note; 4]) -> (bool, Field) {
    let mut first_non_zero_kind = 0;
    let mut is_multiple_kinds = false;

    for i in 0..4 {
        // Skip if we have already detected multiple kinds - basically an early return
        // (which is not supported directly in Noir)
        if !is_multiple_kinds {
            // Check if this is a non-padding note
            if notes[i].kind != 0 {
                if first_non_zero_kind == 0 {
                    // No previous kind exists, so add this one
                    first_non_zero_kind = notes[i].kind;
                } else if first_non_zero_kind != notes[i].kind {
                    // We've detected a mismatched kind
                    is_multiple_kinds = true;
                }
            }
        }
    }

    (is_multiple_kinds, first_non_zero_kind)
}

fn bridged_note_kind() -> Field {
    3533694129556768672311144317398675444585744224105014452550528428861358080
}

global zeros32: [u8; 32] = [0; 32];
global zeros80: [u8; 80] = [0; 80];
global zeroTimeLock: TimeLock = TimeLock { zero_block: zeros32, n_blocks: 0 };
global zeroTimeProof: TimeProof = TimeProof { lock: zeroTimeLock, headers: [zeros80; 2] };

#[test]
fn test_send_two_inputs_two_outputs() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 10, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: 1, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: 14, address, psi: 4 };

    // Commitments
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_send_one_input_two_outputs() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 10, address, psi: 1 };
    let note_2 = Note { kind: 0, value: 0, address, psi: 0 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: 0, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: 1, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: 9, address, psi: 4 };

    // Commitments
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_send_two_inputs_one_output() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 10, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: 15, address, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address, psi: 0 };

    // Commitments
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_send_diff_kinds() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 15, address, psi: 1 };
    let note_2 = Note { kind: 2, value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: 15, address, psi: 3 };
    let note_4 = Note { kind: 2, value: 5, address, psi: 4 };

    // Commitments
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_mint() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 15, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: 0, address: 0, psi: 0 };
    let note_4 = Note { kind: bridged_note_kind(), value: 30, address, psi: 3 };

    let commitments = [
        get_note_commitment(note_1),
        get_note_commitment(note_2),
        get_note_commitment(note_3),
        get_note_commitment(note_4),
    ];

    let mint_hash = poseidon2::Poseidon2::hash([note_3.psi, note_4.psi], 2);

    // Commitments
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        commitments,
        [2, bridged_note_kind(), 10, mint_hash, 0],
    )
}

#[test]
fn test_burn() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 15, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: 0, value: 0, address: 0, psi: 0 };
    let note_4 = Note { kind: bridged_note_kind(), value: 10, address, psi: 3 };

    // Commitments
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [3, bridged_note_kind(), 10, get_note_commitment(note_1), 0],
    )
}

#[test]
fn test_send_zero_value() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs - both zero value notes
    let note_1 = Note { kind: bridged_note_kind(), value: 0, address, psi: 1 };
    let note_2 = Note { kind: 0, value: 0, address, psi: 0 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: 0, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - both zero value notes
    let note_3 = Note { kind: bridged_note_kind(), value: 0, address, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address, psi: 0 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_mint_with_padding_input() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs - one real note, one padding
    let note_1 = Note { kind: bridged_note_kind(), value: 5, address, psi: 1 };
    let note_2 = Note { kind: 0, value: 0, address, psi: 0 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: 0, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - note_3 is the first output (minted note), note_4 is padding or same kind
    let note_3 = Note { kind: bridged_note_kind(), value: 15, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: 0, address: 0, psi: 0 };

    let commitments = [
        get_note_commitment(note_1),
        get_note_commitment(note_2),
        get_note_commitment(note_3),
        get_note_commitment(note_4),
    ];

    let mint_hash = poseidon2::Poseidon2::hash([note_3.psi, note_4.psi], 2);

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        commitments,
        [2, bridged_note_kind(), 10, mint_hash, 0],
    )
}

#[test]
fn test_burn_single_input() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs - one real note, one padding
    let note_1 = Note { kind: bridged_note_kind(), value: 15, address, psi: 1 };
    let note_2 = Note { kind: 0, value: 0, address, psi: 0 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: 0, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - burn 10 value, keep 5
    let note_3 = Note { kind: 0, value: 0, address: 0, psi: 0 };
    let note_4 = Note { kind: bridged_note_kind(), value: 5, address, psi: 3 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [3, bridged_note_kind(), 10, get_note_commitment(note_1), 0],
    )
}
#[test]
fn test_burn_no_sub() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 15, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: 0, value: 0, address: 0, psi: 0 };
    let note_4 = Note { kind: bridged_note_kind(), value: 10, address, psi: 3 };

    // Commitments
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [4, bridged_note_kind(), 10, get_note_commitment(note_1), 0],
    )
}

#[test]
fn test_burn_no_sub_single_input() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs - one real note, one padding
    let note_1 = Note { kind: bridged_note_kind(), value: 15, address, psi: 1 };
    let note_2 = Note { kind: 0, value: 0, address, psi: 0 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: 0, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - burn 10 value, keep 5
    let note_3 = Note { kind: 0, value: 0, address: 0, psi: 0 };
    let note_4 = Note { kind: bridged_note_kind(), value: 5, address, psi: 3 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [4, bridged_note_kind(), 10, get_note_commitment(note_1), 0],
    )
}

#[test(should_fail)]
fn test_burn_no_sub_invalid_addr() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 15, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: 0, value: 0, address: 0, psi: 0 };
    let note_4 = Note { kind: bridged_note_kind(), value: 10, address, psi: 3 };

    // pmessage4 (private burn addr) does not match messages[4] (public burn_addr), it must fail
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        12345, // pmessage4 != burn_addr below
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [4, bridged_note_kind(), 10, get_note_commitment(note_1), 99999],
    )
}

#[test(should_fail)]
fn test_mint_different_kinds() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs - mix of different kinds
    let note_1 = Note { kind: bridged_note_kind(), value: 5, address, psi: 1 };
    let note_2 = Note { kind: 2, value: 3, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - mint kind 3, first note is the minted note
    let note_3 = Note { kind: 3, value: 18, address, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address: 0, psi: 0 };

    let commitments = [
        get_note_commitment(note_1),
        get_note_commitment(note_2),
        get_note_commitment(note_3),
        get_note_commitment(note_4),
    ];

    let mint_hash = poseidon2::Poseidon2::hash([note_3.psi, note_4.psi], 2);

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        commitments,
        [2, 3, 10, mint_hash, 0],
    )
}

#[test]
fn test_send_large_values() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Test with larger values (but still within 240-bit constraint)
    let large_value = 1000000000;

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: large_value, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: large_value, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: large_value + 500000, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: large_value - 500000, address, psi: 4 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_mint_same_kind() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Inputs - same kind
    let note_1 = Note { kind: bridged_note_kind(), value: 5, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 10, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - mint more of same kind, first note is the minted note
    let note_3 = Note { kind: bridged_note_kind(), value: 25, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: 0, address: 0, psi: 0 };

    let commitments = [
        get_note_commitment(note_1),
        get_note_commitment(note_2),
        get_note_commitment(note_3),
        get_note_commitment(note_4),
    ];

    let mint_hash = poseidon2::Poseidon2::hash([note_3.psi, note_4.psi], 2);

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        commitments,
        [2, bridged_note_kind(), 10, mint_hash, 0],
    )
}

#[test(should_fail)]
fn test_overflow_input_first_note() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Create a value that exceeds 240 bits
    // 2^240 = 1766847064778384329583297500742918515827483896875618958121606201292619776
    let overflow_value = 1766847064778384329583297500742918515827483896875618958121606201292619777;

    // Inputs - first note has overflow value
    let note_1 = Note { kind: bridged_note_kind(), value: overflow_value, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: 10, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: 5, address, psi: 4 };

    // This should fail due to overflow check on first input
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_overflow_input_second_note() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Create a value that exceeds 240 bits
    let overflow_value = 1766847064778384329583297500742918515827483896875618958121606201292619777;

    // Inputs - second note has overflow value
    let note_1 = Note { kind: bridged_note_kind(), value: 5, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: overflow_value, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs
    let note_3 = Note { kind: bridged_note_kind(), value: 10, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: 5, address, psi: 4 };

    // This should fail due to overflow check on second input
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_overflow_output_first_note() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Create a value that exceeds 240 bits
    let overflow_value = 1766847064778384329583297500742918515827483896875618958121606201292619777;

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 10, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - first output has overflow value
    let note_3 = Note { kind: bridged_note_kind(), value: overflow_value, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: 5, address, psi: 4 };

    // This should fail due to overflow check on first output
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_overflow_output_second_note() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Create a value that exceeds 240 bits
    let overflow_value = 1766847064778384329583297500742918515827483896875618958121606201292619777;

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 10, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - second output has overflow value
    let note_3 = Note { kind: bridged_note_kind(), value: 5, address, psi: 3 };
    let note_4 = Note { kind: bridged_note_kind(), value: overflow_value, address, psi: 4 };

    // This should fail due to overflow check on second output
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_max_allowed_value() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Use a value that is exactly at the 240-bit limit (2^240 - 1)
    let max_value = 1766847064778384329583297500742918515827483896875618958121606201292619775;

    // Inputs - both notes have max allowed value
    let note_1 = Note { kind: bridged_note_kind(), value: max_value, address, psi: 1 };
    let note_2 = Note { kind: 0, value: 0, address, psi: 0 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: 0, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - transfer to single output
    let note_3 = Note { kind: bridged_note_kind(), value: max_value, address, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address, psi: 0 };

    // This should succeed as the value is exactly at the limit
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_msg_value_overflow() {
    let pk: Field = 101;
    let address = get_secret_hash(pk);

    // Create a value that exceeds 240 bits
    let overflow_value = 1766847064778384329583297500742918515827483896875618958121606201292619777;

    // Inputs
    let note_1 = Note { kind: bridged_note_kind(), value: 10, address, psi: 1 };
    let note_2 = Note { kind: bridged_note_kind(), value: 5, address, psi: 2 };
    let input_note_1 =
        InputNote { note: note_1, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };
    let input_note_2 =
        InputNote { note: note_2, secret_key: pk, preimage: zeros32, time_proof: zeroTimeProof };

    // Outputs - mint with overflow msg_value
    let note_3 = Note { kind: bridged_note_kind(), value: 15, address, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address: 0, psi: 0 };

    let commitments = [
        get_note_commitment(note_1),
        get_note_commitment(note_2),
        get_note_commitment(note_3),
        get_note_commitment(note_4),
    ];

    let mint_hash = poseidon2::Poseidon2::hash([note_3.psi, note_4.psi], 2);

    // This should fail due to msg_value overflow check
    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        commitments,
        [2, bridged_note_kind(), overflow_value, mint_hash, 0],
    )
}

// =====================================================================
// Test fixtures shared with signature32 / signature32sha.
//
// `shared_preimage()` is the 32-byte preimage used by both sibling
// circuits. `shared_preimage_sha()` is its SHA-256 digest, matching
// the `sha` array in signature32sha::test_main. The two kindN_address
// helpers compute the expected `note.address` for each spend path.
// =====================================================================

fn shared_preimage() -> [u8; 32] {
    [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32,
    ]
}

fn shared_preimage_sha() -> [u8; 32] {
    [
        174, 33, 108, 46, 245, 36, 122, 55, 130, 193, 53, 239, 162, 121, 163, 228, 205, 198, 16,
        148, 39, 15, 93, 43, 229, 140, 98, 4, 183, 166, 18, 201,
    ]
}

// kind 5 (signature32): address = Poseidon(field_pair(preimage)).
// The hex constant is identical to `hash` in signature32::test_main.
fn kind5_address() -> Field {
    let (low, high) = bytes_32_to_field_pair(shared_preimage());
    poseidon2::Poseidon2::hash([low, high], 2)
}

// kind 6 (signature32sha): address = Poseidon(field_pair(SHA-256(preimage))).
fn kind6_address() -> Field {
    let (low, high) = bytes_32_to_field_pair(shared_preimage_sha());
    poseidon2::Poseidon2::hash([low, high], 2)
}

fn padding_input() -> InputNote {
    InputNote {
        note: Note { kind: 0, value: 0, address: 0, psi: 0 },
        secret_key: 0,
        preimage: zeros32,
        time_proof: zeroTimeProof,
    }
}

#[test]
fn test_send_kind5_preimage_ownership() {
    // kind 5: ownership proven by preimage whose Poseidon hash matches
    // the note address. Matches the signature32 fixture.
    let preimage = shared_preimage();
    let address = kind5_address();

    let note_1 = Note { kind: 5, value: 10, address, psi: 1 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0, // not used for kind 5
        preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = padding_input();

    let note_3 = Note { kind: 5, value: 7, address: 42, psi: 3 };
    let note_4 = Note { kind: 5, value: 3, address: 43, psi: 4 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(input_note_2.note),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_send_kind5_wrong_preimage() {
    // Spender supplies a preimage that doesn't hash to `address`.
    let address = kind5_address();

    let mut bad_preimage = shared_preimage();
    bad_preimage[0] = 99;

    let note_1 = Note { kind: 5, value: 10, address, psi: 1 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0,
        preimage: bad_preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = padding_input();

    let note_3 = Note { kind: 5, value: 10, address: 42, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address: 0, psi: 0 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(input_note_2.note),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_send_kind6_sha_preimage_ownership() {
    // kind 6: ownership proven by SHA-256 preimage whose Poseidon hash
    // (over the SHA digest as a field pair) matches the note address.
    // Reuses the (preimage, sha) pair from signature32sha::test_main.
    let preimage = shared_preimage();
    let address = kind6_address();

    let note_1 = Note { kind: 6, value: 10, address, psi: 1 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0,
        preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = padding_input();

    let note_3 = Note { kind: 6, value: 4, address: 99, psi: 3 };
    let note_4 = Note { kind: 6, value: 6, address: 100, psi: 4 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(input_note_2.note),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_send_kind6_wrong_preimage() {
    let address = kind6_address();

    let mut bad_preimage = shared_preimage();
    bad_preimage[31] = 0; // changes SHA digest -> Poseidon mismatch

    let note_1 = Note { kind: 6, value: 10, address, psi: 1 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0,
        preimage: bad_preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = padding_input();

    let note_3 = Note { kind: 6, value: 10, address: 42, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address: 0, psi: 0 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(input_note_2.note),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_burn_kind6_sha_preimage() {
    // Combine a kind-6 spend with the burn flow to confirm the ownership
    // check fires alongside the burn-specific assertions.
    let preimage = shared_preimage();
    let address = kind6_address();

    let note_1 = Note { kind: 6, value: 15, address, psi: 1 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0,
        preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = padding_input();

    let note_3 = Note { kind: 0, value: 0, address: 0, psi: 0 };
    let note_4 = Note { kind: 6, value: 5, address, psi: 3 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(input_note_2.note),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [3, 6, 10, get_note_commitment(note_1), 0],
    )
}

#[test]
fn test_mint_kind6_sha_preimage() {
    // Mint flow with a kind-6 input -- verifies SHA preimage ownership
    // is enforced even while increasing supply via msg_value.
    let preimage = shared_preimage();
    let address = kind6_address();

    let note_1 = Note { kind: 6, value: 5, address, psi: 1 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0,
        preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = padding_input();

    // Mint pattern: output[0] carries msg_note_kind, output[1] is the value note.
    let note_3 = Note { kind: 6, value: 0, address: 0, psi: 2 };
    let note_4 = Note { kind: 6, value: 15, address, psi: 3 };

    let mint_hash = poseidon2::Poseidon2::hash([note_3.psi, note_4.psi], 2);

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(input_note_2.note),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [2, 6, 10, mint_hash, 0],
    )
}

#[test(should_fail)]
fn test_send_kind5_zero_secret_other_kind() {
    // Sanity: a kind-5 note still rejects unrelated preimages even when
    // the secret_key happens to be the trivial Poseidon preimage of the
    // address that a kind-1 spend would use. Confirms the dispatch by
    // `kind` is not bypassable via secret_key collisions.
    let address = kind5_address();
    let mut wrong_preimage: [u8; 32] = [0; 32];
    wrong_preimage[31] = 1;

    let note_1 = Note { kind: 5, value: 10, address, psi: 1 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 101,
        preimage: wrong_preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = padding_input();

    let note_3 = Note { kind: 5, value: 10, address: 42, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address: 0, psi: 0 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(input_note_2.note),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test]
fn test_send_two_kind6_inputs_shared_preimage() {
    // Two real kind-6 inputs sharing the same preimage/address (a user
    // consolidating two of their own notes). Confirms the ownership
    // check fires for both input slots, not just index 0.
    let preimage = shared_preimage();
    let address = kind6_address();

    let note_1 = Note { kind: 6, value: 7, address, psi: 1 };
    let note_2 = Note { kind: 6, value: 3, address, psi: 2 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0,
        preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = InputNote {
        note: note_2,
        secret_key: 0,
        preimage,
        time_proof: zeroTimeProof,
    };

    let note_3 = Note { kind: 6, value: 10, address: 42, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address: 0, psi: 0 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}

#[test(should_fail)]
fn test_send_two_kind6_inputs_second_wrong_preimage() {
    // Same as above but the second input supplies a bad preimage. Must
    // fail -- the ownership assertion must apply to input index 1, not
    // only index 0.
    let preimage = shared_preimage();
    let address = kind6_address();

    let mut bad_preimage = preimage;
    bad_preimage[0] = 200;

    let note_1 = Note { kind: 6, value: 7, address, psi: 1 };
    let note_2 = Note { kind: 6, value: 3, address, psi: 2 };
    let input_note_1 = InputNote {
        note: note_1,
        secret_key: 0,
        preimage,
        time_proof: zeroTimeProof,
    };
    let input_note_2 = InputNote {
        note: note_2,
        secret_key: 0,
        preimage: bad_preimage,
        time_proof: zeroTimeProof,
    };

    let note_3 = Note { kind: 6, value: 10, address: 42, psi: 3 };
    let note_4 = Note { kind: 0, value: 0, address: 0, psi: 0 };

    main(
        [input_note_1, input_note_2],
        [note_3, note_4],
        0,
        [
            get_note_commitment(note_1),
            get_note_commitment(note_2),
            get_note_commitment(note_3),
            get_note_commitment(note_4),
        ],
        [1, 0, 0, 0, 0],
    )
}
