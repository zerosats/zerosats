#![warn(clippy::unwrap_used, clippy::expect_used)]
#![deny(clippy::disallowed_methods)]

mod constants;
pub mod smirk_metadata;
use crate::constants::{
    MERKLE_TREE_DEPTH, MERKLE_TREE_PATH_DEPTH, UTXO_AGG_LEAVES, UTXO_AGG_NUMBER, UTXO_AGGREGATIONS,
};
use barretenberg::Prove;
use borsh::{BorshDeserialize, BorshSerialize};
pub use constants::MAXIMUM_TXNS;
use contracts::RollupContract;
use element::Element;
use ethereum_types::H256;
use primitives::sig::Signature;
use smirk::{
    Path, Tree,
    hash_cache::{NoopHashCache, SimpleHashCache},
};
use smirk_metadata::SmirkMetadata;
use std::sync::Arc;
use tracing::{debug, info, warn};
use web3::ethabi;
use zk_primitives::{
    AggAgg, AggAggProof, AggEscrow, AggEscrowProof, AggLeafSource, AggUtxo, AggUtxoProof,
    EscrowProof, EscrowProofBundleWithMerkleProofs, LeafProof, MerklePath, UtxoProof,
    UtxoProofBundleWithMerkleProofs,
};

type Result<T, E = Error> = std::result::Result<T, E>;

type MerkleTree<C = NoopHashCache> = Tree<MERKLE_TREE_DEPTH, SmirkMetadata, C>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to convert H256 to bn256::Fr")]
    ConvertH256ToBn256Fr(H256),

    // Temporary workaround, sometimes rollup transactions get stuck. Restarting the prover fixes it
    #[error("rollup transaction timed out")]
    RollupTransactionTimeout,

    #[error("rollup transaction dropped from mempool: {0}")]
    RollupTransactionDropped(H256),

    #[error("rollup transaction reverted: {0}")]
    RollupTransactionReverted(H256),

    #[error("from hex error")]
    FromHex(#[from] rustc_hex::FromHexError),

    #[error("web3 error")]
    Web3(#[from] web3::Error),

    #[error("ethabi error")]
    EthAbi(#[from] ethabi::Error),

    #[error("TryFromSlice error")]
    TryFromSlice(#[from] std::array::TryFromSliceError),

    #[error("web3 contract error")]
    Web3Contract(#[from] web3::contract::Error),

    #[error("serde_json error")]
    SerdeJson(#[from] serde_json::Error),

    #[error("secp256k1 error")]
    Secp256k1(#[from] secp256k1::Error),

    #[error("smirk storage error")]
    Smirk(#[from] smirk::storage::Error),

    #[error("smirk collision error")]
    SmirkCollision(#[from] smirk::CollisionError),

    #[error("contract error")]
    Contract(#[from] contracts::Error),

    #[error("tokio task join error")]
    TokioTaskJoin(#[from] tokio::task::JoinError),

    #[error("barretenberg prove error: {0}")]
    BarretenbergProve(String),

    #[error("vec to array conversion failed, expected {expected}, got {actual}")]
    VecToArrayConversion { expected: usize, actual: usize },
}

#[derive(Debug, Clone)]
pub struct Transaction {
    /// Either a Utxo or Escrow leaf proof. `generate_aggregate_proof`
    /// routes the two flavours into separate aggregator buckets
    /// (`AggUtxo` vs `AggEscrow`) before combining them via
    /// `AggAgg::new_mixed`.
    pub proof: LeafProof,
}

impl Transaction {
    /// Wrap a leaf proof for the prover mempool.
    pub fn new(proof: LeafProof) -> Self {
        Self { proof }
    }

    /// Convenience constructor for the legacy `UtxoProof`-only call
    /// sites (the burn-substitutor, the CLI's `client.transaction`
    /// path, the node mempool's existing receive paths). New code
    /// should prefer `Transaction::new(LeafProof::Escrow(...))` when
    /// pushing an escrow proof.
    pub fn from_utxo(proof: UtxoProof) -> Self {
        Self {
            proof: LeafProof::Utxo(proof),
        }
    }

    /// Convenience constructor for an escrow leaf proof.
    pub fn from_escrow(proof: EscrowProof) -> Self {
        Self {
            proof: LeafProof::Escrow(proof),
        }
    }
}

#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct RollupInput {
    proof: AggAggProof,
    height: u64,
    other_hash: [u8; 32],
    signatures: Vec<Signature>,
}

impl RollupInput {
    pub fn new(
        proof: AggAggProof,
        height: u64,
        other_hash: [u8; 32],
        signatures: Vec<Signature>,
    ) -> Self {
        Self {
            proof,
            height,
            other_hash,
            signatures,
        }
    }

    pub fn old_root(&self) -> Element {
        self.proof.public_inputs.old_root
    }

    pub fn new_root(&self) -> Element {
        self.proof.public_inputs.new_root
    }

    pub fn height(&self) -> u64 {
        self.height
    }
}

pub struct Prover {
    contract: RollupContract,
}

impl Prover {
    pub fn new(contract: RollupContract) -> Self {
        Self { contract }
    }

    #[tracing::instrument(err, skip_all, fields(height, txns_len = txns.len()))]
    pub async fn prove(
        self: &Arc<Self>,
        notes_tree: &MerkleTree<SimpleHashCache>,
        height: u64,
        txns: [Option<Transaction>; MAXIMUM_TXNS],
    ) -> Result<AggAggProof> {
        // `txns` is padded to `MAXIMUM_TXNS` with `None`; count only the
        // real leaves and split by flavour so the log reflects the
        // actual utxo / escrow workload rather than the padded length.
        let (utxo_count, escrow_count) = txns.iter().flatten().fold(
            (0usize, 0usize),
            |(utxo, escrow), t| match t.proof.is_escrow() {
                true => (utxo, escrow + 1),
                false => (utxo + 1, escrow),
            },
        );
        info!(
            height,
            utxo = utxo_count,
            escrow = escrow_count,
            "Bundling {utxo_count} utxo + {escrow_count} escrow leaf proof(s) and proving new root hash"
        );

        let proof = tokio::task::spawn_blocking({
            let s: Arc<Prover> = Arc::clone(self);
            let mut tree = notes_tree.clone();

            move || s.generate_aggregate_proof(&mut tree, txns, height)
        })
        .await??;

        Ok(proof)
    }

    #[tracing::instrument(err, skip(self), fields(height = input.height))]
    pub async fn rollup(&self, input: &RollupInput) -> Result<H256> {
        info!("Sending proof and new root to EVM");

        let tx = self
            .contract
            .verify_block(
                &input.proof.proof.0,
                &input.proof.public_inputs.old_root,
                &input.proof.public_inputs.new_root,
                &input.proof.public_inputs.commit_hash,
                &input
                    .proof
                    .public_inputs
                    .messages
                    .clone()
                    .into_iter()
                    .collect::<Vec<Element>>(),
                &input.proof.kzg,
                input.other_hash,
                input.height,
                &input
                    .signatures
                    .iter()
                    .map(|s| &s.0[..])
                    .collect::<Vec<_>>(),
                1_000_000,
            )
            .await?;

        info!(?tx, "EVM root rollup update sent. Waiting for receipt...",);

        let wait_start = std::time::Instant::now();
        // Wall-clock deadline for the tx to reappear after going unknown,
        // matching the 60s semantics in Client::wait_for_transaction_inclusion().
        let mut unknown_deadline: Option<std::time::Instant> = None;
        loop {
            if let Some(receipt) = self.contract.client.transaction_receipt(tx).await? {
                if receipt.status == Some(0.into()) {
                    return Err(Error::RollupTransactionReverted(tx));
                }

                info!("EVM root rollup update confirmed");
                return Ok(tx);
            }

            if self.contract.client.transaction(tx).await?.is_none() {
                // RPC nodes can transiently return None for valid pending/included
                // transactions (load balancers, caching, eventual consistency).
                // Set a wall-clock deadline on first miss; do not reset on reappearance
                // so a flapping backend still gets a bounded wait.
                if unknown_deadline.is_none() {
                    unknown_deadline =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(15));
                    warn!(
                        ?tx,
                        "Transaction not found in mempool; \
                         will declare dropped if still absent in 15s"
                    );
                }
                #[allow(clippy::expect_used)]
                if std::time::Instant::now() > unknown_deadline.expect("just set above") {
                    return Err(Error::RollupTransactionDropped(tx));
                }
            }

            // Wait for a maximum of 5 minutes
            if wait_start.elapsed() > std::time::Duration::from_secs(5 * 60) {
                return Err(Error::RollupTransactionTimeout);
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    #[tracing::instrument(err, skip_all)]
    fn generate_aggregate_proof(
        &self,
        tree: &mut MerkleTree<SimpleHashCache>,
        txns: [Option<Transaction>; 6],
        current_block: u64,
    ) -> Result<AggAggProof, Error> {
        // Fan-out: route each leaf into the bucket whose aggregator
        // accepts it. Each bucket holds three `UTXO_AGG_NUMBER` slots
        // (utxo and escrow share the slot count because both leaf
        // circuits produce the same public-input shape).
        //
        // After fan-out, both buckets always hold exactly 3 entries
        // (real or padding). They then aggregate independently into
        // an `AggUtxoProof` / `AggEscrowProof` and combine in a
        // heterogeneous `AggAgg::new_mixed` if any escrow proof was
        // present, or the cheap homogeneous `AggAgg::new` otherwise.
        let mut utxo_bucket: Vec<Transaction> = Vec::with_capacity(UTXO_AGG_NUMBER);
        let mut escrow_bucket: Vec<Transaction> = Vec::with_capacity(UTXO_AGG_NUMBER);

        for slot in txns.into_iter() {
            match slot {
                Some(t) if t.proof.is_escrow() => escrow_bucket.push(t),
                Some(t) => utxo_bucket.push(t),
                None => {}
            }
        }

        // Pad each bucket up to UTXO_AGG_NUMBER (3) with default
        // padding proofs of the matching circuit flavour. The
        // aggregator circuits expect exactly 3 slots and treat
        // padding entries specially.
        while utxo_bucket.len() < UTXO_AGG_NUMBER {
            utxo_bucket.push(Transaction::new(LeafProof::Utxo(UtxoProof::default())));
        }
        while escrow_bucket.len() < UTXO_AGG_NUMBER {
            escrow_bucket.push(Transaction::new(LeafProof::Escrow(EscrowProof::default())));
        }

        // Report the real (non-padding) leaves routed into each bucket so
        // the flavour split through the aggregator is visible at `info`.
        let real_utxo = utxo_bucket.iter().filter(|t| !t.proof.is_padding()).count();
        let real_escrow = escrow_bucket
            .iter()
            .filter(|t| !t.proof.is_padding())
            .count();
        info!(
            block = current_block,
            utxo = real_utxo,
            escrow = real_escrow,
            "Aggregating {real_utxo} utxo + {real_escrow} escrow leaf proof(s) into AggUtxo + AggEscrow buckets"
        );

        #[allow(clippy::unwrap_used)]
        let utxo_slots: [Transaction; UTXO_AGG_NUMBER] = utxo_bucket.try_into().unwrap();
        #[allow(clippy::unwrap_used)]
        let escrow_slots: [Transaction; UTXO_AGG_NUMBER] = escrow_bucket.try_into().unwrap();

        let utxo_agg_proof = self.aggregate_utxo(tree, utxo_slots, current_block)?;
        let escrow_agg_proof = self.aggregate_escrow(tree, escrow_slots, current_block)?;

        // Slot 0 must always carry the non-padding aggregator so that
        // `AggAgg::old_root()` (which unconditionally reads proofs[0].old_root)
        // returns the real pre-block tree root, and so the Noir agg_agg
        // circuit's root-chaining assertion (proofs[0].new_root ==
        // proofs[1].old_root) is satisfiable.
        //
        // When the UTXO bucket is all-padding (escrow-only block) the default
        // AggUtxoProof has old_root = new_root = 0, which would cause both of
        // those invariants to fail.  Swap the slots so the real aggregator
        // (AggEscrow in that case) is always in slot 0.
        let (slot0, src0, slot1, src1) = if utxo_agg_proof.public_inputs.is_padding() {
            (
                escrow_agg_proof.into_inner(),
                AggLeafSource::AggEscrow,
                utxo_agg_proof,
                AggLeafSource::AggUtxo,
            )
        } else {
            (
                utxo_agg_proof,
                AggLeafSource::AggUtxo,
                escrow_agg_proof.into_inner(),
                AggLeafSource::AggEscrow,
            )
        };
        let agg_agg = AggAgg::new_mixed([slot0, slot1], [src0, src1]);
        let proof = agg_agg
            .prove()
            .map_err(|e| Error::BarretenbergProve(e.to_string()))?;

        Ok(proof)
    }

    // Suppress legacy UTXO_AGGREGATIONS lint: the constant is still
    // exported for downstream callers that quote it in scheduling /
    // capacity math. Keeping the use site lets us share one
    // assertion of the shape with the bucket loop above.
    #[allow(dead_code)]
    const _BUCKET_SHAPE_AGREES: () = assert!(UTXO_AGGREGATIONS == 2);

    #[tracing::instrument(err, skip_all)]
    fn aggregate_utxo(
        &self,
        tree: &mut MerkleTree<SimpleHashCache>,
        utxos: [Transaction; UTXO_AGG_NUMBER],
        current_block: u64,
    ) -> Result<AggUtxoProof, Error> {
        if utxos.iter().all(|utxo| utxo.proof.is_padding()) {
            debug!(
                block = current_block,
                "UTXO bucket is all padding; emitting default AggUtxoProof"
            );
            return Ok(AggUtxoProof::default());
        }

        let real = utxos.iter().filter(|u| !u.proof.is_padding()).count();
        debug!(
            block = current_block,
            real, "Proving AggUtxo over {real} real utxo leaf slot(s)"
        );

        let (_, old_tree, new_tree, merkle_paths) =
            self.gen_merkle_paths(tree, &utxos, current_block)?;

        // Chunk the merkle paths into 3 x 4
        let merkle_paths = merkle_paths.chunks(4).collect::<Vec<_>>();

        let mut utxo_proof_bundles = Vec::new();

        for (i, chunk) in merkle_paths.into_iter().enumerate() {
            let utxo = &utxos[i];

            let utxo_proof_bundle = match utxo.proof.is_padding() {
                false => {
                    // The fan-out in `generate_aggregate_proof` puts
                    // only `LeafProof::Utxo` entries in this bucket, so
                    // the `as_utxo()` unwrap is a real invariant -- if
                    // it fails the caller routed something into the
                    // wrong slot.
                    #[allow(clippy::unwrap_used)]
                    let utxo_proof = utxo.proof.as_utxo().cloned().unwrap();
                    UtxoProofBundleWithMerkleProofs::new(
                        utxo_proof,
                        &[
                            chunk[0].clone(),
                            chunk[1].clone(),
                            chunk[2].clone(),
                            chunk[3].clone(),
                        ],
                    )
                }
                true => UtxoProofBundleWithMerkleProofs::default(),
            };

            utxo_proof_bundles.push(utxo_proof_bundle);
        }

        let utxo_proof_bundles: [UtxoProofBundleWithMerkleProofs; UTXO_AGG_NUMBER] =
            utxo_proof_bundles
                .try_into()
                .map_err(|v: Vec<_>| Error::VecToArrayConversion {
                    expected: UTXO_AGG_NUMBER,
                    actual: v.len(),
                })?;

        let agg_utxo = AggUtxo::new(utxo_proof_bundles, old_tree, new_tree);

        let agg_utxo_proof = agg_utxo
            .prove()
            .map_err(|e| Error::BarretenbergProve(e.to_string()))?;

        Ok(agg_utxo_proof)
    }

    /// `aggregate_utxo`'s sibling for the escrow-circuit bucket. The
    /// shape is identical (`AggEscrow` takes `[EscrowProofBundleWithMerkleProofs; 3]`)
    /// and the Merkle-path bookkeeping is shared with `gen_merkle_paths`
    /// because the tree doesn't care which leaf circuit produced a
    /// commitment.
    #[tracing::instrument(err, skip_all)]
    fn aggregate_escrow(
        &self,
        tree: &mut MerkleTree<SimpleHashCache>,
        txns: [Transaction; UTXO_AGG_NUMBER],
        current_block: u64,
    ) -> Result<AggEscrowProof, Error> {
        if txns.iter().all(|t| t.proof.is_padding()) {
            debug!(
                block = current_block,
                "Escrow bucket is all padding; emitting default AggEscrowProof"
            );
            return Ok(AggEscrowProof::default());
        }

        let real = txns.iter().filter(|t| !t.proof.is_padding()).count();
        debug!(
            block = current_block,
            real, "Proving AggEscrow over {real} real escrow leaf slot(s)"
        );

        let (_, old_tree, new_tree, merkle_paths) =
            self.gen_merkle_paths(tree, &txns, current_block)?;
        let merkle_paths = merkle_paths.chunks(4).collect::<Vec<_>>();

        let mut escrow_proof_bundles = Vec::new();
        for (i, chunk) in merkle_paths.into_iter().enumerate() {
            let t = &txns[i];
            let bundle = match t.proof.is_padding() {
                false => {
                    #[allow(clippy::unwrap_used)]
                    let escrow_proof = t.proof.as_escrow().cloned().unwrap();
                    EscrowProofBundleWithMerkleProofs::new(
                        escrow_proof,
                        &[
                            chunk[0].clone(),
                            chunk[1].clone(),
                            chunk[2].clone(),
                            chunk[3].clone(),
                        ],
                    )
                }
                true => EscrowProofBundleWithMerkleProofs::default(),
            };
            escrow_proof_bundles.push(bundle);
        }

        let escrow_proof_bundles: [EscrowProofBundleWithMerkleProofs; UTXO_AGG_NUMBER] =
            escrow_proof_bundles
                .try_into()
                .map_err(|v: Vec<_>| Error::VecToArrayConversion {
                    expected: UTXO_AGG_NUMBER,
                    actual: v.len(),
                })?;

        let agg_escrow = AggEscrow::new(escrow_proof_bundles, old_tree, new_tree);
        let agg_escrow_proof = agg_escrow
            .prove()
            .map_err(|e| Error::BarretenbergProve(e.to_string()))?;
        Ok(agg_escrow_proof)
    }

    #[tracing::instrument(err, skip_all)]
    fn gen_merkle_paths(
        &self,
        tree: &mut MerkleTree<SimpleHashCache>,
        txns: &[Transaction; UTXO_AGG_NUMBER],
        current_block: u64,
    ) -> Result<(
        usize,
        Element,
        Element,
        [MerklePath<MERKLE_TREE_DEPTH>; UTXO_AGG_LEAVES],
    )> {
        let padding_path = MerklePath::default();

        let (merkle_paths, old_tree, new_tree) = {
            let old_tree = tree.root_hash();

            let mut merkle_paths = vec![];

            // Extract leaves to be inserted from proof
            for Transaction { proof } in txns {
                for leaf in proof.public_inputs().input_commitments {
                    if leaf.is_zero() {
                        merkle_paths.push(padding_path.clone());
                        continue;
                    }

                    merkle_paths.push(path_to_merkle_path(tree.path_for(leaf)));
                    tree.remove(leaf)?;
                }

                for leaf in proof.public_inputs().output_commitments {
                    if leaf.is_zero() {
                        merkle_paths.push(padding_path.clone());
                        continue;
                    }

                    // Insert the leaf into the tree
                    tree.insert(
                        leaf,
                        SmirkMetadata {
                            inserted_in: current_block,
                        },
                    )?;
                    // Then add the path to the merkle paths
                    merkle_paths.push(path_to_merkle_path(tree.path_for(leaf)));
                }
            }

            let new_tree = tree.root_hash();

            (merkle_paths, old_tree, new_tree)
        };
        let inserts_len: usize = merkle_paths.len();
        let merkle_paths: [MerklePath<MERKLE_TREE_DEPTH>; UTXO_AGG_LEAVES] = merkle_paths
            .try_into()
            .map_err(|v: Vec<_>| Error::VecToArrayConversion {
                expected: UTXO_AGG_LEAVES,
                actual: v.len(),
            })?;
        Ok((inserts_len, old_tree, new_tree, merkle_paths))
    }

    // pub fn get_merkle_path_for_commitment(
    //     &self,
    //     notes_tree: &MerkleTree,
    //     commitment: Base,
    // ) -> Result<Vec<Base>, Error> {
    //     let el = Element::from_base(commitment);

    //     // Sibling path
    //     let path = notes_tree.path_for(el);

    //     let path = path
    //         .siblings_deepest_first()
    //         .iter()
    //         .copied()
    //         .map(Element::to_base)
    //         .collect();

    //     Ok(path)
    // }
}

fn path_to_merkle_path(path: Path) -> MerklePath<MERKLE_TREE_DEPTH> {
    let elements = path
        .siblings_deepest_first()
        .iter()
        .cloned()
        .take(MERKLE_TREE_PATH_DEPTH)
        .collect::<Vec<Element>>();

    MerklePath::new(elements)
}

// #[cfg(test)]
// mod tests {
//     use std::str::FromStr;

//     use web3::types::Address;
//     use zk_circuits::utxo::{InputNote, Note, Utxo, UtxoKind};

//     use super::*;

//     struct Env {
//         rollup_contract_addr: Address,
//         evm_secret_key: SecretKey,
//     }

//     fn get_env() -> Env {
//         Env {
//             rollup_contract_addr: Address::from_str(
//                 &std::env::var("ROLLUP_CONTRACT_ADDR")
//                     .expect("env var ROLLUP_CONTRACT_ADDR is not set"),
//             )
//             .unwrap(),
//             evm_secret_key: SecretKey::from_str(&std::env::var("PROVER_SECRET_KEY").unwrap_or(
//                 // Seems to be the default when deploying with hardhat to a local node
//                 "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_owned(),
//             ))
//             .unwrap(),
//         }
//     }

//     fn contract(addr: Address) -> RollupContract {
//         let rpc = std::env::var("ETHEREUM_RPC").unwrap_or("http://localhost:8545".to_owned());
//         let web3 = web3::Web3::new(web3::transports::Http::new(&rpc).unwrap());
//         let contract = include_bytes!("../../../citrea/artifacts/contracts/Rollup.sol/Rollup.json");
//         let contract = serde_json::from_slice::<serde_json::Value>(contract).unwrap();
//         let contract =
//             serde_json::from_value::<ethabi::Contract>(contract.get("abi").unwrap().clone())
//                 .unwrap();
//         let contract = web3::contract::Contract::new(web3.eth(), addr, contract);

//         RollupContract::new(web3, contract)
//     }

//     #[tokio::test]
//     async fn test_prover() {
//         let env = get_env();
//         let contract = contract(env.rollup_contract_addr);

//         let notes_tree: Arc<Mutex<Tree<33>>> =
//             Arc::new(Mutex::new(Tree::<MERKLE_TREE_DEPTH>::new()));

//         let ban_tree: Arc<Mutex<Tree<MERKLE_TREE_DEPTH>>> =
//             Arc::new(Mutex::new(Tree::<MERKLE_TREE_DEPTH>::new()));

//         let root_hash = notes_tree.lock().root_hash().to_base();

//         let prover = Prover::new(contract, env.evm_secret_key, notes_tree, ban_tree);

//         let inputs = [
//             InputNote::<MERKLE_TREE_DEPTH>::padding_note(),
//             InputNote::padding_note(),
//         ];
//         let outputs = [Note::padding_note(), Note::padding_note()];
//         let utxo_proof = Utxo::new(inputs, outputs, root_hash, UtxoKind::Transfer);
//         let utxo_params = read_utxo_params();

//         // Prove
//         let snark = utxo_proof.snark(&utxo_params).unwrap();

//         prover
//             .add_tx(Transaction {
//                 proof: snark.to_witness(),
//             })
//             .unwrap();

//         prover.rollup().await.unwrap();
//     }

//     lazy_static::lazy_static! {
//         // Without this, we get "nonce too low" error when running tests in parallel:
//         // "Nonce too low. Expected nonce to be 1008 but got 1007."
//         static ref TEST_NONCE_BUG_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::new(());
//     }

//     #[tokio::test]
//     async fn add_prover() {
//         let _lock = TEST_NONCE_BUG_MUTEX.lock().await;

//         let env = get_env();
//         let contract = contract(env.rollup_contract_addr);

//         let tx = contract
//             .add_prover(&env.evm_secret_key, &Address::from_low_u64_be(0xfe))
//             .await
//             .unwrap();

//         while contract
//             .web3_client
//             .eth()
//             .transaction_receipt(tx)
//             .await
//             .unwrap()
//             .is_none()
//         {
//             tokio::time::sleep(std::time::Duration::from_secs(1)).await;
//         }
//     }
// }
