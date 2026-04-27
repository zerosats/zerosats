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
use ethereum_types::{H256, U256};

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

        info!("looking into swaps for burner {:x}", self.substitutor_address);

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
                    debug!("Received swap {:?} {:?} - {:?}", addr_match, amount_match, state_match);
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

                let claimed_with_preimage = matches!(
                    offramp_resp.state,
                    2 | 3
                ) && offramp_resp.preimage.is_some();

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
            let to: Address = commit_tx.to.parse().context("Failed to parse commitTx.to")?;

            let data_bytes =
                hex::decode(commit_tx.data.trim_start_matches("0x"))
                    .context("Failed to decode commitTx.data")?;

            let Ok(escrow) = EscrowData::from_transaction_calldata(&data_bytes) else {
                tracing::warn!(
                    "Failed to decode EscrowData from commitTx.data; skipping"
                );
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
                .map(|s| web3::types::U256::from_dec_str(s))
                .transpose()
                .context("Failed to parse commitTx.gasLimit")?
                .unwrap_or_else(|| web3::types::U256::from(1_000_000u64));

            let max_fee_per_gas = commit_tx
                .max_fee_per_gas
                .as_deref()
                .map(|s| web3::types::U256::from_dec_str(s))
                .transpose()
                .context("Failed to parse commitTx.maxFeePerGas")?;

            let max_priority_fee_per_gas = commit_tx
                .max_priority_fee_per_gas
                .as_deref()
                .map(|s| web3::types::U256::from_dec_str(s))
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

/// Flags used in EscrowData
const FLAG_PAY_OUT: u64 = 0x01;
const FLAG_PAY_IN: u64 = 0x02;
const FLAG_REPUTATION: u64 = 0x04;

/// Represents the decoded flags from the uint256 flags field
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flags {
    pub pay_out: bool,
    pub pay_in: bool,
    pub reputation: bool,
    /// Upper 64 bits used as sequence number
    pub sequence: u64,
}

impl Flags {
    /// Decode flags from uint256
    /// Sequence is in upper 64 bits (after right shift by 64)
    pub fn from_u256(value: U256) -> Self {
        // Extract upper 64 bits (bits 64-127)
        let sequence = (value >> 64u32).as_u64();

        // Extract lower 64 bits by masking (bits 0-63)
        let mask = U256::from(0xFFFFFFFFFFFFFFFFu64);
        let lower_bits = (value & mask).as_u64();

        Flags {
            sequence,
            pay_out: (lower_bits & FLAG_PAY_OUT) == FLAG_PAY_OUT,
            pay_in: (lower_bits & FLAG_PAY_IN) == FLAG_PAY_IN,
            reputation: (lower_bits & FLAG_REPUTATION) == FLAG_REPUTATION,
        }
    }
    /// Encode flags to U256
    /// Matches TypeScript: (sequence << 64n) | flags
    pub fn to_u256(&self) -> U256 {
        let sequence_bits = U256::from(self.sequence) << 64u32;
        let flag_bits = U256::from(
            (if self.pay_out { FLAG_PAY_OUT } else { 0 })
                | (if self.pay_in { FLAG_PAY_IN } else { 0 })
                | (if self.reputation { FLAG_REPUTATION } else { 0 })
        );
        sequence_bits | flag_bits
    }
}

/// EscrowData structure matching the Solidity/TypeScript definition
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowData {
    /// Account funding the escrow
    pub offerer: Address,
    /// Account entitled to claim the funds from the escrow
    pub claimer: Address,

    /// Amount of tokens in the escrow
    pub amount: U256,
    /// Token of the escrow
    pub token: Address,

    /// Misc escrow data flags (payIn, payOut, reputation)
    pub flags: Flags,

    /// Address of the IClaimHandler
    pub claim_handler: Address,
    /// Data provided to the claim handler
    pub claim_data: [u8; 32],

    /// Address of the IRefundHandler
    pub refund_handler: Address,
    /// Data provided to the refund handler
    pub refund_data: [u8; 32],

    /// Security deposit
    pub security_deposit: U256,
    /// Claimer bounty
    pub claimer_bounty: U256,
    /// Deposit token
    pub deposit_token: Address,

    /// ExecutionAction hash commitment
    pub success_action_commitment: [u8; 32],
}

impl EscrowData {
    /// Create a new EscrowData instance
    pub fn new(
        offerer: Address,
        claimer: Address,
        amount: U256,
        token: Address,
        pay_out: bool,
        pay_in: bool,
        reputation: bool,
        sequence: u64,
        claim_handler: Address,
        claim_data: [u8; 32],
        refund_handler: Address,
        refund_data: [u8; 32],
        security_deposit: U256,
        claimer_bounty: U256,
        deposit_token: Address,
        success_action_commitment: [u8; 32],
    ) -> Self {
        EscrowData {
            offerer,
            claimer,
            amount,
            token,
            flags: Flags {
                pay_out,
                pay_in,
                reputation,
                sequence,
            },
            claim_handler,
            claim_data,
            refund_handler,
            refund_data,
            security_deposit,
            claimer_bounty,
            deposit_token,
            success_action_commitment,
        }
    }

    /// Deserialize from ABI-encoded bytes (as from raw EVM transaction)
    ///
    /// This matches the Solidity struct encoding:
    /// ```solidity
    /// struct EscrowData {
    ///     address offerer;           // 0x0-0x14  (20 bytes, padded to 32)
    ///     address claimer;           // 0x20-0x34 (20 bytes, padded to 32)
    ///     uint256 amount;            // 0x40-0x5F (32 bytes)
    ///     address token;             // 0x60-0x74 (20 bytes, padded to 32)
    ///     uint256 flags;             // 0x80-0x9F (32 bytes)
    ///     address claimHandler;      // 0xA0-0xB4 (20 bytes, padded to 32)
    ///     bytes32 claimData;         // 0xC0-0xDF (32 bytes)
    ///     address refundHandler;     // 0xE0-0xF4 (20 bytes, padded to 32)
    ///     bytes32 refundData;        // 0x100-0x11F (32 bytes)
    ///     uint256 securityDeposit;   // 0x120-0x13F (32 bytes)
    ///     uint256 claimerBounty;     // 0x140-0x15F (32 bytes)
    ///     address depositToken;      // 0x160-0x174 (20 bytes, padded to 32)
    ///     bytes32 successActionCommitment; // 0x180-0x19F (32 bytes)
    /// }
    /// ```
    pub fn from_abi_encoded(data: &[u8]) -> Result<Self, String> {
        // Total expected size: 13 fields * 32 bytes = 416 bytes
        if data.len() < 416 {
            return Err(format!(
                "Insufficient data for EscrowData: expected at least 416 bytes, got {}",
                data.len()
            ));
        }

        let mut offset = 0;

        // offerer (address, padded to 32 bytes)
        let offerer = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // claimer (address, padded to 32 bytes)
        let claimer = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // amount (uint256)
        let amount = Self::decode_u256(&data[offset..offset + 32])?;
        offset += 32;

        // token (address, padded to 32 bytes)
        let token = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // flags (uint256)
        let flags_raw = Self::decode_u256(&data[offset..offset + 32])?;
        let flags = Flags::from_u256(flags_raw);
        offset += 32;

        // claimHandler (address, padded to 32 bytes)
        let claim_handler = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // claimData (bytes32)
        let claim_data = Self::decode_bytes32(&data[offset..offset + 32])?;
        offset += 32;

        // refundHandler (address, padded to 32 bytes)
        let refund_handler = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // refundData (bytes32)
        let refund_data = Self::decode_bytes32(&data[offset..offset + 32])?;
        offset += 32;

        // securityDeposit (uint256)
        let security_deposit = Self::decode_u256(&data[offset..offset + 32])?;
        offset += 32;

        // claimerBounty (uint256)
        let claimer_bounty = Self::decode_u256(&data[offset..offset + 32])?;
        offset += 32;

        // depositToken (address, padded to 32 bytes)
        let deposit_token = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // successActionCommitment (bytes32)
        let success_action_commitment = Self::decode_bytes32(&data[offset..offset + 32])?;

        Ok(EscrowData {
            offerer,
            claimer,
            amount,
            token,
            flags,
            claim_handler,
            claim_data,
            refund_handler,
            refund_data,
            security_deposit,
            claimer_bounty,
            deposit_token,
            success_action_commitment,
        })
    }

    /// Deserialize from transaction calldata
    /// Assumes the data starts after the function selector (first 4 bytes)
    pub fn from_transaction_calldata(calldata: &[u8]) -> Result<Self, String> {
        // Skip function selector if present (4 bytes)
        let data = if calldata.len() > 4 && calldata.len() % 32 == 4 {
            &calldata[4..]
        } else {
            calldata
        };

        Self::from_abi_encoded(data)
    }

    /// Serialize to ABI-encoded bytes
    pub fn to_abi_encoded(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(416);

        // offerer
        result.extend_from_slice(&Self::encode_address(self.offerer));

        // claimer
        result.extend_from_slice(&Self::encode_address(self.claimer));

        // amount
        result.extend_from_slice(&Self::encode_u256(self.amount));

        // token
        result.extend_from_slice(&Self::encode_address(self.token));

        // flags
        result.extend_from_slice(&Self::encode_u256(self.flags.to_u256()));

        // claimHandler
        result.extend_from_slice(&Self::encode_address(self.claim_handler));

        // claimData
        result.extend_from_slice(&self.claim_data);

        // refundHandler
        result.extend_from_slice(&Self::encode_address(self.refund_handler));

        // refundData
        result.extend_from_slice(&self.refund_data);

        // securityDeposit
        result.extend_from_slice(&Self::encode_u256(self.security_deposit));

        // claimerBounty
        result.extend_from_slice(&Self::encode_u256(self.claimer_bounty));

        // depositToken
        result.extend_from_slice(&Self::encode_address(self.deposit_token));

        // successActionCommitment
        result.extend_from_slice(&self.success_action_commitment);

        result
    }

    /// Calculate the escrow hash (keccak256 of ABI-encoded data)
    pub fn escrow_hash(&self) -> H256 {
        use web3::signing::keccak256;
        H256::from(keccak256(&self.to_abi_encoded()))
    }

    /// Get the claim data as H256
    pub fn claim_data_hash(&self) -> H256 {
        H256::from(self.claim_data)
    }

    /// Get the refund data as H256
    pub fn refund_data_hash(&self) -> H256 {
        H256::from(self.refund_data)
    }

    /// Check if success action is set (non-zero)
    pub fn has_success_action(&self) -> bool {
        self.success_action_commitment != [0u8; 32]
    }

    /// Helper: Decode address from 32-byte padded value
    fn decode_address(data: &[u8]) -> Result<Address, String> {
        if data.len() != 32 {
            return Err("Address data must be 32 bytes".to_string());
        }
        // Address is in the last 20 bytes (right-padded)
        let mut addr_bytes = [0u8; 20];
        addr_bytes.copy_from_slice(&data[12..32]);
        Ok(Address::from(addr_bytes))
    }

    /// Helper: Decode U256 from 32-byte value
    fn decode_u256(data: &[u8]) -> Result<U256, String> {
        if data.len() != 32 {
            return Err("U256 data must be 32 bytes".to_string());
        }
        Ok(U256::from_big_endian(data))
    }

    /// Helper: Decode bytes32
    fn decode_bytes32(data: &[u8]) -> Result<[u8; 32], String> {
        if data.len() != 32 {
            return Err("bytes32 data must be 32 bytes".to_string());
        }
        let mut result = [0u8; 32];
        result.copy_from_slice(data);
        Ok(result)
    }

    /// Helper: Encode address to 32-byte padded value
    fn encode_address(addr: Address) -> [u8; 32] {
        let mut result = [0u8; 32];
        // Address goes in the last 20 bytes (right-aligned)
        result[12..32].copy_from_slice(addr.as_bytes());
        result
    }

    /// Helper: Encode U256 to 32-byte big-endian value
    fn encode_u256(value: U256) -> [u8; 32] {
        let mut result = [0u8; 32];
        value.to_big_endian(&mut result);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn test_flags_encoding_decoding() {
        let flags = Flags {
            pay_out: true,
            pay_in: false,
            reputation: true,
            sequence: 0x1234,
        };

        let encoded = flags.to_u256();
        let decoded = Flags::from_u256(encoded);

        assert_eq!(flags, decoded);
    }

    #[test]
    fn test_escrow_data_creation() {
        let escrow = EscrowData::new(
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            "0x2222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            U256::from(1_000_000u64),
            "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            true,
            false,
            true,
            42,
            "0x4444444444444444444444444444444444444444"
                .parse()
                .unwrap(),
            [5u8; 32],
            "0x6666666666666666666666666666666666666666"
                .parse()
                .unwrap(),
            [7u8; 32],
            U256::from(50_000u64),
            U256::from(10_000u64),
            "0x8888888888888888888888888888888888888888"
                .parse()
                .unwrap(),
            [9u8; 32],
        );

        assert_eq!(escrow.amount, U256::from(1_000_000u64));
        assert_eq!(escrow.flags.pay_out, true);
        assert_eq!(escrow.flags.pay_in, false);
        assert_eq!(escrow.flags.reputation, true);
        assert_eq!(escrow.flags.sequence, 42);
    }

    #[test]
    fn test_escrow_data_roundtrip() {
        let original = EscrowData::new(
            "0x1234567890123456789012345678901234567890"
                .parse()
                .unwrap(),
            "0x0987654321098765432109876543210987654321"
                .parse()
                .unwrap(),
            U256::from(5_000_000u64),
            "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
                .parse()
                .unwrap(),
            true,
            true,
            false,
            999,
            "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
                .parse()
                .unwrap(),
            [1u8; 32],
            "0xaabbccddaabbccddaabbccddaabbccddaabbccdd"
                .parse()
                .unwrap(),
            [2u8; 32],
            U256::from(100_000u64),
            U256::from(25_000u64),
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                .parse()
                .unwrap(),
            [3u8; 32],
        );

        let encoded = original.to_abi_encoded();
        assert_eq!(encoded.len(), 416);

        let decoded = EscrowData::from_abi_encoded(&encoded).expect("Failed to decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_escrow_hash_consistency() {
        let escrow = EscrowData::new(
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            "0x2222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            U256::from(1_000_000u64),
            "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            true,
            false,
            true,
            42,
            "0x4444444444444444444444444444444444444444"
                .parse()
                .unwrap(),
            [5u8; 32],
            "0x6666666666666666666666666666666666666666"
                .parse()
                .unwrap(),
            [7u8; 32],
            U256::from(50_000u64),
            U256::from(10_000u64),
            "0x8888888888888888888888888888888888888888"
                .parse()
                .unwrap(),
            [9u8; 32],
        );

        let hash1 = escrow.escrow_hash();
        let hash2 = escrow.escrow_hash();

        assert_eq!(hash1, hash2, "Hashes should be deterministic");
    }

    #[test]
    fn test_success_action_commitment() {
        let mut escrow = EscrowData::new(
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            "0x2222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            U256::from(1_000_000u64),
            "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            true,
            false,
            true,
            42,
            "0x4444444444444444444444444444444444444444"
                .parse()
                .unwrap(),
            [5u8; 32],
            "0x6666666666666666666666666666666666666666"
                .parse()
                .unwrap(),
            [7u8; 32],
            U256::from(50_000u64),
            U256::from(10_000u64),
            "0x8888888888888888888888888888888888888888"
                .parse()
                .unwrap(),
            [0u8; 32], // Zero = no success action
        );

        assert!(!escrow.has_success_action());

        escrow.success_action_commitment = [1u8; 32];
        assert!(escrow.has_success_action());
    }

    #[test]
    fn test_from_abi_encoded_minimal() {
        // Create minimal valid data (416 bytes)
        let mut data = vec![0u8; 416];

        // Set offerer at offset 12-32
        let offerer_bytes = hex::decode("1111111111111111111111111111111111111111").unwrap();
        data[12..32].copy_from_slice(&offerer_bytes);

        // Set claimer at offset 44-64 (offset 32 + 12)
        let claimer_bytes = hex::decode("2222222222222222222222222222222222222222").unwrap();
        data[44..64].copy_from_slice(&claimer_bytes);

        let result = EscrowData::from_abi_encoded(&data);
        assert!(result.is_ok());

        let escrow = result.unwrap();
        assert_eq!(
            escrow.offerer,
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap()
        );
    }

    #[test]
    fn test_from_transaction_calldata() {
        // bolt11
        // lntb21u1p57w7ujpp5w9vzr5f5rg426l6pmtumg5kmahw53u985pd4a38k9dn6ycy3hz3sdq2f38xy6t5wvcqzzsxqrrsssp5c5gmwupuu4vhh7gmvlxh32gw6n7a6wf9s9hzx59nrw96sjnf3vas9qxpqysgqyxx5qhuwywdul8z8dkum3qgy6l5rqfanqcpvzwcwek2va2x3c5ej83v2k526g5p4upqztr4gnkvzkaheecvj3u42lfm26ylg60qr5dcqatw2xz

        let mut data = vec![0u8; 676];
        let tx_bytes = hex::decode("07dd7a29000000000000000000000000ba7633f36a86a4f572a918350574c1a\
        44b924ebf000000000000000000000000110caeb55493b119f208b73245464ef5d9a1c39e000000000000000000\
        000000000000000000000000000000000013ac20776c00000000000000000000000000000000000000000000000\
        0000000000000000000000000000000000000000000000000009222e8d83b4d9225000000000000000600000000\
        00000000000000001120e1eb3049148aebee497331774bfe1f6c174d715821d1341a2aad7f41daf9b452dbeddd4\
        8f0a7a05b5ec4f62b67a26091b8a30000000000000000000000004699450973c21d6fe09e36a8a475eae4d78a31\
        370000000000000000000000000000000000000000000000000000000069ee14200000000000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000002000000000000000000000000000000000000000000000000000000000069e77cdc00000\
        0000000000000000000000000000000000000000000000000000000028000000000000000000000000000000000\
        000000000000000000000000000000416546c65d2624fba2d98ce98fa149f43856a0c6f892b24e03774c1da0ed8\
        90c427798d9351b293e7d3e4b6cac34779bcba1d3c1a58f0a3dc042247018a53c28a31c00000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        000000000000000").unwrap();
        data.copy_from_slice(&tx_bytes);


        let result = EscrowData::from_transaction_calldata(&data);
        assert!(result.is_ok());
        let escrow = result.unwrap();

        assert_eq!(
            escrow.offerer, "0xba7633f36a86a4f572a918350574c1a44b924ebf".parse().unwrap(),
            "Offerer mismatch"
        );
        assert_eq!(
            escrow.claimer, "0x110caeb55493b119f208b73245464ef5d9a1c39e".parse().unwrap(),
            "Claimer mismatch"
        );
        assert_eq!(
            escrow.amount, U256::from(21630000000000_u64),
            "Amount must be equal"
        );
        assert_eq!(
            escrow.token, Address::zero(),
            "Destination token is native token"
        );
        assert!(
            escrow.flags.pay_out || escrow.flags.pay_in,
            "At least one of pay_out or pay_in must be true"
        );
        assert!(
            escrow.claim_handler != Address::zero(),
            "Claim handler must not be zero address"
        );
        assert!(
            escrow.refund_handler != Address::zero(),
            "Refund handler must not be zero address"
        );
        assert!(
            !escrow.has_success_action(),
            "Success action should not be set (all zeros)"
        );

        let payment_hash = H256::from(escrow.claim_data);
        let payment_hash_arr: [u8; 32] = payment_hash.try_into().unwrap();

        let hex_formatted = format!("{:#x}", payment_hash);
        assert_eq!(
            hex_formatted,
            "0x7158_21d1_341a_2aad_7f41_daf9_b452_dbed_dd48_f0a7_a05b_5ec4_f62b_67a2_6091_b8a3"
                .replace('_', "").to_lowercase()
        );

        let preimage = "373cbb0a28b180d9f9171480f7f73df4d554620a73cccb05d6c6ce0af6a8d8a4";
        let preimage_bytes = hex::decode(preimage).unwrap();

        let mut sha256 = Sha256::new();
        sha256.update(&preimage_bytes);
        let hash: [u8; 32] = sha256.finalize().into();

        assert_eq!(hash, payment_hash_arr, "Hash must be equal to given preimage");
    }
}