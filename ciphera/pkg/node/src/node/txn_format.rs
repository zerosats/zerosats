use borsh::{BorshDeserialize, BorshSerialize};
use primitives::block_height::BlockHeight;
use serde::{Deserialize, Serialize};
use wire_message::WireMessage;
use zk_primitives::{LeafProof, UtxoProof};

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TxnMetadata {
    pub block_height: BlockHeight,
    pub block_time: Option<u64>,
    pub block_hash: [u8; 32],
    pub block_txn_index: u32,
}

/// On-disk transaction format.
///
/// `V1(UtxoProof, …)` is the original utxo-only layout that the
/// `block_store` was writing before the heterogeneous-aggregation
/// refactor; old databases on operator boxes contain V1 entries.
///
/// `V2(LeafProof, …)` is the new layout that the proposal path writes
/// since `LeafProof` (utxo or escrow) became the in-memory txn type.
/// `upgrade_once` lifts V1 entries into V2 by wrapping the raw
/// `UtxoProof` as `LeafProof::Utxo`, so consumers only ever see V2 at
/// the trait-impl boundary.
#[derive(Debug, Clone)]
#[wire_message::wire_message]
pub enum TxnFormat {
    V1(UtxoProof, TxnMetadata),
    V2(LeafProof, TxnMetadata),
}

impl WireMessage for TxnFormat {
    type Ctx = ();
    type Err = core::convert::Infallible;

    fn version(&self) -> u64 {
        match self {
            Self::V1(_, _) => 1,
            Self::V2(_, _) => 2,
        }
    }

    fn upgrade_once(self, _ctx: &mut Self::Ctx) -> Result<Self, wire_message::Error> {
        match self {
            // V1 → V2: lift the bare `UtxoProof` into the new
            // `LeafProof::Utxo` wrapper so downstream code only needs
            // a single dispatch.
            Self::V1(utxo, metadata) => Ok(Self::V2(LeafProof::Utxo(utxo), metadata)),
            Self::V2(_, _) => Err(Self::max_version_error()),
        }
    }
}

impl TxnFormat {
    /// Borrow the inner txn metadata regardless of variant.
    pub(crate) fn metadata(&self) -> &TxnMetadata {
        match self {
            Self::V1(_, m) | Self::V2(_, m) => m,
        }
    }

    /// Mutable borrow of the inner txn metadata regardless of variant.
    pub(crate) fn metadata_mut(&mut self) -> &mut TxnMetadata {
        match self {
            Self::V1(_, m) | Self::V2(_, m) => m,
        }
    }

    /// Consume the wrapper and yield a `LeafProof` regardless of which
    /// version was on disk. Cheap on V2, lifts V1 to `LeafProof::Utxo`.
    pub(crate) fn into_leaf_proof(self) -> (LeafProof, TxnMetadata) {
        match self {
            Self::V1(utxo, m) => (LeafProof::Utxo(utxo), m),
            Self::V2(leaf, m) => (leaf, m),
        }
    }
}

impl block_store::Transaction for TxnFormat {
    fn txn_hash(&self) -> [u8; 32] {
        match self {
            Self::V1(txn, _) => txn.hash().to_be_bytes(),
            Self::V2(txn, _) => txn.hash().to_be_bytes(),
        }
    }

    fn input_elements(&self) -> Vec<element::Element> {
        let pi = match self {
            Self::V1(p, _) => &p.public_inputs,
            Self::V2(p, _) => p.public_inputs(),
        };
        pi.input_commitments
            .iter()
            .filter(|c| !c.is_zero())
            .copied()
            .collect()
    }

    fn output_elements(&self) -> Vec<element::Element> {
        let pi = match self {
            Self::V1(p, _) => &p.public_inputs,
            Self::V2(p, _) => p.public_inputs(),
        };
        pi.output_commitments
            .iter()
            .filter(|c| !c.is_zero())
            .copied()
            .collect()
    }

    fn mint_hash(&self) -> Option<element::Element> {
        let kind_messages = match self {
            Self::V1(p, _) => p.kind_messages(),
            Self::V2(p, _) => p.kind_messages(),
        };
        match kind_messages {
            zk_primitives::UtxoKindMessages::Mint(m) => Some(m.mint_hash),
            zk_primitives::UtxoKindMessages::Burn(_) => None,
            zk_primitives::UtxoKindMessages::None => None,
        }
    }
}
