use barretenberg::Prove;
use noirc_abi::{InputMap, input_parser::InputValue};
use std::collections::BTreeMap;
use zk_primitives::{
    InputNote, Note, UTXO_PROOF_SIZE, UTXO_PUBLIC_INPUTS_COUNT, Utxo, UtxoProof, UtxoProofBytes,
    UtxoPublicInput, bytes_to_elements,
};

/// The Payy-flavored UTXO Noir program (`program.json`).
///
/// Carried inline so the payy crate is self-contained — no runtime fixtures,
/// no path-deps on the cli crate.
const PAYY_UTXO_PROGRAM: &str = include_str!("../circuit/program.json");

/// Raw verification key bytes (`bb write_vk` output) for the Payy UTXO circuit.
const PAYY_UTXO_KEY: &[u8] = include_bytes!("../circuit/key");

/// Build and prove a send-kind UTXO transaction with the **workspace** (Ciphera)
/// UTXO circuit. This is the path the Ciphera node expects.
pub fn prove_send(
    inputs: [InputNote; 2],
    outputs: [Note; 2],
) -> Result<UtxoProof, Box<dyn std::error::Error>> {
    Utxo::new_send(inputs, outputs).prove()
}

/// Build and prove a send-kind UTXO transaction with the **Payy** UTXO circuit.
///
/// The Payy circuit ABI is wider than the workspace circuit's: it takes
/// `input_notes`, `output_notes`, `pmessage4` (= messages[4] as a private
/// witness), `commitments` (public, 4 fields), and `messages` (public, 5
/// fields). Both `commitments` and `messages` are passed as InputMap entries
/// even though they are public — the Noir prover needs the witness solver to
/// see them to constrain the circuit.
///
/// Returns a [`UtxoProof`] whose `proof` field contains the raw backend output
/// and whose `public_inputs` are derived deterministically from the input/output
/// notes via [`Utxo::public_inputs`], so node-side verification can rebuild
/// the public-input encoding without trusting the prover.
pub fn prove_send_payy(
    inputs: [InputNote; 2],
    outputs: [Note; 2],
) -> Result<UtxoProof, Box<dyn std::error::Error>> {
    let utxo = Utxo::new_send(inputs, outputs);
    let messages = utxo.messages();
    let commitments = utxo.leaf_elements();

    let mut input_map: InputMap = BTreeMap::new();
    input_map.insert(
        "input_notes".to_owned(),
        InputValue::Vec(
            utxo.input_notes
                .iter()
                .map(input_note_to_input_value)
                .collect(),
        ),
    );
    input_map.insert(
        "output_notes".to_owned(),
        InputValue::Vec(utxo.output_notes.iter().map(note_to_input_value).collect()),
    );
    input_map.insert(
        "pmessage4".to_owned(),
        InputValue::Field(messages[4].to_base()),
    );
    input_map.insert(
        "commitments".to_owned(),
        InputValue::Vec(
            commitments
                .iter()
                .map(|c| InputValue::Field(c.to_base()))
                .collect(),
        ),
    );
    input_map.insert(
        "messages".to_owned(),
        InputValue::Vec(
            messages
                .iter()
                .map(|m| InputValue::Field(m.to_base()))
                .collect(),
        ),
    );

    // `recursive: true` matches the workspace's Ciphera utxo path
    // (`pkg/barretenberg/src/circuits/utxo.rs`). The bb backend then prefixes
    // the proof bytes with the 9 public-input fields it actually used during
    // proving. We strip those off and reparse them into UtxoPublicInput so
    // the wire shape exactly matches what the Payy node expects (raw proof
    // == 508 fields = 16256 bytes, public_inputs reflecting the bb-emitted
    // values verbatim rather than our locally recomputed ones).
    let proof_bytes =
        barretenberg::prove_default(PAYY_UTXO_PROGRAM, PAYY_UTXO_KEY, &input_map, true, false)?;

    let pi_byte_len = UTXO_PUBLIC_INPUTS_COUNT * 32;
    if proof_bytes.len() < pi_byte_len {
        return Err(format!(
            "Payy proof too short: got {} bytes, need at least {} for public-input prefix",
            proof_bytes.len(),
            pi_byte_len,
        )
        .into());
    }
    let pi_elements = bytes_to_elements(&proof_bytes[..pi_byte_len]);
    let raw_proof = proof_bytes[pi_byte_len..].to_vec();

    if raw_proof.len() != UTXO_PROOF_SIZE * 32 {
        return Err(format!(
            "Payy proof body has unexpected length: got {} bytes, expected {} ({} fields × 32)",
            raw_proof.len(),
            UTXO_PROOF_SIZE * 32,
            UTXO_PROOF_SIZE,
        )
        .into());
    }

    Ok(UtxoProof {
        proof: UtxoProofBytes(raw_proof),
        public_inputs: UtxoPublicInput {
            input_commitments: [pi_elements[0], pi_elements[1]],
            output_commitments: [pi_elements[2], pi_elements[3]],
            messages: [
                pi_elements[4],
                pi_elements[5],
                pi_elements[6],
                pi_elements[7],
                pi_elements[8],
            ],
        },
    })
}

/// Map a [`Note`] into the Payy circuit's `Note { kind, value, address, psi }`
/// struct input. The Noir field named `kind` is fed from `note.contract`
/// (not `note.kind`) to match the snapshot's `BNote`/`CommonNote` convention.
fn note_to_input_value(note: &Note) -> InputValue {
    let mut m = BTreeMap::new();
    m.insert("kind".to_owned(), InputValue::Field(note.contract.to_base()));
    m.insert("value".to_owned(), InputValue::Field(note.value.to_base()));
    m.insert(
        "address".to_owned(),
        InputValue::Field(note.address.to_base()),
    );
    m.insert("psi".to_owned(), InputValue::Field(note.psi.to_base()));
    InputValue::Struct(m)
}

fn input_note_to_input_value(input_note: &InputNote) -> InputValue {
    let mut m = BTreeMap::new();
    m.insert("note".to_owned(), note_to_input_value(&input_note.note));
    m.insert(
        "secret_key".to_owned(),
        InputValue::Field(input_note.secret_key.to_base()),
    );
    InputValue::Struct(m)
}
