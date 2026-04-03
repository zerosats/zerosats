use std::sync::Arc;
use std::time::Duration;

use crate::{config::Config, types::BlockHeight, NodeShared};
use contracts::RollupContract;
use element::Element;
use futures::StreamExt;
use tracing::info;

use tokio::sync::Notify;

use super::error::Result;

/// Entry point launched by `bin/node.rs` when the node runs in `Validator` mode.
pub async fn run_validator(config: &Config, node: Arc<NodeShared>) -> Result<()> {
    info!("Starting validator worker for setting up checkpoints");
    let secret_key =
        web3::signing::SecretKey::from_slice(&config.secret_key.secret_key().secret_bytes()[..])
            .unwrap();

    let contracts_client =
        contracts::Client::new(&config.evm_rpc_url, config.minimum_gas_price_gwei);

    let contract = RollupContract::load(
        contracts_client,
        &config.chain_id,
        &config.rollup_contract_addr,
        secret_key,
    )
    .await?;

    let block_notifier = Arc::new(Notify::new());

    tokio::try_join!(
        run_validator_worker(config, node, contract, block_notifier)
    )?;

    Ok(())
}

async fn run_validator_worker(
    config: &Config,
    node: Arc<NodeShared>,
    contract: RollupContract,
    _block_notifier: Arc<Notify>,
) -> Result<()> {
    let initial_contract_block_height = BlockHeight(contract.block_height().await?);
    info!(initial_contract_block_height =? initial_contract_block_height, "Rollup contract height");
    let block_delta = config.min_block_duration as u64;
    // Wait for node to notice it's out of sync
    tokio::time::sleep(Duration::from_millis(2 * block_delta)).await;
    // Wait for node to sync.
    // Trying to sync without waiting would mean invalid prover smirk tree,
    // if the node synced with fast sync.
    while node.is_out_of_sync() {
        tokio::time::sleep(Duration::from_millis(3 * block_delta)).await;
    }

    let last_confirmed = *node.block_cache.lock().hash();
    info!(last_confirmed =? last_confirmed, "Last confirmed block observed");

    let mut stream = node
        .commit_stream(Some(initial_contract_block_height.next()))
        .await;

    while let Some(commit) = stream.next().await {
        let commit = commit?;
        let commit_height = commit.content.header.height;

        // Block N's approvals endorse block N as target, signing
        // keccak256(N || block_hash(N-1)), which matches the contract's
        // acceptMsg = keccak256(height+1, proposalHash) for height = N-1.
        // Only finalized blocks (N-1 and older) ever enter the contract.
        if commit_height.0 < 1 {
            continue;
        }

        let contract_root_hash = Element::from_be_bytes(contract.root_hash().await?.0);

        let prev_height = BlockHeight(commit_height.0 - 1);
        let Some(prev_block) = node.get_block(prev_height)? else {
            info!(?prev_height, "Prev block not found in store, skipping");
            continue;
        };
        let prev_block = prev_block.into_block();
        let prev_root_hash = prev_block.content.state.root_hash;
        let prev_other_hash = *prev_block.content.header_hash().inner();

        if prev_root_hash == contract_root_hash {
            info!(?commit_height, "Contract root already matches block, skipping");
            continue;
        }

        let signatures = commit.content.header.approvals.clone();

        info!(
            ?prev_height,
            ?prev_root_hash,
            ?contract_root_hash,
            "Setting checkpoint on contract"
        );

        match contract
            .set_checkpoint(&prev_root_hash, prev_height.0, prev_other_hash, &signatures.iter().map(|s| &s.0[..]).collect::<Vec<_>>())
            .await
        {
            Ok(tx) => info!(?tx, ?prev_height, "Checkpoint set successfully"),
            Err(err) => tracing::error!(?err, ?prev_height, "Failed to set checkpoint"),
        }
    }

    unreachable!()
}
