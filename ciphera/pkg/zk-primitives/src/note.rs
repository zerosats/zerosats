use crate::{
    bridged_polygon_usdc_note_kind,
    get_address_for_private_key, hash_private_key_for_psi,
};
use element::Element;
use noirc_abi::input_parser::InputValue;
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(feature = "ts-rs")]
use ts_rs::TS;

/// A note is used in zk circuits to represent some utxo_kind of token (e.g. USDC) on
/// the Ciphera Network.
///
/// This is used to create notes in the zk-rollup
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The kind of utxo
    pub utxo_kind: Element,
    /// The kind of a note
    pub note_kind: Element,
    /// The address of the note
    pub address: Element,
    /// The psi adds additional entropy to the note, to ensure uniqueness
    pub psi: Element,
    /// The value of the note (dependent on the token)
    pub value: Element,
}

impl Note {
    /// Create a new note
    #[must_use]
    pub fn new(address: Element, value: Element) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind: bridged_polygon_usdc_note_kind(),
            address,
            psi: Element::secure_random(thread_rng()),
            value,
        }
    }

    /// Create a new note with custom PSI
    #[must_use]
    pub fn new_with_psi(address: Element, value: Element, psi: Element) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind: bridged_polygon_usdc_note_kind(),
            address,
            psi,
            value,
        }
    }

    /// Create a new note with a custom `note_kind` and PSI. Use for non-USDC
    /// kinds (5/6/7/8 spend paths, or anything else that doesn't sit on the
    /// default bridged-polygon-USDC kind).
    #[must_use]
    pub fn new_with_note_kind(
        address: Element,
        value: Element,
        psi: Element,
        note_kind: Element,
    ) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind,
            address,
            psi,
            value,
        }
    }

    /// New note from ephemeral private key (only use private key once)
    #[must_use]
    pub fn new_from_ephemeral_private_key(private_key: Element, value: Element) -> Self {
        let address = get_address_for_private_key(private_key);
        let psi = hash_private_key_for_psi(private_key);
        Self {
            utxo_kind: Element::new(2),
            note_kind: bridged_polygon_usdc_note_kind(),
            address,
            psi,
            value,
        }
    }

    /// Deterministic padding note, because circuits have a fixed note input size,
    /// and so we pad extra notes with zeros
    #[must_use]
    pub fn padding_note() -> Self {
        Note {
            utxo_kind: Element::new(2),
            note_kind: Element::ZERO,
            address: Element::ZERO,
            psi: Element::ZERO,
            value: Element::ZERO,
        }
    }

    /// Check if the note is a padding note
    #[must_use]
    pub fn is_padding_note(&self) -> bool {
        self.note_kind == Element::ZERO && self.value == Element::ZERO
    }

    /// Commitment of the note, this is stored in the merkle tree and proves the note exists.
    ///
    /// Matches the Noir `get_note_commitment` helper: padding (Noir `note.utxo_kind == 0`,
    /// which maps to Rust `self.note_kind == 0`) commits to zero, every other note
    /// commits to `Poseidon([2, note_kind, value, address, psi, 0, 0], 7)`.
    // TODO: should we leave some space in here?
    #[must_use]
    pub fn commitment(&self) -> Element {
        if self.note_kind == Element::ZERO {
            Element::ZERO
        } else {
            hash::hash_merge([
                self.utxo_kind,
                self.note_kind,
                self.value,
                self.address,
                self.psi,
                Element::ZERO,
                Element::ZERO,
            ])
        }
    }
}

impl Default for Note {
    fn default() -> Self {
        Self::padding_note()
    }
}

impl From<&Note> for InputValue {
    fn from(note: &Note) -> Self {
        let mut struct_ = BTreeMap::new();

        struct_.insert(
            "address".to_owned(),
            InputValue::Field(note.address.to_base()),
        );
        struct_.insert(
            "utxo_kind".to_owned(),
            InputValue::Field(note.note_kind.to_base()),
        );
        struct_.insert("psi".to_owned(), InputValue::Field(note.psi.to_base()));
        struct_.insert("value".to_owned(), InputValue::Field(note.value.to_base()));

        InputValue::Struct(struct_)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_check_gates_on_note_kind() {
        // note_kind=0 → commitment is zero regardless of value (matches Noir utxo_kind==0 branch).
        let zero_utxo_kind = Note {
            utxo_kind: Element::new(2),
            note_kind: Element::ZERO,
            address: Element::new(123),
            psi: Element::new(456),
            value: Element::new(10),
        };
        assert_eq!(zero_utxo_kind.commitment(), Element::ZERO);

        // note_kind!=0 with value=0 → still commits to a non-zero hash, matching
        // Noir which only zero-commits when note.utxo_kind == 0.
        let zero_value = Note {
            utxo_kind: Element::new(2),
            note_kind: Element::new(5),
            address: Element::ZERO,
            psi: Element::ZERO,
            value: Element::ZERO,
        };
        assert_ne!(zero_value.commitment(), Element::ZERO);
    }
}
