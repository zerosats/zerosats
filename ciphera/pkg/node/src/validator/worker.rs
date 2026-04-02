use std::sync::Arc;
use std::time::Duration;

use crate::{NodeShared, config::Config, types::BlockHeight};
use contracts::RollupContract;
use tracing::debug;

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

    run_validator_worker(config, node, contract).await
}

async fn run_validator_worker(
    config: &Config,
    node: Arc<NodeShared>,
    contract: RollupContract,
) -> Result<()> {
    let initial_contract_block_height = BlockHeight(contract.block_height().await?);
    debug!(initial_contract_block_height =? initial_contract_block_height,"Rollup contract height");
    let block_delta = config.min_block_duration as u64;
    // Wait for node to notice it's out of sync
    tokio::time::sleep(Duration::from_secs(2*block_delta)).await;
    // Wait for node to sync.
    // Trying to sync without waiting would mean invalid prover smirk tree,
    // if the node synced with fast sync.
    while node.is_out_of_sync() {
        tokio::time::sleep(Duration::from_secs(3*block_delta)).await;
    }

    let last_confirmed = *node.block_cache.lock().hash();
    debug!(last_confirmed =? last_confirmed,"Last confirmed block observed");
    unimplemented!("validator worker: watch committed blocks and call contract.set_checkpoint when queues are empty")
}
