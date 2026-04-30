use atomiq::EscrowData;
use contracts::{Address, ConfirmationType, ERC20Contract, RollupContract};
use element::Element;
use eth_util::Eth;
use eyre::{Context, ContextCompat};
use primitives::{
    block_height::BlockHeight,
    pagination::{CursorChoice, CursorChoiceAfter, OpaqueCursor, OpaqueCursorChoice},
};
use reqwest::StatusCode;
use std::time::Duration;
use zk_primitives::{UtxoKind, UtxoKindMessages, UtxoProof};

use tracing::{debug, info};

const MAX_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// Call `op` up to `max_attempts` times with `delay` between attempts, returning the first
/// `Some` result. On `Ok(None)` the helper sleeps and retries; on `Err` it short-circuits.
/// Returns `Ok(None)` if all attempts are exhausted without a `Some`.
async fn retry_until_some<F, Fut, T>(
    max_attempts: u32,
    delay: Duration,
    mut op: F,
) -> Result<Option<T>, eyre::Error>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, eyre::Error>>,
{
    for attempt in 1..=max_attempts {
        if let Some(value) = op(attempt).await? {
            return Ok(Some(value));
        }
        if attempt < max_attempts {
            tokio::time::sleep(delay).await;
        }
    }
    Ok(None)
}

pub struct BurnSubstitutor {
    rollup_contract: RollupContract,
    erc20_contract: ERC20Contract,
    node_rpc_url: String,
    eth_txn_confirm_wait_interval: Duration,
    cursor: Option<OpaqueCursorChoice<ListTxnsPosition>>,
    offramp_url: String,
    substitutor_address: Address,
}

impl BurnSubstitutor {
    pub fn new(
        rollup_contract: RollupContract,
        erc20_contract: ERC20Contract,
        node_rpc_url: String,
        eth_txn_confirm_wait_interval: Duration,
        offramp_url: String,
        substitutor_address: Address,
    ) -> Self {
        BurnSubstitutor {
            rollup_contract,
            erc20_contract,
            node_rpc_url,
            eth_txn_confirm_wait_interval,
            cursor: None,
            offramp_url,
            substitutor_address,
        }
    }

    pub async fn tick(&mut self) -> Result<Vec<Element>, eyre::Error> {
        if self.cursor.is_none() {
            let last_rollup = self.fetch_last_rollup_block().await?;

            info!("Last rollup height: {}", last_rollup);

            self.cursor = Some(
                CursorChoice::After(CursorChoiceAfter::After(ListTxnsPosition {
                    block: last_rollup,
                    txn: u64::MAX,
                }))
                .opaque(),
            );
        }

        let (txns, cursor) = Self::fetch_transactions(
            &reqwest::Client::new(),
            &self.node_rpc_url,
            None,
            self.cursor.as_ref(),
            false,
        )
        .await
        .context("Failed to fetch transactions")?;

        info!("Fetched transactions");

        let mut substituted_burns = Vec::new();
        let mut other_burns = Vec::new();

        for txn in &txns {
            if let UtxoKindMessages::Burn(burn_msgs) = txn.proof.kind_messages() {
                let hash = burn_msgs.burn_hash;
                let burn_address =
                    Address::from_slice(&burn_msgs.burn_address.to_be_bytes()[12..32]);
                let amount = burn_msgs.value;
                let note_kind = burn_msgs.note_kind;

                if UtxoKind::from(burn_msgs.utxo_kind) == UtxoKind::Burn {
                    if self
                        .rollup_contract
                        .was_burn_substituted(
                            &burn_address,
                            &note_kind,
                            &hash,
                            &amount,
                            txn.block_height.0,
                        )
                        .await?
                    {
                        info!("Skipping substituted Burn with hash {:2x}", hash);
                        continue;
                    }

                    // Calculate the burn value as an EVM U256
                    let burn_value = burn_msgs.value.to_eth_u256();

                    // Check ERC20 balance and optionally skip if burn exceeds available balance
                    let token_balance = self
                        .erc20_contract
                        .balance(self.rollup_contract.signer_address)
                        .await
                        .context("Failed to fetch ERC20 balance for burn substitution")?;

                    if burn_value > token_balance {
                        info!(
                            ?txn.proof.public_inputs,
                            %burn_value,
                            %token_balance,
                            "Skipping burn: value exceeds substitutor balance"
                        );
                        continue;
                    }

                    let txn = self
                        .rollup_contract
                        .substitute_burn(
                            &burn_address,
                            &note_kind,
                            &hash,
                            &amount,
                            txn.block_height.0,
                        )
                        .await
                        .context("Failed to substitute burn")?;

                    info!("Substitution transaction {:x} has been sent", txn);

                    self.rollup_contract
                        .client
                        .wait_for_confirm(
                            txn,
                            self.eth_txn_confirm_wait_interval,
                            ConfirmationType::Latest,
                        )
                        .await
                        .context("Failed to wait for burn substitution")?;

                    substituted_burns.push(hash);
                } else {
                    info!("Transaction of NoSubstitution kind");
                    self.handle_nosub_burn(amount, hash).await?;
                    other_burns.push(hash);
                }
            }
        }

        if !txns.is_empty() {
            self.cursor = cursor
                .after
                .map(|after| CursorChoice::After(after.0).opaque());
        }

        Ok(substituted_burns)
    }

    async fn handle_nosub_burn(
        &mut self,
        amount: Element,
        burn_hash: Element,
    ) -> Result<(), eyre::Error> {
        let burn_value = amount.to_eth_u256();
        let burner_addr = format!("{:#x}", self.substitutor_address);

        // Step A — Query /swaps (with retry)
        let client = reqwest::Client::new();
        let swaps_url = format!("{}/swaps", self.offramp_url);

        info!(
            "looking into swaps for burner {:x}",
            self.substitutor_address
        );

        let swap = retry_until_some(MAX_ATTEMPTS, RETRY_DELAY, |attempt| {
            let client = client.clone();
            let swaps_url = swaps_url.clone();
            let burner_addr = burner_addr.clone();
            async move {
                let resp = client
                    .get(&swaps_url)
                    .query(&[
                        ("amount", burn_value.to_string()),
                        ("address", burner_addr.clone()),
                    ])
                    .send()
                    .await
                    .context("Failed to query /swaps")?;

                match resp.status() {
                    StatusCode::OK => {}
                    e => return Err(eyre::eyre!("/swaps returned unexpected status: {e}")),
                }

                let swaps_resp = resp
                    .json::<SwapsResponse>()
                    .await
                    .context("Failed to parse /swaps response")?;

                // Step B — Find a matching swap in a state the burner can still act on.
                let swap = swaps_resp.swaps.into_iter().find(|s| {
                    let addr_match = s.input_address.eq_ignore_ascii_case(&burner_addr);

                    let amount_match = web3::types::U256::from_dec_str(&s.amount)
                        .map(|a| a == burn_value)
                        .unwrap_or(false);

                    let state_match = matches!(
                        s.state,
                        0 | -1 // 0 - CREATED in all types of swaps, -1 - QUOTE_SOFT_EXPIRED
                    );
                    debug!(
                        "Received swap {:?} {:?} - {:?}",
                        addr_match, amount_match, state_match
                    );
                    addr_match && amount_match && state_match
                });

                if swap.is_none() {
                    info!(
                        ?burn_hash,
                        %burn_value,
                        %burner_addr,
                        attempt,
                        "No matching swap found; retrying in {}s",
                        RETRY_DELAY.as_secs()
                    );
                }
                Ok(swap)
            }
        })
        .await?;

        let Some(swap) = swap else {
            info!(
                ?burn_hash,
                "No matching swap after retries; will retry on next tick"
            );
            return Ok(());
        };

        let swap_id = swap.id.clone();
        info!(?burn_hash, %swap_id, "Matched NoSub burn to swap");

        // Step C — Query /offramp/:swapId (with retry)
        // We can't rely on swap status since it may soft-expire before burner picks it;
        // retry while commitTxs is empty or the server returns a transient 5xx.
        let offramp_url = format!("{}/offramp/{}", self.offramp_url, swap_id);

        let offramp_resp = retry_until_some(MAX_ATTEMPTS, RETRY_DELAY, |attempt| {
            let client = client.clone();
            let offramp_url = offramp_url.clone();
            let swap_id = swap_id.clone();
            async move {
                let resp = client
                    .get(&offramp_url)
                    .send()
                    .await
                    .context("Failed to query /offramp/:swapId")?;

                match resp.status() {
                    StatusCode::OK => {}
                    e if e.is_server_error() => {
                        tracing::warn!(
                            ?burn_hash,
                            %swap_id,
                            status = %e,
                            attempt,
                            "Transient server error from /offramp/:id; retrying in {}s",
                            RETRY_DELAY.as_secs()
                        );
                        return Ok(None);
                    }
                    e => return Err(eyre::eyre!("/offramp/:id returned unexpected status: {e}")),
                }

                let offramp_resp = resp
                    .json::<OfframpResponse>()
                    .await
                    .context("Failed to parse /offramp response")?;

                // Step D — Accept either a commit-ready state with commitTxs, or a
                // claimed state with a revealed preimage. Otherwise retry.
                let commit_ready = matches!(
                    offramp_resp.state,
                    0 | -1 // -1 QUOTE_SOFT_EXPIRED to BTC swaps
                ) && !offramp_resp.commit_txs.is_empty();

                let claimed_with_preimage =
                    matches!(offramp_resp.state, 2 | 3) && offramp_resp.preimage.is_some();

                if !commit_ready && !claimed_with_preimage {
                    info!(
                        ?burn_hash,
                        %swap_id,
                        state = %offramp_resp.state,
                        attempt,
                        "Swap not ready (no commitTxs or preimage); retrying in {}s",
                        RETRY_DELAY.as_secs()
                    );
                    return Ok(None);
                }

                Ok(Some(offramp_resp))
            }
        })
        .await?;

        let Some(offramp_resp) = offramp_resp else {
            info!(
                ?burn_hash,
                %swap_id,
                "Swap did not reach a ready state after retries; skipping"
            );
            return Ok(());
        };

        if matches!(offramp_resp.state, 2 | 3) {
            if let Some(preimage) = offramp_resp.preimage.as_deref() {
                info!(
                    ?burn_hash,
                    %swap_id,
                    state = %offramp_resp.state,
                    %preimage,
                    "Swap already claimed; preimage revealed"
                );
            }
            return Ok(());
        }

        info!(%offramp_resp.state, %swap_id, "Proceeding to commitment step");

        let web3_client = self.rollup_contract.client.client().clone();

        for commit_tx in &offramp_resp.commit_txs {
            let to: Address = commit_tx
                .to
                .parse()
                .context("Failed to parse commitTx.to")?;

            let data_bytes = hex::decode(commit_tx.data.trim_start_matches("0x"))
                .context("Failed to decode commitTx.data")?;

            let Ok(escrow) = EscrowData::from_transaction_calldata(&data_bytes) else {
                tracing::warn!("Failed to decode EscrowData from commitTx.data; skipping");
                return Ok(());
            };

            if escrow.offerer != self.substitutor_address {
                tracing::warn!(
                    ?escrow.offerer,
                    "Swap was created for a different address; skipping"
                );
                return Ok(());
            }

            if escrow.amount != burn_value {
                tracing::warn!(
                    ?escrow.amount,
                    "Swap was created for a different amount; skipping"
                );
                return Ok(());
            }

            let data = web3::types::Bytes(data_bytes);

            let value = web3::types::U256::from_dec_str(&commit_tx.value)
                .context("Failed to parse commitTx.value")?;

            let gas = commit_tx
                .gas_limit
                .as_deref()
                .map(web3::types::U256::from_dec_str)
                .transpose()
                .context("Failed to parse commitTx.gasLimit")?
                .unwrap_or_else(|| web3::types::U256::from(1_000_000u64));

            let max_fee_per_gas = commit_tx
                .max_fee_per_gas
                .as_deref()
                .map(web3::types::U256::from_dec_str)
                .transpose()
                .context("Failed to parse commitTx.maxFeePerGas")?;

            let max_priority_fee_per_gas = commit_tx
                .max_priority_fee_per_gas
                .as_deref()
                .map(web3::types::U256::from_dec_str)
                .transpose()
                .context("Failed to parse commitTx.maxPriorityFeePerGas")?;

            let tx_params = web3::types::TransactionParameters {
                nonce: commit_tx.nonce.map(web3::types::U256::from),
                to: Some(to),
                gas,
                gas_price: None,
                value,
                data,
                chain_id: None, // fetched automatically by sign_transaction
                transaction_type: Some(web3::types::U64::from(2u64)),
                access_list: None,
                max_fee_per_gas,
                max_priority_fee_per_gas,
            };

            let signed = web3_client
                .accounts()
                .sign_transaction(tx_params, &self.rollup_contract.signer)
                .await
                .map_err(|e| eyre::eyre!("Failed to sign commitTx: {e}"))?;

            let tx_hash = web3_client
                .eth()
                .send_raw_transaction(signed.raw_transaction)
                .await
                .map_err(|e| eyre::eyre!("Failed to send commitTx: {e}"))?;

            info!(?burn_hash, %swap_id, "Sent commitTx {:x}", tx_hash);

            self.rollup_contract
                .client
                .wait_for_confirm(
                    tx_hash,
                    self.eth_txn_confirm_wait_interval,
                    ConfirmationType::Latest,
                )
                .await
                .context("Failed to wait for commitTx confirmation")?;
        }

        Ok(())
    }

    async fn fetch_last_rollup_block(&mut self) -> Result<BlockHeight, contracts::Error> {
        self.rollup_contract.block_height().await.map(BlockHeight)
    }

    async fn fetch_transactions(
        client: &reqwest::Client,
        network_base_url: &str,
        limit: Option<usize>,
        cursor: Option<&OpaqueCursorChoice<ListTxnsPosition>>,
        poll: bool,
    ) -> Result<(Vec<Transaction>, OpaqueCursor<ListTxnsPosition>), eyre::Error> {
        let req = client
            .get(format!("{network_base_url}/v0/transactions"))
            .query(&[
                ("limit", limit.map(|l| l.to_string())),
                ("order", Some("OldestToNewest".to_owned())),
                ("cursor", cursor.map(|c| c.serialize()).transpose()?),
                ("poll", Some(poll.to_string())),
            ]);

        let resp = req.send().await?;

        match resp.status() {
            StatusCode::OK => {}
            e => return Err(eyre::eyre!("Unexpected status code: {e}")),
        }

        let mut resp = resp.json::<serde_json::Value>().await?;

        let txns = serde_json::from_value::<Vec<Transaction>>(
            resp.get_mut("txns").context("Missing txns field")?.take(),
        )?;

        let cursor = resp
            .get_mut("cursor")
            .context("Missing pagination field")?
            .take();

        let cursor = serde_json::from_value(cursor).context("Failed to parse cursor")?;

        Ok((txns, cursor))
    }
}

#[derive(Debug, serde::Deserialize)]
struct Transaction {
    pub proof: UtxoProof,
    pub block_height: BlockHeight,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ListTxnsPosition {
    block: BlockHeight,
    txn: u64,
}

#[derive(Debug, serde::Deserialize)]
struct SwapEntry {
    id: String,
    state: i32,
    #[serde(rename = "inputAddress")]
    input_address: String,
    #[allow(dead_code)]
    #[serde(rename = "outputAddress")]
    output_address: String,
    amount: String,
}

#[derive(Debug, serde::Deserialize)]
struct SwapsResponse {
    swaps: Vec<SwapEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct CommitTx {
    to: String,
    data: String,
    value: String,
    #[serde(rename = "gasLimit")]
    gas_limit: Option<String>,
    #[serde(rename = "maxFeePerGas")]
    max_fee_per_gas: Option<String>,
    #[serde(rename = "maxPriorityFeePerGas")]
    max_priority_fee_per_gas: Option<String>,
    nonce: Option<u64>,
}

fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(<Option<Vec<T>> as serde::Deserialize>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, serde::Deserialize)]
struct OfframpResponse {
    state: i32,
    #[serde(rename = "commitTxs", default, deserialize_with = "null_as_empty")]
    commit_txs: Vec<CommitTx>,
    #[serde(default)]
    preimage: Option<String>,
}
