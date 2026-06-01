use borsh::{BorshDeserialize, BorshSerialize};
use element::Element;
use primitives::{
    block_height::BlockHeight, hash::CryptoHash, sig::Signature,
};
use wire_message::WireMessage;
use zk_primitives::{LeafProof, UtxoProof};

use crate::{
    TxnFormat,
    block::{Block, BlockContent, BlockHeader, BlockState},
};

use super::txn_format::TxnMetadata;

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BlockMetadata {
    pub timestamp_unix_s: Option<u64>,
}

/// Back-compat structures mirroring the **old** in-memory layout of
/// `Block` (`BlockState::txns: Vec<UtxoProof>`) before the
/// `LeafProof` refactor. These exist solely so existing on-disk
/// `BlockFormat::V1` and `V2` entries -- written before the refactor
/// -- still deserialize cleanly. They are never returned to call
/// sites: every variant promotes its inner block to the current
/// `Block` (with `Vec<LeafProof>`) via `into_block`.
///
/// **Do not change these field shapes.** They have to match what the
/// pre-refactor borsh encoder produced byte-for-byte.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct LegacyBlock {
    pub content: LegacyBlockContent,
    pub signature: Signature,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct LegacyBlockContent {
    pub header: BlockHeader,
    pub state: LegacyBlockState,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct LegacyBlockState {
    pub root_hash: Element,
    pub txns: Vec<UtxoProof>,
}

impl LegacyBlock {
    /// Promote a legacy block (utxo-only txns) into the current
    /// in-memory `Block` representation (`LeafProof` txns).
    fn into_block(self) -> Block {
        Block {
            content: BlockContent {
                header: self.content.header,
                state: BlockState {
                    root_hash: self.content.state.root_hash,
                    txns: self
                        .content
                        .state
                        .txns
                        .into_iter()
                        .map(LeafProof::Utxo)
                        .collect(),
                },
            },
            signature: self.signature,
        }
    }

    pub(crate) fn block_height(&self) -> BlockHeight {
        self.content.header.height
    }

    pub(crate) fn block_hash(&self) -> [u8; 32] {
        // Hashing semantics are identical to the modern `Block::hash`
        // because the header + root_hash field layout is unchanged.
        use ethereum_types::U256;
        use sha3::{Digest, Keccak256};

        let root_hash = self.content.state.root_hash.to_be_bytes();
        let header_hash: [u8; 32] = {
            #[allow(clippy::disallowed_methods)]
            let mut hasher = Keccak256::new();
            #[allow(clippy::disallowed_methods)]
            hasher.update(borsh::to_vec(&self.content.header).unwrap());
            let h: [u8; 32] = hasher.finalize().into();
            h
        };

        let mut hasher = Keccak256::new();
        hasher.update(root_hash);
        let mut height_bytes = [0u8; 32];
        U256::from(self.content.header.height.0).to_big_endian(&mut height_bytes);
        hasher.update(height_bytes);
        hasher.update(header_hash);
        let result = hasher.finalize();
        let arr: [u8; 32] = result.into();
        *CryptoHash::new(arr).inner()
    }
}

/// On-disk block format.
///
/// * `V1(LegacyBlock)` -- original, utxo-only.
/// * `V2(LegacyBlock, BlockMetadata)` -- added the wall-clock
///    `timestamp_unix_s`; still utxo-only.
/// * `V3(Block, BlockMetadata)` -- current. Block's `state.txns` is
///    `Vec<LeafProof>` so utxo and escrow leaves share one channel.
///    `upgrade_once` chains V1 → V2 → V3 by lifting `UtxoProof` to
///    `LeafProof::Utxo`.
#[derive(Debug, Clone)]
#[wire_message::wire_message]
pub enum BlockFormat {
    V1(LegacyBlock),
    V2(LegacyBlock, BlockMetadata),
    V3(Block, BlockMetadata),
}

impl BlockFormat {
    pub(crate) fn into_block(self) -> Block {
        match self {
            Self::V1(block) => block.into_block(),
            Self::V2(block, _) => block.into_block(),
            Self::V3(block, _) => block,
        }
    }

    pub(crate) fn metadata(&self) -> &BlockMetadata {
        match self {
            Self::V1(_) => &BlockMetadata {
                timestamp_unix_s: None,
            },
            Self::V2(_, metadata) => metadata,
            Self::V3(_, metadata) => metadata,
        }
    }
}

impl WireMessage for BlockFormat {
    type Ctx = ();
    type Err = core::convert::Infallible;

    fn version(&self) -> u64 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_, _) => 2,
            Self::V3(_, _) => 3,
        }
    }

    fn upgrade_once(self, _ctx: &mut Self::Ctx) -> Result<Self, wire_message::Error> {
        match self {
            Self::V1(block) => Ok(Self::V2(
                block,
                BlockMetadata {
                    timestamp_unix_s: None,
                },
            )),
            Self::V2(legacy, metadata) => Ok(Self::V3(legacy.into_block(), metadata)),
            Self::V3(_, _) => Err(Self::max_version_error()),
        }
    }
}

impl block_store::Block for BlockFormat {
    type Txn = TxnFormat;

    fn block_height(&self) -> BlockHeight {
        match self {
            Self::V1(block) => block.block_height(),
            Self::V2(block, _) => block.block_height(),
            Self::V3(block, _) => block.block_height(),
        }
    }

    fn block_hash(&self) -> [u8; 32] {
        match self {
            Self::V1(block) => block.block_hash(),
            Self::V2(block, _) => block.block_hash(),
            Self::V3(block, _) => block.block_hash(),
        }
    }

    fn txns(&self) -> Vec<Self::Txn> {
        match self {
            // V1/V2 read back the legacy `UtxoProof`s and emit the
            // legacy `TxnFormat::V1` -- preserving on-disk semantics
            // bit-for-bit means anything downstream that round-trips
            // an old block sees what it always saw.
            Self::V1(block) | Self::V2(block, _) => {
                let block_height = block.block_height();
                let block_hash = block.block_hash();
                block
                    .content
                    .state
                    .txns
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        TxnFormat::V1(
                            t.clone(),
                            TxnMetadata {
                                block_height,
                                block_time: match self {
                                    Self::V2(_, m) => m.timestamp_unix_s,
                                    _ => None,
                                },
                                block_hash,
                                block_txn_index: i as u32,
                            },
                        )
                    })
                    .collect()
            }
            Self::V3(block, block_metadata) => {
                let mut txns = block.txns();
                for txn in &mut txns {
                    txn.metadata_mut().block_time = block_metadata.timestamp_unix_s;
                }
                txns
            }
        }
    }
}

impl block_store::Block for Block {
    type Txn = TxnFormat;

    fn block_height(&self) -> BlockHeight {
        self.content.header.height
    }

    fn block_hash(&self) -> [u8; 32] {
        self.hash().into_inner()
    }

    fn txns(&self) -> Vec<Self::Txn> {
        self.content
            .state
            .txns
            .iter()
            .enumerate()
            .map(|(i, t)| {
                // New blocks always write V2 (carries `LeafProof`).
                // The block_store will preserve that tag round-trip.
                TxnFormat::V2(
                    t.clone(),
                    TxnMetadata {
                        block_height: self.block_height(),
                        block_time: None,
                        block_hash: self.block_hash(),
                        block_txn_index: i as u32,
                    },
                )
            })
            .collect()
    }
}
