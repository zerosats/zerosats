//! Rust wrapping for the Noir `escrow` leaf circuit.
//!
//! `escrow` is a sibling of `utxo`: it produces a proof with the same
//! public-input shape (4 commitments + 5 messages) and the same
//! `UTXO_PROOF_SIZE` bytes, but its input notes are `EscrowInputNote`s
//! that carry a `spend_type` selector and the corresponding witnesses
//! (`preimage`, `time_proof`). The circuit currently only accepts the
//! `Send` transaction kind; other kinds are reserved for future use and
//! reject inside Noir's `else` branch.
use std::fmt::Debug;

use crate::traits::ToBytes;
use crate::{
    EscrowInputNote, Note, UTXO_PROOF_SIZE, UTXO_PUBLIC_INPUTS_COUNT, UtxoKind, UtxoKindMessages,
    UtxoPublicInput, impl_serde_for_element_array,
};
use borsh::{BorshDeserialize, BorshSerialize};
use element::{Base, Element};
use hash::hash_merge;
use primitives::serde::{deserialize_base64, serialize_base64};
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts-rs")]
use ts_rs::TS;

/// Number of public input fields for an escrow proof. Same shape as the
/// utxo proof so an `agg_escrow` aggregator can slot into `agg_agg`
/// without re-shaping anything.
pub const ESCROW_PUBLIC_INPUTS_COUNT: usize = UTXO_PUBLIC_INPUTS_COUNT;
/// Number of 32-byte field elements in the escrow proof body. Same as
/// the utxo proof.
pub const ESCROW_PROOF_SIZE: usize = UTXO_PROOF_SIZE;

/// Data needed to produce an escrow proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Escrow {
    /// The kind of transaction. The escrow circuit only accepts
    /// [`UtxoKind::Send`] today; this field is kept for parity with
    /// [`crate::Utxo`] so the same downstream tooling (aggregator
    /// messages, public-input helpers) can be reused.
    pub kind: UtxoKind,
    /// The input notes (being spent). Each note carries its own
    /// `spend_type` + witness combination.
    pub input_notes: [EscrowInputNote; 2],
    /// The output notes (being created).
    pub output_notes: [Note; 2],
    /// Burn address. Always `None` today; reserved for symmetry with
    /// [`crate::Utxo`].
    pub burn_address: Option<Element>,
}

impl Escrow {
    /// Build a new escrow transaction. `kind` must currently be
    /// [`UtxoKind::Send`]; the Noir circuit asserts on anything else.
    #[must_use]
    pub fn new(
        kind: UtxoKind,
        input_notes: [EscrowInputNote; 2],
        output_notes: [Note; 2],
        burn_address: Option<Element>,
    ) -> Self {
        Self {
            kind,
            input_notes,
            output_notes,
            burn_address,
        }
    }

    /// Build a new send-kind escrow transaction.
    #[must_use]
    pub fn new_send(input_notes: [EscrowInputNote; 2], output_notes: [Note; 2]) -> Self {
        Self {
            kind: UtxoKind::Send,
            input_notes,
            output_notes,
            burn_address: None,
        }
    }

    /// Get the leaf elements (input + output commitments) for the
    /// transaction. Order matches `commitments` in the Noir circuit.
    #[must_use]
    pub fn leaf_elements(&self) -> [Element; 4] {
        [
            self.input_notes[0].note.commitment(),
            self.input_notes[1].note.commitment(),
            self.output_notes[0].commitment(),
            self.output_notes[1].commitment(),
        ]
    }

    /// Public messages emitted by the circuit. Same shape as
    /// `Utxo::messages`; today escrow only produces `Send`.
    #[must_use]
    pub fn messages(&self) -> [Element; 5] {
        match self.kind {
            UtxoKind::Send => [
                Element::new(1),
                Element::ZERO,
                Element::ZERO,
                Element::ZERO,
                Element::ZERO,
            ],
            _ => [Element::ZERO; 5],
        }
    }

    /// Sum of the two input note values.
    #[must_use]
    pub fn input_value(&self) -> Element {
        self.input_notes[0].note.value + self.input_notes[1].note.value
    }

    /// Sum of the two output note values.
    #[must_use]
    pub fn output_value(&self) -> Element {
        self.output_notes[0].value + self.output_notes[1].value
    }

    /// Public inputs as a [`UtxoPublicInput`], reusing the shared shape.
    #[must_use]
    pub fn public_inputs(&self) -> UtxoPublicInput {
        UtxoPublicInput {
            input_commitments: [
                self.input_notes[0].note.commitment(),
                self.input_notes[1].note.commitment(),
            ],
            output_commitments: [
                self.output_notes[0].commitment(),
                self.output_notes[1].commitment(),
            ],
            messages: self.messages(),
        }
    }
}

/// Raw proof body for an escrow transaction (does NOT include public
/// inputs).
#[derive(Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct EscrowProofBytes(
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub Vec<u8>,
);

impl Debug for EscrowProofBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x")?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Default for EscrowProofBytes {
    fn default() -> Self {
        Self(vec![0; 32 * ESCROW_PROOF_SIZE])
    }
}

impl EscrowProofBytes {
    /// Re-interpret the proof body as field elements.
    #[must_use]
    pub fn to_fields(&self) -> Vec<Element> {
        crate::bytes_to_elements(&self.0)
    }
}

/// Proof body as field elements (instead of bytes).
#[derive(Debug, Clone)]
pub struct EscrowProofFields(pub [Element; 93]);
impl_serde_for_element_array!(EscrowProofFields, 93);

impl From<EscrowProofFields> for [Base; 93] {
    fn from(value: EscrowProofFields) -> Self {
        value.0.map(|e| e.to_base())
    }
}

impl From<[Base; 93]> for EscrowProofFields {
    fn from(elements: [Base; 93]) -> Self {
        EscrowProofFields(elements.map(Element::from_base))
    }
}

/// The output proof for an escrow transaction.
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[cfg_attr(feature = "ts-rs", ts(export))]
#[derive(Default, Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct EscrowProof {
    /// The proof body for the escrow transaction.
    #[cfg_attr(feature = "ts-rs", ts(type = "string"))]
    pub proof: EscrowProofBytes,
    /// The public input for the escrow transaction. Shares the
    /// [`UtxoPublicInput`] shape so aggregator code can treat utxo and
    /// escrow leaves interchangeably.
    pub public_inputs: UtxoPublicInput,
}

impl PartialEq for EscrowProof {
    fn eq(&self, other: &Self) -> bool {
        self.public_inputs == other.public_inputs
    }
}

impl Eq for EscrowProof {}

impl ToBytes for EscrowProof {
    fn to_bytes(&self) -> Vec<u8> {
        let pi = self.public_inputs.to_bytes();
        let proof = self.proof.0.clone();
        [pi.as_slice(), proof.as_slice()].concat()
    }
}

impl EscrowProof {
    /// Hash the proof — uniquely identifies the public inputs.
    #[must_use]
    pub fn hash(&self) -> Element {
        self.public_inputs.hash()
    }

    /// Get the kind messages associated with the proof.
    #[must_use]
    pub fn kind_messages(&self) -> UtxoKindMessages {
        self.public_inputs.kind_messages()
    }

    /// Transaction kind.
    #[must_use]
    pub fn kind(&self) -> UtxoKind {
        self.public_inputs.kind()
    }

    /// Commitment hash over the 4 leaf commitments.
    #[must_use]
    pub fn commit_hash(&self) -> Element {
        hash_merge(self.public_inputs.commitments())
    }
}
