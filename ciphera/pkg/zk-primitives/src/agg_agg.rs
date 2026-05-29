use crate::{AggUtxoProof, ToBytes};
use borsh::{BorshDeserialize, BorshSerialize};
use element::Element;
use hash::hash_merge;
use primitives::serde::{deserialize_base64, serialize_base64};
use serde::{Deserialize, Serialize};

/// Identifies which 1-level aggregator produced the proof in each
/// `AggAgg` slot. The Noir `agg_agg` circuit accepts either
/// `agg_utxo` or `agg_escrow` leaf proofs in either slot; the Rust
/// prover uses this enum to pick the matching verification key per
/// slot when building the input map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggLeafSource {
    /// The slot's proof was produced by the `agg_utxo` circuit.
    AggUtxo,
    /// The slot's proof was produced by the `agg_escrow` circuit.
    AggEscrow,
}

/// The data required to prove an AggAgg transaction, this aggregates two
/// 1-level aggregator proofs into a single proof. Expects each new_root
/// from the previous proof to be the same as the old_root of the next
/// one.
#[derive(Debug, Clone)]
pub struct AggAgg {
    /// The proofs for the AggAgg transaction. Each proof shares the
    /// `AggUtxoProof` shape regardless of whether it was produced by
    /// `agg_utxo` or `agg_escrow`.
    pub proofs: [AggUtxoProof; 2],
    /// Which 1-level aggregator produced each slot's proof. The prover
    /// uses this to feed the matching verification key into the
    /// per-slot Noir input.
    pub sources: [AggLeafSource; 2],
}

impl AggAgg {
    /// Create a new AggAgg from two `agg_utxo` proofs.
    #[must_use]
    pub fn new(proofs: [AggUtxoProof; 2]) -> Self {
        Self {
            proofs,
            sources: [AggLeafSource::AggUtxo, AggLeafSource::AggUtxo],
        }
    }

    /// Create a new AggAgg with explicit per-slot leaf-source tags so
    /// that one slot can carry an `agg_utxo` proof and the other an
    /// `agg_escrow` proof (or any other mix).
    #[must_use]
    pub fn new_mixed(proofs: [AggUtxoProof; 2], sources: [AggLeafSource; 2]) -> Self {
        Self { proofs, sources }
    }

    /// Get the old root of the AggAgg transaction
    #[must_use]
    pub fn old_root(&self) -> Element {
        self.proofs[0].public_inputs.old_root
    }

    /// Get the new root of the AggAgg transaction
    #[must_use]
    pub fn new_root(&self) -> Element {
        if self.proofs[1].public_inputs.is_padding() {
            self.proofs[0].public_inputs.new_root
        } else {
            self.proofs[1].public_inputs.new_root
        }
    }

    /// Get the messages from the UTXO proofs
    #[must_use]
    pub fn messages(&self) -> Vec<Element> {
        self.proofs
            .iter()
            .flat_map(|p| p.public_inputs.messages)
            .collect::<Vec<_>>()
    }

    /// Get the public inputs for the AggAgg circuit
    #[must_use]
    pub fn public_inputs(&self) -> AggAggPublicInput {
        AggAggPublicInput {
            old_root: self.old_root(),
            new_root: self.new_root(),
            commit_hash: self.commit_hash(),
            messages: self.messages(),
        }
    }

    /// Commit hash of the agg_agg (will be posted onchain and verified by Celestia)
    #[must_use]
    pub fn commit_hash(&self) -> Element {
        hash_merge([
            self.proofs[0].public_inputs.commit_hash,
            self.proofs[1].public_inputs.commit_hash,
        ])
    }
}

/// Raw proof bytes for AggAgg proof (no public inputs)
#[derive(Debug, Default, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AggAggProofBytes(
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub Vec<u8>,
);

/// The public input for a AggAgg transaction
#[derive(Default, Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AggAggPublicInput {
    /// The old root of the tree
    pub old_root: Element,
    /// The new root of the tree
    pub new_root: Element,
    /// Commit hash
    pub commit_hash: Element,
    /// The messages of the transactions
    pub messages: Vec<Element>,
}

impl AggAggPublicInput {
    /// Convert the AggAggPublicInput to a AggAggPublicInputBytes
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 * (1 + 1 + 1 + 2 * 15));

        bytes.extend(self.old_root.to_be_bytes());
        bytes.extend(self.new_root.to_be_bytes());
        bytes.extend(self.commit_hash.to_be_bytes());

        for message in &self.messages {
            bytes.extend(message.to_be_bytes());
        }

        bytes
    }
}

/// The output proof for a AggAgg transaction
#[derive(Debug, Default, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AggAggProof {
    /// The proof for the AggAgg transaction
    pub proof: AggAggProofBytes,
    /// The public input for the AggAgg transaction
    pub public_inputs: AggAggPublicInput,
    /// KZG accumulator inputs
    pub kzg: Vec<Element>,
}

impl ToBytes for AggAggProof {
    /// Convert the AggAggProof to a AggAggProofBytes
    fn to_bytes(&self) -> Vec<u8> {
        // TODO: move to impl detail of proving backend
        let pi = self.public_inputs.to_bytes();
        let kzg = self
            .kzg
            .iter()
            .flat_map(|e| e.to_be_bytes())
            .collect::<Vec<u8>>();
        let proof = &self.proof.0;
        [pi.as_slice(), kzg.as_slice(), proof.as_slice()].concat()
    }
}
