use element::Base;
use noirc_abi::input_parser::InputValue;
use std::collections::BTreeMap;
use zk_primitives::{InputNote, Note, TimeLock, TimeProof};

#[derive(Debug, Clone)]
pub struct BInputNote {
    pub note: BNote,
    pub secret_key: Base,
    pub preimage: [u8; 32],
    pub time_proof: TimeProof,
}

impl From<&InputNote> for BInputNote {
    fn from(note: &InputNote) -> Self {
        BInputNote {
            note: BNote::from(&note.note),
            secret_key: note.secret_key.to_base(),
            preimage: note.preimage,
            time_proof: note.time_proof.clone(),
        }
    }
}

impl From<BInputNote> for InputValue {
    fn from(note: BInputNote) -> Self {
        InputValue::Struct(BTreeMap::from([
            ("note".to_owned(), note.note.into()),
            ("secret_key".to_owned(), InputValue::Field(note.secret_key)),
            ("preimage".to_owned(), bytes_to_input_value(&note.preimage)),
            ("time_proof".to_owned(), time_proof_to_input_value(&note.time_proof)),
        ]))
    }
}

#[derive(Debug, Clone)]
pub struct BNote {
    pub kind: Base,
    pub value: Base,
    pub address: Base,
    pub psi: Base,
}

impl From<&Note> for BNote {
    fn from(note: &Note) -> Self {
        BNote {
            kind: note.contract.to_base(),
            value: note.value.to_base(),
            address: note.address.to_base(),
            psi: note.psi.to_base(),
        }
    }
}

impl From<BNote> for InputValue {
    fn from(note: BNote) -> Self {
        let mut struct_ = BTreeMap::new();

        struct_.insert("kind".to_owned(), InputValue::Field(note.kind));
        struct_.insert("value".to_owned(), InputValue::Field(note.value));
        struct_.insert("address".to_owned(), InputValue::Field(note.address));
        struct_.insert("psi".to_owned(), InputValue::Field(note.psi));

        InputValue::Struct(struct_)
    }
}

fn bytes_to_input_value(bytes: &[u8]) -> InputValue {
    InputValue::Vec(
        bytes
            .iter()
            .map(|&b| InputValue::Field(Base::from(u128::from(b))))
            .collect(),
    )
}

fn time_lock_to_input_value(lock: &TimeLock) -> InputValue {
    InputValue::Struct(BTreeMap::from([
        (
            "zero_block".to_owned(),
            bytes_to_input_value(&lock.zero_block),
        ),
        (
            "n_blocks".to_owned(),
            InputValue::Field(lock.n_blocks.to_base()),
        ),
    ]))
}

fn time_proof_to_input_value(proof: &TimeProof) -> InputValue {
    InputValue::Struct(BTreeMap::from([
        ("lock".to_owned(), time_lock_to_input_value(&proof.lock)),
        (
            "headers".to_owned(),
            InputValue::Vec(
                proof
                    .headers
                    .iter()
                    .map(|h| bytes_to_input_value(h))
                    .collect(),
            ),
        ),
    ]))
}
