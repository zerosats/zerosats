use std::sync::Arc;
use std::time::Duration;

use crate::{NodeShared, config::Config, types::BlockHeight};
use contracts::RollupContract;
use element::Element;
use futures::StreamExt;
use tracing::{debug, info};

use tokio::sync::Notify;

use super::error::Result;

/// Entry point launched by `bin/node.rs` when the node runs in `Validator` mode.
pub async fn run_validator(config: &Config, node: Arc<NodeShared>) -> Result<()> {
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

    run_validator_worker(config, node, contract, block_notifier).await
}

async fn run_validator_worker(
    config: &Config,
    node: Arc<NodeShared>,
    contract: RollupContract,
    _block_notifier: Arc<Notify>,
) -> Result<()> {
    let initial_contract_block_height = BlockHeight(contract.block_height().await?);
    debug!(initial_contract_block_height =? initial_contract_block_height, "Rollup contract height");
    let block_delta = config.min_block_duration as u64;
    // Wait for node to notice it's out of sync
    tokio::time::sleep(Duration::from_secs(2 * block_delta)).await;
    // Wait for node to sync.
    // Trying to sync without waiting would mean invalid prover smirk tree,
    // if the node synced with fast sync.
    while node.is_out_of_sync() {
        tokio::time::sleep(Duration::from_secs(3 * block_delta)).await;
    }

    let last_confirmed = *node.block_cache.lock().hash();
    debug!(last_confirmed =? last_confirmed, "Last confirmed block observed");

    let mut stream = node
        .commit_stream(Some(initial_contract_block_height.next()))
        .await;

    while let Some(commit) = stream.next().await {
        let commit = commit?;
        let commit_height = commit.content.header.height;
        let commit_root_hash = commit.content.state.root_hash;

        let contract_root_hash = Element::from_be_bytes(contract.root_hash().await?.0);

        if commit_root_hash == contract_root_hash {
            debug!(?commit_height, "Contract root already matches block, skipping");
            continue;
        }

        // The contract must be exactly at n-2 for us to checkpoint block n
        if commit_height.0 < 2 {
            continue;
        }
        let n_minus_2_height = BlockHeight(commit_height.0 - 2);
        let Some(n_minus_2_block) = node.get_block(n_minus_2_height)? else {
            debug!(?n_minus_2_height, "Block n-2 not found in store, skipping");
            continue;
        };
        let n_minus_2_root_hash = n_minus_2_block.into_block().content.state.root_hash;

        if n_minus_2_root_hash != contract_root_hash {
            debug!(
                ?n_minus_2_height,
                ?n_minus_2_root_hash,
                ?contract_root_hash,
                "Block n-2 root hash does not match contract root hash, skipping"
            );
            continue;
        }

        let other_hash = *commit.content.header_hash().inner();
        let signatures: Vec<&[u8]> = commit
            .content
            .header
            .approvals
            .iter()
            .map(|sig| sig.inner())
            .collect();

        info!(
            ?commit_height,
            ?commit_root_hash,
            ?contract_root_hash,
            "Setting checkpoint on contract"
        );

        match contract
            .set_checkpoint(&commit_root_hash, commit_height.0, other_hash, &signatures)
            .await
        {
            Ok(tx) => info!(?tx, ?commit_height, "Checkpoint set successfully"),
            Err(err) => tracing::error!(?err, ?commit_height, "Failed to set checkpoint"),
        }
    }

    unreachable!()
}
