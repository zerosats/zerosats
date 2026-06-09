//! Wire-level leaf proof: either a `UtxoProof` (Poseidon-key spend) or
//! an `EscrowProof` (escrow-circuit spend -- HTLC, signature32,
//! signature32sha, or timelocked Poseidon-key). Both share the same
//! [`UtxoPublicInput`] shape (4 commitments + 5 messages), the same
//! `508 * 32`-byte proof body, and the same `messages_length` on the
//! on-chain verifier; the only difference is which leaf VK the
//! 1-level aggregator (`agg_utxo` vs `agg_escrow`) accepts. The
//! `LeafProof` enum exists so the node mempool / network / block
//! storage can carry either flavour through the same channels, and so
//! the prover can sort them into separate aggregator buckets before
//! combining via `AggAgg::new_mixed`.

use crate::{EscrowProof, UtxoKind, UtxoKindMessages, UtxoProof, UtxoPublicInput};
use borsh::{BorshDeserialize, BorshSerialize};
use element::Element;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A single leaf-circuit proof that the node mempool accepts.
///
/// The JSON wire form (used by `/v0/transaction` and any future
/// leaf-proof endpoints) is **backward compatible** with the
/// pre-refactor `UtxoProof`-only shape: deserialization first tries
/// the modern tagged form (`{"kind":"utxo"|"escrow", …}`) and falls
/// back to a bare `UtxoProof` (no `kind` field) which is wrapped as
/// `LeafProof::Utxo`. Modern clients always emit the tagged form via
/// the custom `Serialize` below.
///
/// The borsh wire form is the default-derived enum tag (one byte: 0
/// for `Utxo`, 1 for `Escrow`); pre-refactor on-disk entries are
/// handled by the version-bumped `TxnFormat` / `BlockFormat`
/// `wire_message` chains, not at this layer.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum LeafProof {
    /// Poseidon-key spend (the original `utxo` circuit). Carries the
    /// standard `UtxoProof` shape used by mints, burns, and ordinary
    /// sends.
    Utxo(UtxoProof),
    /// Escrow-circuit spend (`escrow`/`agg_escrow`). Carries the same
    /// public-input shape but is verified against the escrow leaf VK.
    Escrow(EscrowProof),
}

// ---------------------------------------------------------------------------
// Custom serde -- internally-tagged for modern clients, untagged
// fallback to a bare UtxoProof for legacy clients still posting the
// pre-LeafProof shape.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Tagged {
    Utxo(UtxoProof),
    Escrow(EscrowProof),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Wire {
    /// Modern: `{"kind":"utxo"|"escrow", <inner fields>}`.
    Tagged(Tagged),
    /// Legacy: a bare `UtxoProof` payload with no `kind` field. The
    /// old CLI client and the `pkg/contracts` rollup client both
    /// historically sent this. Treated as `LeafProof::Utxo`.
    LegacyUtxo(UtxoProof),
}

impl Serialize for LeafProof {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let tagged = match self {
            LeafProof::Utxo(p) => Tagged::Utxo(p.clone()),
            LeafProof::Escrow(p) => Tagged::Escrow(p.clone()),
        };
        tagged.serialize(s)
    }
}

impl<'de> Deserialize<'de> for LeafProof {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Wire::deserialize(d)? {
            Wire::Tagged(Tagged::Utxo(p)) | Wire::LegacyUtxo(p) => Ok(LeafProof::Utxo(p)),
            Wire::Tagged(Tagged::Escrow(p)) => Ok(LeafProof::Escrow(p)),
        }
    }
}

impl LeafProof {
    /// Inner `UtxoPublicInput`. Both variants share this shape -- the
    /// mempool, block storage, and prover all index by the commitments
    /// in here.
    #[must_use]
    pub fn public_inputs(&self) -> &UtxoPublicInput {
        match self {
            LeafProof::Utxo(p) => &p.public_inputs,
            LeafProof::Escrow(p) => &p.public_inputs,
        }
    }

    /// Stable hash uniquely identifying this proof; used as the
    /// mempool key.
    #[must_use]
    pub fn hash(&self) -> Element {
        match self {
            LeafProof::Utxo(p) => p.hash(),
            LeafProof::Escrow(p) => p.hash(),
        }
    }

    /// Transaction kind (Send / Mint / Burn / SlowBurn / Null).
    #[must_use]
    pub fn kind(&self) -> UtxoKind {
        match self {
            LeafProof::Utxo(p) => p.kind(),
            LeafProof::Escrow(p) => p.kind(),
        }
    }

    /// Structured per-kind messages (used by the node's mint-validation
    /// path and the burn substitutor).
    #[must_use]
    pub fn kind_messages(&self) -> UtxoKindMessages {
        match self {
            LeafProof::Utxo(p) => p.kind_messages(),
            LeafProof::Escrow(p) => p.kind_messages(),
        }
    }

    /// True if this is a default-constructed padding proof. Used by
    /// the prover to skip aggregator slots that have nothing in them.
    #[must_use]
    pub fn is_padding(&self) -> bool {
        match self {
            LeafProof::Utxo(p) => p.is_padding(),
            LeafProof::Escrow(p) => p.kind() == UtxoKind::Null,
        }
    }

    /// True if this is an escrow-circuit proof. Lets the prover route
    /// the proof to `agg_escrow` instead of `agg_utxo`.
    #[must_use]
    pub fn is_escrow(&self) -> bool {
        matches!(self, LeafProof::Escrow(_))
    }

    /// Short, stable, lower-case label for the leaf-circuit flavour
    /// (`"utxo"` or `"escrow"`). Intended as a structured-logging
    /// field so validator / prover logs can be filtered or grepped by
    /// flavour without re-deriving it at every call site.
    #[must_use]
    pub fn flavour(&self) -> &'static str {
        match self {
            LeafProof::Utxo(_) => "utxo",
            LeafProof::Escrow(_) => "escrow",
        }
    }

    /// `Some(proof)` if this is a utxo-circuit proof, else `None`.
    /// Convenience for the burn-substitutor and other code paths that
    /// only care about the `UtxoProof` flavour today.
    #[must_use]
    pub fn as_utxo(&self) -> Option<&UtxoProof> {
        match self {
            LeafProof::Utxo(p) => Some(p),
            LeafProof::Escrow(_) => None,
        }
    }

    /// `Some(proof)` if this is an escrow-circuit proof, else `None`.
    #[must_use]
    pub fn as_escrow(&self) -> Option<&EscrowProof> {
        match self {
            LeafProof::Utxo(_) => None,
            LeafProof::Escrow(p) => Some(p),
        }
    }
}

/// Count how many leaf proofs of each flavour appear in `proofs`,
/// returning `(utxo, escrow)`. Used by the validator and prover to emit
/// a one-line per-block summary that highlights the leaf mix at `info`
/// level (e.g. `"2 utxo + 1 escrow"`) without dumping every proof.
#[must_use]
pub fn count_flavours<'a>(proofs: impl IntoIterator<Item = &'a LeafProof>) -> (usize, usize) {
    let mut counts = (0, 0);
    for proof in proofs {
        match proof {
            LeafProof::Utxo(_) => counts.0 += 1,
            LeafProof::Escrow(_) => counts.1 += 1,
        }
    }
    counts
}

impl Default for LeafProof {
    /// Default is a padding `UtxoProof`. Matches what
    /// `prover::generate_aggregate_proof` does today when a slot is
    /// empty.
    fn default() -> Self {
        LeafProof::Utxo(UtxoProof::default())
    }
}

impl From<UtxoProof> for LeafProof {
    fn from(p: UtxoProof) -> Self {
        LeafProof::Utxo(p)
    }
}

impl From<EscrowProof> for LeafProof {
    fn from(p: EscrowProof) -> Self {
        LeafProof::Escrow(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_bare_utxo_json_deserializes_as_leaf_utxo() {
        // The pre-LeafProof wire shape: a bare `UtxoProof` JSON with
        // no `kind` field. The node mempool needs to keep accepting
        // these so older clients survive the upgrade.
        let utxo = UtxoProof::default();
        let bare = serde_json::to_string(&utxo).unwrap();
        let leaf: LeafProof = serde_json::from_str(&bare).unwrap();
        match leaf {
            LeafProof::Utxo(p) => assert_eq!(p.public_inputs, utxo.public_inputs),
            LeafProof::Escrow(_) => panic!("bare UtxoProof must round-trip as LeafProof::Utxo"),
        }
    }

    #[test]
    fn modern_tagged_json_round_trips() {
        let utxo_leaf = LeafProof::Utxo(UtxoProof::default());
        let s = serde_json::to_string(&utxo_leaf).unwrap();
        assert!(s.contains("\"kind\":\"utxo\""));
        let back: LeafProof = serde_json::from_str(&s).unwrap();
        assert_eq!(utxo_leaf, back);

        let escrow_leaf = LeafProof::Escrow(EscrowProof::default());
        let s = serde_json::to_string(&escrow_leaf).unwrap();
        assert!(s.contains("\"kind\":\"escrow\""));
        let back: LeafProof = serde_json::from_str(&s).unwrap();
        assert_eq!(escrow_leaf, back);
    }
}

impl PartialEq for LeafProof {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LeafProof::Utxo(a), LeafProof::Utxo(b)) => a == b,
            (LeafProof::Escrow(a), LeafProof::Escrow(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for LeafProof {}
