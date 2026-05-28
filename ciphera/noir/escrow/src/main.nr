// escrow: scaffolding for an alternative leaf-level UTXO circuit that can be
// aggregated alongside the original `utxo` via `agg_agg`. Public-input shape
// and proof size are intentionally identical to `utxo`, so an `agg_escrow`
// proof is shape-compatible with an `agg_utxo` proof inside `agg_agg`.
//
// What makes this circuit distinct (and thus give a different verification
// key) is the `escrow_sentinel` constraint below. Replace that constraint
// with the real alternative-kind logic you want; the sentinel exists only
// so the first build produces a VK that differs from `utxo`'s.
use common::{check_commitment, check_input_note_ownership, InputNote, Note};

fn main(
    input_notes: [InputNote; 2],
    output_notes: [Note; 2],
    _pmessage4: Field,
    commitments: pub [Field; 4],
    messages: pub [Field; 5],
) {
    check_commitment(input_notes[0].note, commitments[0]);
    check_commitment(input_notes[1].note, commitments[1]);
    check_commitment(output_notes[0], commitments[2]);
    check_commitment(output_notes[1], commitments[3]);

    output_notes[0].value.assert_max_bit_size::<240>();
    output_notes[1].value.assert_max_bit_size::<240>();

    let input_value = input_notes[0].note.value + input_notes[1].note.value;
    let output_value = output_notes[0].value + output_notes[1].value;

    let (is_multiple_kinds, _note_kind) = is_multiple_kinds([
        input_notes[0].note,
        input_notes[1].note,
        output_notes[0],
        output_notes[1],
    ]);
    assert(!is_multiple_kinds, "Inconsistent kinds are not allowed");

    let kind = messages[0];
    let msg_value = messages[2];

    msg_value.assert_max_bit_size::<240>();

    let escrow_sentinel: Field = 0xdeadbeefcafebabef00d;
    assert(kind != escrow_sentinel, "escrow sentinel");

    if (kind == 1) {
        assert(input_value == output_value, "Input and output totals do not match");
    } else {
        assert(false, "Invalid kind");
    }

    check_input_note_ownership(input_notes[0]);
    check_input_note_ownership(input_notes[1]);
}

fn is_multiple_kinds(notes: [Note; 4]) -> (bool, Field) {
    let mut first_non_zero_kind = 0;
    let mut is_multiple_kinds = false;

    for i in 0..4 {
        if !is_multiple_kinds {
            if notes[i].kind != 0 {
                if first_non_zero_kind == 0 {
                    first_non_zero_kind = notes[i].kind;
                } else if first_non_zero_kind != notes[i].kind {
                    is_multiple_kinds = true;
                }
            }
        }
    }

    (is_multiple_kinds, first_non_zero_kind)
}
