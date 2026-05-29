//! Rust primitive for the Noir `agg_escrow` 1-level aggregator.
//!
//! `agg_escrow` is the sibling of [`crate::AggUtxo`]: it batches three
//! `escrow` leaf proofs (rather than three `utxo` leaf proofs) into a
//! single bundle whose public inputs (`messages[15]`, `old_root`,
//! `new_root`, `commit_hash`) and proof body share `AggUtxoProof`'s
//! exact shape. Because the output shape is the same, an `agg_escrow`
//! proof slots into [`crate::AggAgg`] in place of an `agg_utxo` proof
//! provided the matching slot of [`crate::AggAgg::sources`] is set to
//! [`crate::AggLeafSource::Escrow`].
use crate::{AggUtxoProof, EscrowProof, MerklePath, ToBytes, UtxoKind};
use element::Element;
use hash::hash_merge;

/// Data required to prove an `agg_escrow` aggregation. Mirrors
/// [`crate::AggUtxo`] but takes [`EscrowProofBundleWithMerkleProofs`].
#[derive(Debug, Clone)]
pub struct AggEscrow {
    /// The three escrow leaf proofs to aggregate.
    pub proofs: [EscrowProofBundleWithMerkleProofs; 3],
    /// The tree root before any of these escrow proofs are applied.
    pub old_root: Element,
    /// The tree root after the aggregated escrow proofs have been
    /// applied.
    pub new_root: Element,
}

impl AggEscrow {
    /// Create a new `AggEscrow` bundle. Padding slots can be filled
    /// with [`EscrowProofBundleWithMerkleProofs::default`].
    #[must_use]
    pub fn new(
        proofs: [EscrowProofBundleWithMerkleProofs; 3],
        old_root: Element,
        new_root: Element,
    ) -> Self {
        Self {
            proofs,
            old_root,
            new_root,
        }
    }

    /// Commit hash over the three leaf-proof commit hashes. Same shape
    /// as [`crate::AggUtxo::commit_hash`] so an `agg_escrow` proof can
    /// participate in `agg_agg`'s top-level commit-hash chain.
    #[must_use]
    pub fn commit_hash(&self) -> Element {
        hash_merge([
            self.proofs[0].escrow_proof.public_inputs.commit_hash(),
            self.proofs[1].escrow_proof.public_inputs.commit_hash(),
            self.proofs[2].escrow_proof.public_inputs.commit_hash(),
        ])
    }

    /// Flatten the per-proof message vectors into the 15-field public
    /// `messages` slot. `escrow` only emits `Send` today, so every real
    /// slot contributes zero messages; the helper is kept symmetric
    /// with [`crate::AggUtxo::messages`] for forward compatibility.
    #[must_use]
    pub fn messages(&self) -> [Element; 15] {
        let mut messages = [Element::ZERO; 15];
        let mut index = 0;

        for proof in &self.proofs {
            let proof_messages = match proof.escrow_proof.kind() {
                UtxoKind::Null | UtxoKind::Send => &[][..],
                UtxoKind::Mint => &proof.escrow_proof.public_inputs.messages[..4],
                UtxoKind::Burn | UtxoKind::SlowBurn => &proof.escrow_proof.public_inputs.messages[..],
            };

            for &message in proof_messages {
                messages[index] = message;
                index += 1;
            }
        }

        messages
    }
}

/// An escrow leaf proof bundled with the Merkle paths used by
/// `agg_escrow` to fold its commitments into the tree. Mirrors
/// [`crate::UtxoProofBundleWithMerkleProofs`].
#[derive(Debug, Clone)]
pub struct EscrowProofBundleWithMerkleProofs {
    /// The escrow leaf proof being aggregated.
    pub escrow_proof: EscrowProof,
    /// Merkle paths for the two input commitments (used to remove them
    /// from the tree).
    pub input_merkle_paths: [MerklePath<161>; 2],
    /// Merkle paths for the two output commitments (used to insert
    /// them into the tree).
    pub output_merkle_paths: [MerklePath<161>; 2],
}

impl EscrowProofBundleWithMerkleProofs {
    /// Bundle a single escrow proof with its 4 Merkle paths
    /// (two inputs followed by two outputs).
    #[must_use]
    pub fn new(escrow_proof: EscrowProof, merkle_paths: &[MerklePath<161>; 4]) -> Self {
        Self {
            escrow_proof,
            input_merkle_paths: [merkle_paths[0].clone(), merkle_paths[1].clone()],
            output_merkle_paths: [merkle_paths[2].clone(), merkle_paths[3].clone()],
        }
    }
}

impl Default for EscrowProofBundleWithMerkleProofs {
    fn default() -> EscrowProofBundleWithMerkleProofs {
        let merkle_path = MerklePath::default();
        Self {
            escrow_proof: EscrowProof::default(),
            input_merkle_paths: [merkle_path.clone(), merkle_path.clone()],
            output_merkle_paths: [merkle_path.clone(), merkle_path.clone()],
        }
    }
}

/// Output type for an [`AggEscrow`] proof. Wraps [`AggUtxoProof`]
/// because the agg_escrow circuit produces a byte-for-byte identical
/// proof + public-input shape; the wrapper is what lets the backend
/// dispatch `Verify` to `agg_escrow`'s verification key instead of
/// `agg_utxo`'s. Unwrap with [`AggEscrowProof::into_inner`] when feeding
/// the proof to [`crate::AggAgg::new_mixed`] (the top-level aggregator
/// tracks the leaf source via [`crate::AggLeafSource`]).
#[derive(Default, Debug, Clone)]
pub struct AggEscrowProof(pub AggUtxoProof);

impl AggEscrowProof {
    /// Borrow the underlying `AggUtxoProof`.
    #[must_use]
    pub fn as_agg_utxo_proof(&self) -> &AggUtxoProof {
        &self.0
    }

    /// Consume the wrapper, yielding the underlying `AggUtxoProof`.
    /// Use this when handing the proof to `agg_agg`, which stores
    /// per-slot leaf provenance separately in `AggAgg::sources`.
    #[must_use]
    pub fn into_inner(self) -> AggUtxoProof {
        self.0
    }
}

impl From<AggEscrowProof> for AggUtxoProof {
    fn from(value: AggEscrowProof) -> Self {
        value.0
    }
}

impl ToBytes for AggEscrowProof {
    fn to_bytes(&self) -> Vec<u8> {
        self.0.to_bytes()
    }
}
