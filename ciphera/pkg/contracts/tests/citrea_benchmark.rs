use std::{
    fs,
    future::Future,
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use contracts::{Address, Client, ConfirmationType, ERC20Contract, FeeStrategy, RollupContract};
use element::Element;
use eyre::{Result, WrapErr, eyre};
use rand::RngCore;
use serde::Serialize;
use serde_json::{Value, json};
use testutil::{
    ACCOUNT_1_SK,
    eth::{EthNode, EthNodeOptions},
};
use web3::{
    ethabi::{Contract as AbiContract, Token},
    signing::{Key, SecretKey, SecretKeyRef},
    types::{BlockId, BlockNumber, Bytes, CallRequest, TransactionParameters, U64, U256},
};
use zk_primitives::{Note, Utxo};

const TYPE2_TX_ID: u64 = 2;
const REPORT_JSON: &str = "citrea_benchmark_latest.json";
const REPORT_MD: &str = "citrea_benchmark_latest.md";
const LOAD_SWEEP_TPS: &[usize] = &[1, 2, 4, 8, 16];
const LOAD_ROUNDS_PER_WORKER: usize = 4;
const LOAD_SENDER_POOL_SIZE: usize = 16;

#[derive(Debug, Serialize)]
struct CaseResult {
    name: String,
    ok: bool,
    duration_ms: u128,
    error: Option<String>,
    data: Value,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    started_at_unix_s: u64,
    rpc_url: String,
    startup_and_deploy_ms: u128,
    deployed: Value,
    fee_snapshot: Value,
    cases: Vec<CaseResult>,
}

struct RawRpcClient {
    client: reqwest::Client,
    rpc_url: String,
}

#[derive(Clone)]
struct LoadSender {
    secret: SecretKey,
    address: Address,
    client: Client,
    erc20: ERC20Contract,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum LoadTxKind {
    NativeTransferType2,
    Erc20Transfer,
}

impl LoadTxKind {
    fn label(&self) -> &'static str {
        match self {
            Self::NativeTransferType2 => "native_type2",
            Self::Erc20Transfer => "erc20_transfer",
        }
    }
}

#[derive(Debug, Serialize)]
struct LoadWorkerResult {
    sender: String,
    recipient: String,
    successes: u64,
    latencies_ms: Vec<u64>,
    failures: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LoadProfileResult {
    tx_kind: LoadTxKind,
    target_tps: usize,
    worker_count: usize,
    rounds_per_worker: usize,
    scheduled_tx_count: usize,
    slot_spacing_us: u64,
    test_duration_ms: u128,
    total_successes: u64,
    total_failures: u64,
    achieved_tps: f64,
    avg_latency_ms: Option<f64>,
    p50_latency_ms: Option<u64>,
    p95_latency_ms: Option<u64>,
    max_latency_ms: Option<u64>,
    workers: Vec<LoadWorkerResult>,
}

impl RawRpcClient {
    fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("failed to build reqwest client"),
            rpc_url: rpc_url.into(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let response = self
            .client
            .post(&self.rpc_url)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .wrap_err_with(|| format!("rpc send failed for {method}"))?;

        let response = response
            .error_for_status()
            .wrap_err_with(|| format!("rpc status error for {method}"))?;

        let value: Value = response
            .json()
            .await
            .wrap_err_with(|| format!("rpc json decode failed for {method}"))?;

        if let Some(error) = value.get("error") {
            return Err(eyre!("rpc {method} failed: {error}"));
        }

        value
            .get("result")
            .cloned()
            .ok_or_else(|| eyre!("rpc {method} returned no result"))
    }
}

fn report_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("benchmark-results")
}

fn report_json_path() -> PathBuf {
    report_dir().join(REPORT_JSON)
}

fn report_md_path() -> PathBuf {
    report_dir().join(REPORT_MD)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_secs()
}

fn address_for_secret(secret: &SecretKey) -> Address {
    Key::address(&SecretKeyRef::new(secret))
}

fn random_secret_key() -> SecretKey {
    loop {
        let mut bytes = [0_u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        if let Ok(secret) = SecretKey::from_slice(&bytes) {
            return secret;
        }
    }
}

fn u256_to_decimal(value: U256) -> String {
    value.to_string()
}

fn u256_from_hex(value: &str) -> Result<U256> {
    let trimmed = value.trim_start_matches("0x");
    Ok(U256::from_str_radix(trimmed, 16)?)
}

fn json_u256(value: U256) -> Value {
    json!({
        "hex": format!("{:#x}", value),
        "decimal": u256_to_decimal(value),
    })
}

fn latency_percentile(latencies_ms: &[u64], percentile: f64) -> Option<u64> {
    if latencies_ms.is_empty() {
        return None;
    }

    let mut sorted = latencies_ms.to_vec();
    sorted.sort_unstable();

    let rank = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted.get(rank).copied()
}

fn decimal_field(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(|value| value.get("decimal"))
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_owned()
}

fn number_field(data: &Value, key: &str) -> String {
    match data.get(key) {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Null) | None => "-".to_owned(),
        Some(value) => value.to_string(),
    }
}

fn float_field(data: &Value, key: &str, precision: usize) -> String {
    data.get(key)
        .and_then(Value::as_f64)
        .map(|value| format!("{value:.precision$}"))
        .unwrap_or_else(|| "-".to_owned())
}

async fn run_case<F, Fut>(name: &str, f: F) -> CaseResult
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let start = Instant::now();
    match f().await {
        Ok(data) => CaseResult {
            name: name.to_owned(),
            ok: true,
            duration_ms: start.elapsed().as_millis(),
            error: None,
            data,
        },
        Err(error) => CaseResult {
            name: name.to_owned(),
            ok: false,
            duration_ms: start.elapsed().as_millis(),
            error: Some(format!("{error:?}")),
            data: json!({}),
        },
    }
}

fn eip1559_fees_json(fees: &contracts::Eip1559Fees) -> Value {
    json!({
        "base_fee": json_u256(fees.base_fee),
        "priority_fee": json_u256(fees.priority_fee),
        "max_fee": json_u256(fees.max_fee),
    })
}

fn type2_transfer_request(from: Address, to: Address, value: U256) -> CallRequest {
    CallRequest {
        from: Some(from),
        to: Some(to),
        value: Some(value),
        transaction_type: Some(U64::from(TYPE2_TX_ID)),
        ..Default::default()
    }
}

fn native_transfer_request(from: Address, to: Address, value: U256) -> CallRequest {
    CallRequest {
        from: Some(from),
        to: Some(to),
        value: Some(value),
        ..Default::default()
    }
}

fn erc20_transfer_request(
    from: Address,
    token: Address,
    to: Address,
    amount: u128,
) -> Result<CallRequest> {
    let contract_json =
        include_str!("../../../citrea/openzeppelin-contracts/token/ERC20/IERC20.json");
    let contract_json_value = serde_json::from_str::<serde_json::Value>(contract_json)?;
    let abi = serde_json::from_value::<AbiContract>(
        contract_json_value
            .get("abi")
            .cloned()
            .ok_or_else(|| eyre!("IERC20 json missing abi"))?,
    )?;
    let function = abi.function("transfer")?;
    let data = function.encode_input(&[Token::Address(to), Token::Uint(amount.into())])?;

    Ok(CallRequest {
        from: Some(from),
        to: Some(token),
        data: Some(Bytes(data)),
        transaction_type: Some(U64::from(TYPE2_TX_ID)),
        ..Default::default()
    })
}

async fn send_signed_transaction(
    client: &Client,
    secret: &SecretKey,
    tx: TransactionParameters,
) -> Result<web3::types::H256> {
    let signed = client
        .client()
        .accounts()
        .sign_transaction(tx, secret)
        .await
        .wrap_err("sign_transaction failed")?;

    let tx_hash = client
        .client()
        .eth()
        .send_raw_transaction(signed.raw_transaction)
        .await
        .wrap_err("send_raw_transaction failed")?;

    Ok(tx_hash)
}

async fn send_type2_native_transfer(
    client: &Client,
    secret: &SecretKey,
    to: Address,
    value: U256,
    gas_limit: U256,
    fees: &contracts::Eip1559Fees,
) -> Result<web3::types::H256> {
    let tx = TransactionParameters {
        nonce: Some(client.nonce(address_for_secret(secret)).await?),
        to: Some(to),
        gas: gas_limit,
        value,
        chain_id: Some(client.chain_id().await?.as_u64()),
        transaction_type: Some(U64::from(TYPE2_TX_ID)),
        max_fee_per_gas: Some(fees.max_fee),
        max_priority_fee_per_gas: Some(fees.priority_fee),
        ..Default::default()
    };

    send_signed_transaction(client, secret, tx).await
}

async fn send_legacy_native_transfer(
    client: &Client,
    secret: &SecretKey,
    to: Address,
    value: U256,
    gas_limit: U256,
    gas_price: U256,
) -> Result<web3::types::H256> {
    let tx = TransactionParameters {
        nonce: Some(client.nonce(address_for_secret(secret)).await?),
        to: Some(to),
        gas: gas_limit,
        value,
        gas_price: Some(gas_price),
        chain_id: Some(client.chain_id().await?.as_u64()),
        ..Default::default()
    };

    send_signed_transaction(client, secret, tx).await
}

async fn wait_for_receipt_json(
    client: &Client,
    raw_rpc: &RawRpcClient,
    tx_hash: web3::types::H256,
) -> Result<Value> {
    client
        .wait_for_confirm(
            tx_hash,
            Duration::from_millis(200),
            ConfirmationType::Latest,
        )
        .await
        .wrap_err("wait_for_confirm failed")?;

    raw_rpc
        .call(
            "eth_getTransactionReceipt",
            json!([format!("{:#x}", tx_hash)]),
        )
        .await
        .wrap_err("raw receipt fetch failed")
}

async fn block_json(
    client: &Client,
    block_number: u64,
) -> Result<web3::types::Block<web3::types::H256>> {
    client
        .client()
        .eth()
        .block(BlockId::Number(BlockNumber::Number(block_number.into())))
        .await?
        .ok_or_else(|| eyre!("block {block_number} not found"))
}

async fn observe_transaction(
    client: &Client,
    raw_rpc: &RawRpcClient,
    tx_hash: web3::types::H256,
    max_lock: Option<U256>,
) -> Result<Value> {
    let receipt_json = wait_for_receipt_json(client, raw_rpc, tx_hash).await?;
    let receipt = client
        .client()
        .eth()
        .transaction_receipt(tx_hash)
        .await?
        .ok_or_else(|| eyre!("typed receipt missing for {tx_hash:#x}"))?;

    let block_number = receipt
        .block_number
        .ok_or_else(|| eyre!("receipt missing block number"))?;
    let block = block_json(client, block_number.as_u64()).await?;
    let l1_fee_rate = block.l1_fee_rate.unwrap_or_default();
    let l1_diff_size = receipt_json
        .get("l1DiffSize")
        .and_then(Value::as_str)
        .map(u256_from_hex)
        .transpose()?
        .unwrap_or_default();

    let gas_used = receipt.gas_used.unwrap_or_default();
    let effective_gas_price = receipt.effective_gas_price.unwrap_or_default();
    let l2_fee_paid = gas_used.saturating_mul(effective_gas_price);
    let l1_fee_paid = l1_fee_rate.saturating_mul(l1_diff_size);

    Ok(json!({
        "tx_hash": format!("{:#x}", tx_hash),
        "block_number": block_number.as_u64(),
        "gas_used": json_u256(gas_used),
        "effective_gas_price": json_u256(effective_gas_price),
        "l2_fee_paid": json_u256(l2_fee_paid),
        "l1_diff_size": json_u256(l1_diff_size),
        "l1_fee_rate": json_u256(l1_fee_rate),
        "l1_fee_paid": json_u256(l1_fee_paid),
        "max_lock": max_lock.map(json_u256),
        "receipt": receipt_json,
    }))
}

async fn fund_sender(
    funder_client: &Client,
    funder_secret: &SecretKey,
    erc20: &ERC20Contract,
    recipient: Address,
    native_amount: U256,
    token_amount: u128,
    fees: &contracts::Eip1559Fees,
) -> Result<()> {
    let native_hash = send_type2_native_transfer(
        funder_client,
        funder_secret,
        recipient,
        native_amount,
        21_000_u64.into(),
        fees,
    )
    .await
    .wrap_err_with(|| format!("native funding transfer failed for recipient {recipient:#x}"))?;
    funder_client
        .wait_for_confirm(
            native_hash,
            Duration::from_millis(200),
            ConfirmationType::Latest,
        )
        .await
        .wrap_err_with(|| {
            format!("native funding confirmation failed for recipient {recipient:#x}")
        })?;

    let token_hash = erc20
        .transfer(recipient, token_amount)
        .await
        .wrap_err_with(|| format!("erc20 funding transfer failed for recipient {recipient:#x}"))?;
    erc20
        .client()
        .wait_for_confirm(
            token_hash,
            Duration::from_millis(200),
            ConfirmationType::Latest,
        )
        .await
        .wrap_err_with(|| {
            format!("erc20 funding confirmation failed for recipient {recipient:#x}")
        })?;

    Ok(())
}

async fn provision_load_senders(
    eth_node: &EthNode,
    funder_client: &Client,
    funder_secret: &SecretKey,
    erc20: &ERC20Contract,
    count: usize,
) -> Result<Vec<LoadSender>> {
    let funding_fees = funder_client
        .estimate_eip1559_fees(FeeStrategy::Fast)
        .await
        .wrap_err("failed to estimate fast fees for load sender funding")?;

    let mut senders = Vec::with_capacity(count);
    for sender_index in 0..count {
        let sender_secret = random_secret_key();
        let sender_address = address_for_secret(&sender_secret);
        fund_sender(
            funder_client,
            funder_secret,
            erc20,
            sender_address,
            U256::exp10(16),
            100_000,
            &funding_fees,
        )
        .await
        .wrap_err_with(|| {
            format!("failed to provision load sender {sender_index} at {sender_address:#x}")
        })?;

        let sender_client = Client::from_eth_node(eth_node);
        let sender_erc20 = ERC20Contract::load(
            sender_client.clone(),
            &format!("{:#x}", erc20.address()),
            sender_secret.clone(),
        )
        .await
        .wrap_err_with(|| {
            format!(
                "failed to load ERC20 contract for provisioned load sender {sender_index} at {sender_address:#x}"
            )
        })?;

        senders.push(LoadSender {
            secret: sender_secret,
            address: sender_address,
            client: sender_client,
            erc20: sender_erc20,
        });
    }

    Ok(senders)
}

async fn run_load_profile(
    senders: &[LoadSender],
    tx_kind: LoadTxKind,
    target_tps: usize,
    rounds_per_worker: usize,
) -> Result<Value> {
    if target_tps == 0 {
        return Err(eyre!("target_tps must be positive"));
    }
    if target_tps > senders.len() {
        return Err(eyre!(
            "target_tps={} exceeds provisioned sender pool={}",
            target_tps,
            senders.len()
        ));
    }

    let active_senders = senders[..target_tps].to_vec();
    let slot_spacing_us = (1_000_000.0 / target_tps as f64).round() as u64;
    let started = tokio::time::Instant::now() + Duration::from_millis(300);
    let test_started = Instant::now();
    let mut handles = Vec::with_capacity(active_senders.len());

    for (index, sender) in active_senders.into_iter().enumerate() {
        let recipient = address_for_secret(&random_secret_key());
        let worker_started =
            started + Duration::from_micros(slot_spacing_us.saturating_mul(index as u64));
        handles.push(tokio::spawn(async move {
            tokio::time::sleep_until(worker_started).await;
            let mut latencies_ms = Vec::with_capacity(rounds_per_worker);
            let mut successes = 0_u64;
            let mut failures = Vec::new();

            for round in 0..rounds_per_worker {
                let start = Instant::now();
                let send_result = match tx_kind {
                    LoadTxKind::NativeTransferType2 => {
                        let fees = sender
                            .client
                            .estimate_eip1559_fees(FeeStrategy::default())
                            .await
                            .wrap_err("failed to estimate fees for native load transfer");

                        match fees {
                            Ok(fees) => send_type2_native_transfer(
                                &sender.client,
                                &sender.secret,
                                recipient,
                                U256::from(1_u64),
                                21_000_u64.into(),
                                &fees,
                            )
                            .await
                            .wrap_err("failed to send native load transfer"),
                            Err(error) => Err(error),
                        }
                    }
                    LoadTxKind::Erc20Transfer => sender
                        .erc20
                        .transfer(recipient, 1)
                        .await
                        .wrap_err("failed to send erc20 load transfer"),
                };

                match send_result {
                    Ok(tx_hash) => {
                        let confirm_result = match tx_kind {
                            LoadTxKind::NativeTransferType2 => sender
                                .client
                                .wait_for_confirm(
                                    tx_hash,
                                    Duration::from_millis(200),
                                    ConfirmationType::Latest,
                                )
                                .await
                                .wrap_err("failed to confirm native load transfer")
                                .map(|_| ()),
                            LoadTxKind::Erc20Transfer => sender
                                .erc20
                                .client()
                                .wait_for_confirm(
                                    tx_hash,
                                    Duration::from_millis(200),
                                    ConfirmationType::Latest,
                                )
                                .await
                                .wrap_err("failed to confirm erc20 load transfer")
                                .map(|_| ()),
                        };

                        match confirm_result {
                            Ok(()) => {
                                successes += 1;
                                latencies_ms.push(start.elapsed().as_millis() as u64);
                            }
                            Err(error) => failures.push(format!(
                                "confirm round {round} kind {} sender {:#x}: {error:?}",
                                tx_kind.label(),
                                sender.address
                            )),
                        }
                    }
                    Err(error) => failures.push(format!(
                        "send round {round} kind {} sender {:#x}: {error:?}",
                        tx_kind.label(),
                        sender.address
                    )),
                }

                if round + 1 < rounds_per_worker {
                    let next_slot = worker_started + Duration::from_secs((round + 1) as u64);
                    tokio::time::sleep_until(next_slot).await;
                }
            }

            LoadWorkerResult {
                sender: format!("{:#x}", sender.address),
                recipient: format!("{:#x}", recipient),
                successes,
                latencies_ms,
                failures,
            }
        }));
    }

    let mut worker_results = Vec::new();
    let mut all_latencies_ms = Vec::new();
    let mut total_successes = 0_u64;
    let mut total_failures = 0_u64;

    for handle in handles {
        let result = handle.await?;
        total_successes += result.successes;
        total_failures += result.failures.len() as u64;
        all_latencies_ms.extend(result.latencies_ms.iter().copied());
        worker_results.push(result);
    }

    let elapsed = test_started.elapsed();
    let load_result = LoadProfileResult {
        tx_kind,
        target_tps,
        worker_count: target_tps,
        rounds_per_worker,
        scheduled_tx_count: target_tps * rounds_per_worker,
        slot_spacing_us,
        test_duration_ms: elapsed.as_millis(),
        total_successes,
        total_failures,
        achieved_tps: if elapsed.is_zero() {
            0.0
        } else {
            total_successes as f64 / elapsed.as_secs_f64()
        },
        avg_latency_ms: if all_latencies_ms.is_empty() {
            None
        } else {
            Some(all_latencies_ms.iter().sum::<u64>() as f64 / all_latencies_ms.len() as f64)
        },
        p50_latency_ms: latency_percentile(&all_latencies_ms, 0.50),
        p95_latency_ms: latency_percentile(&all_latencies_ms, 0.95),
        max_latency_ms: all_latencies_ms.iter().max().copied(),
        workers: worker_results,
    };

    Ok(serde_json::to_value(load_result)?)
}

fn benchmark_markdown(report: &BenchmarkReport) -> String {
    let mut out = String::new();
    out.push_str("# Citrea Benchmark Report\n\n");
    out.push_str(&format!(
        "- Started at (unix): `{}`\n- RPC URL: `{}`\n- Startup + deploy: `{} ms`\n\n",
        report.started_at_unix_s, report.rpc_url, report.startup_and_deploy_ms
    ));
    out.push_str("## Fee Strategy Matrix\n\n");
    out.push_str("| Strategy | Base Fee | Priority Fee | Max Fee |\n");
    out.push_str("| --- | ---: | ---: | ---: |\n");
    for (label, key) in [
        ("Lowest", "client_lowest_fees"),
        ("Slow", "client_slow_fees"),
        ("Standard", "client_standard_fees"),
        ("Fast", "client_fast_fees"),
    ] {
        if let Some(fees) = report.fee_snapshot.get(key) {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                label,
                decimal_field(fees, "base_fee"),
                decimal_field(fees, "priority_fee"),
                decimal_field(fees, "max_fee"),
            ));
        }
    }
    out.push('\n');

    out.push_str("## Single Transaction Matrix\n\n");
    out.push_str("| Case | Status | Gas Used | Effective Gas Price | L1 Diff Size | Duration |\n");
    out.push_str("| --- | --- | ---: | ---: | ---: | ---: |\n");
    for case in &report.cases {
        if case.data.get("gas_used").is_some() {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} ms |\n",
                case.name,
                if case.ok { "ok" } else { "error" },
                decimal_field(&case.data, "gas_used"),
                decimal_field(&case.data, "effective_gas_price"),
                decimal_field(&case.data, "l1_diff_size"),
                case.duration_ms,
            ));
        }
    }
    out.push('\n');

    out.push_str("## Estimate Matrix\n\n");
    out.push_str("| Case | Status | estimateGas | diffGas | L1 Diff Size | Duration |\n");
    out.push_str("| --- | --- | ---: | ---: | ---: | ---: |\n");
    for case in &report.cases {
        if let Some(diff) = case.data.get("estimate_diff_size") {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} ms |\n",
                case.name,
                if case.ok { "ok" } else { "error" },
                decimal_field(&case.data, "estimate_gas"),
                decimal_field(diff, "gas"),
                decimal_field(diff, "l1_diff_size"),
                case.duration_ms,
            ));
        }
    }
    out.push('\n');

    out.push_str("## Load Sweep Matrix\n\n");
    out.push_str(
        "| Case | Tx Kind | Target TPS | Successes | Failures | Achieved TPS | Avg Latency | P95 Latency | Max Latency | Duration |\n",
    );
    out.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for case in &report.cases {
        if case.data.get("target_tps").is_some() && case.data.get("achieved_tps").is_some() {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} ms |\n",
                case.name,
                case.data
                    .get("tx_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("-"),
                number_field(&case.data, "target_tps"),
                number_field(&case.data, "total_successes"),
                number_field(&case.data, "total_failures"),
                float_field(&case.data, "achieved_tps", 2),
                float_field(&case.data, "avg_latency_ms", 1),
                number_field(&case.data, "p95_latency_ms"),
                number_field(&case.data, "max_latency_ms"),
                case.duration_ms,
            ));
        }
    }
    out.push('\n');

    out.push_str("## Fee Snapshot\n\n");
    out.push_str("```json\n");
    out.push_str(&serde_json::to_string_pretty(&report.fee_snapshot).unwrap());
    out.push_str("\n```\n\n");
    out.push_str("## Cases\n\n");
    for case in &report.cases {
        let status = if case.ok { "ok" } else { "error" };
        out.push_str(&format!(
            "### {}\n\n- Status: `{}`\n- Duration: `{} ms`\n",
            case.name, status, case.duration_ms
        ));
        if let Some(error) = &case.error {
            out.push_str(&format!("- Error: `{}`\n", error.replace('`', "'")));
        }
        out.push_str("\n```json\n");
        out.push_str(&serde_json::to_string_pretty(&case.data).unwrap());
        out.push_str("\n```\n\n");
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn citrea_benchmark() -> Result<()> {
    let started_at_unix_s = unix_now();
    let startup_started = Instant::now();
    let eth_node = EthNode::new(EthNodeOptions {
        use_noop_verifier: true,
        ..Default::default()
    })
    .run_and_deploy()
    .await;
    let startup_and_deploy_ms = startup_started.elapsed().as_millis();

    let rpc_url = eth_node.rpc_url();
    let funder_secret = SecretKey::from_str(ACCOUNT_1_SK)?;
    let funder_address = address_for_secret(&funder_secret);
    let client = Client::from_eth_node(&eth_node);
    let rollup = RollupContract::from_eth_node(&eth_node, funder_secret.clone()).await?;
    let erc20 = ERC20Contract::from_eth_node(&eth_node, funder_secret.clone()).await?;

    let latest_block = client
        .client()
        .eth()
        .block(BlockId::Number(BlockNumber::Latest))
        .await?
        .ok_or_else(|| eyre!("latest block missing"))?;
    let gas_price = client.client().eth().gas_price().await?;
    let fee_history = client
        .client()
        .eth()
        .fee_history(
            U256::from(10),
            BlockNumber::Latest,
            Some(vec![1.0, 25.0, 50.0, 90.0]),
        )
        .await?;
    let current_lowest_fees = client.estimate_eip1559_fees(FeeStrategy::Lowest).await?;
    let current_slow_fees = client.estimate_eip1559_fees(FeeStrategy::Slow).await?;
    let current_standard_fees = client.estimate_eip1559_fees(FeeStrategy::Standard).await?;
    let current_fast_fees = client.estimate_eip1559_fees(FeeStrategy::Fast).await?;

    let fee_snapshot = json!({
        "block_number": latest_block.number.map(|n| n.as_u64()),
        "base_fee_per_gas": latest_block.base_fee_per_gas.map(json_u256),
        "l1_fee_rate": latest_block.l1_fee_rate.map(json_u256),
        "gas_price": json_u256(gas_price),
        "fee_history": fee_history,
        "client_lowest_fees": eip1559_fees_json(&current_lowest_fees),
        "client_slow_fees": eip1559_fees_json(&current_slow_fees),
        "client_standard_fees": eip1559_fees_json(&current_standard_fees),
        "client_fast_fees": eip1559_fees_json(&current_fast_fees),
        "default_write_fee_strategy": format!("{:?}", FeeStrategy::default()),
    });

    let mut cases = Vec::new();

    cases.push(
        run_case("estimate_compare_native_transfer", || {
            let client = client.clone();
            async move {
                let recipient = address_for_secret(&random_secret_key());
                let call = type2_transfer_request(funder_address, recipient, U256::from(1_u64));
                let estimate_gas = client
                    .client()
                    .eth()
                    .estimate_gas(call.clone(), None)
                    .await?;
                let diff = client.client().eth().estimate_diff_size(call, None).await?;
                let fees = client.estimate_eip1559_fees(FeeStrategy::Lowest).await?;
                let precise_gas_limit = diff.gas.saturating_add(diff.gas / 10);
                let naive_lock = estimate_gas.saturating_mul(fees.max_fee);
                let precise_lock = precise_gas_limit.saturating_mul(fees.max_fee);
                let l1_fee_rate = latest_block.l1_fee_rate.unwrap_or_default();
                let estimated_l1_fee = diff.l1_diff_size.saturating_mul(l1_fee_rate);

                Ok(json!({
                    "estimate_gas": json_u256(estimate_gas),
                    "estimate_diff_size": {
                        "gas": json_u256(diff.gas),
                        "l1_diff_size": json_u256(diff.l1_diff_size),
                    },
                    "current_lowest_fees": eip1559_fees_json(&fees),
                    "precise_gas_limit_with_10_percent_buffer": json_u256(precise_gas_limit),
                    "naive_lock_max_fee_times_estimate_gas": json_u256(naive_lock),
                    "precise_lock_max_fee_times_precise_gas_limit": json_u256(precise_lock),
                    "estimated_l1_fee_from_diff_size": json_u256(estimated_l1_fee),
                }))
            }
        })
        .await,
    );

    cases.push(
        run_case("estimate_compare_erc20_transfer", || {
            let client = client.clone();
            let erc20 = erc20.clone();
            async move {
                let recipient = address_for_secret(&random_secret_key());
                let call = erc20_transfer_request(funder_address, erc20.address(), recipient, 1)?;
                let estimate_gas = client
                    .client()
                    .eth()
                    .estimate_gas(call.clone(), None)
                    .await?;
                let diff = client.client().eth().estimate_diff_size(call, None).await?;
                let fees = client.estimate_eip1559_fees(FeeStrategy::Lowest).await?;
                let precise_gas_limit = diff.gas.saturating_add(diff.gas / 10);

                Ok(json!({
                    "estimate_gas": json_u256(estimate_gas),
                    "estimate_diff_size": {
                        "gas": json_u256(diff.gas),
                        "l1_diff_size": json_u256(diff.l1_diff_size),
                    },
                    "current_lowest_fees": eip1559_fees_json(&fees),
                    "precise_gas_limit_with_10_percent_buffer": json_u256(precise_gas_limit),
                }))
            }
        })
        .await,
    );

    cases.push(
        run_case("type2_native_transfer_naive_gaslimit", || {
            let client = client.clone();
            let raw_rpc = RawRpcClient::new(rpc_url.clone());
            let funder_secret = funder_secret.clone();
            async move {
                let recipient = address_for_secret(&random_secret_key());
                let call = type2_transfer_request(funder_address, recipient, U256::from(1_u64));
                let estimate_gas = client.client().eth().estimate_gas(call, None).await?;
                let fees = client.estimate_eip1559_fees(FeeStrategy::Lowest).await?;
                let tx_hash = send_type2_native_transfer(
                    &client,
                    &funder_secret,
                    recipient,
                    U256::from(1_u64),
                    estimate_gas,
                    &fees,
                )
                .await?;

                observe_transaction(
                    &client,
                    &raw_rpc,
                    tx_hash,
                    Some(estimate_gas.saturating_mul(fees.max_fee)),
                )
                .await
            }
        })
        .await,
    );

    cases.push(
        run_case("type2_native_transfer_precise_gaslimit", || {
            let client = client.clone();
            let raw_rpc = RawRpcClient::new(rpc_url.clone());
            let funder_secret = funder_secret.clone();
            async move {
                let recipient = address_for_secret(&random_secret_key());
                let call = type2_transfer_request(funder_address, recipient, U256::from(1_u64));
                let diff = client.client().eth().estimate_diff_size(call, None).await?;
                let precise_gas_limit = diff.gas.saturating_add(diff.gas / 10);
                let fees = client.estimate_eip1559_fees(FeeStrategy::Lowest).await?;
                let tx_hash = send_type2_native_transfer(
                    &client,
                    &funder_secret,
                    recipient,
                    U256::from(1_u64),
                    precise_gas_limit,
                    &fees,
                )
                .await?;

                observe_transaction(
                    &client,
                    &raw_rpc,
                    tx_hash,
                    Some(precise_gas_limit.saturating_mul(fees.max_fee)),
                )
                .await
            }
        })
        .await,
    );

    cases.push(
        run_case("legacy_native_transfer", || {
            let client = client.clone();
            let raw_rpc = RawRpcClient::new(rpc_url.clone());
            let funder_secret = funder_secret.clone();
            async move {
                let recipient = address_for_secret(&random_secret_key());
                let call = native_transfer_request(funder_address, recipient, U256::from(1_u64));
                let diff = client.client().eth().estimate_diff_size(call, None).await?;
                let gas_limit = diff.gas.saturating_add(diff.gas / 10);
                let gas_price = client.client().eth().gas_price().await?;
                let tx_hash = send_legacy_native_transfer(
                    &client,
                    &funder_secret,
                    recipient,
                    U256::from(1_u64),
                    gas_limit,
                    gas_price,
                )
                .await?;

                observe_transaction(
                    &client,
                    &raw_rpc,
                    tx_hash,
                    Some(gas_limit.saturating_mul(gas_price)),
                )
                .await
            }
        })
        .await,
    );

    cases.push(
        run_case("erc20_approve_current_client", || {
            let erc20 = erc20.clone();
            let raw_rpc = RawRpcClient::new(rpc_url.clone());
            async move {
                let spender = address_for_secret(&random_secret_key());
                let tx_hash = erc20.approve(spender, 10_000).await?;
                observe_transaction(erc20.client(), &raw_rpc, tx_hash, None).await
            }
        })
        .await,
    );

    cases.push(
        run_case("erc20_transfer_current_client", || {
            let erc20 = erc20.clone();
            let raw_rpc = RawRpcClient::new(rpc_url.clone());
            async move {
                let recipient = address_for_secret(&random_secret_key());
                let tx_hash = erc20.transfer(recipient, 1_000).await?;
                observe_transaction(erc20.client(), &raw_rpc, tx_hash, None).await
            }
        })
        .await,
    );

    cases.push(
        run_case("rollup_mint_current_client", || {
            let raw_rpc = RawRpcClient::new(rpc_url.clone());
            async move {
                let note =
                    Note::new_with_psi(Element::new(0xBEEF), Element::from(25_u64), Element::ZERO);
                let utxo = Utxo::new_mint([note.clone(), Note::padding_note()]);
                let tx_hash = rollup
                    .mint(&utxo.mint_hash(), &note.value, &note.contract)
                    .await?;
                observe_transaction(&rollup.client, &raw_rpc, tx_hash, None).await
            }
        })
        .await,
    );

    let load_sender_pool_started = Instant::now();
    let load_senders = provision_load_senders(
        &eth_node,
        &client,
        &funder_secret,
        &erc20,
        LOAD_SENDER_POOL_SIZE,
    )
    .await?;
    let load_sender_pool_setup_ms = load_sender_pool_started.elapsed().as_millis();

    for &target_tps in LOAD_SWEEP_TPS {
        let case_name = format!("native_type2_load_{}_tps", target_tps);
        cases.push(
            run_case(&case_name, || {
                let load_senders = load_senders.clone();
                async move {
                    run_load_profile(
                        &load_senders,
                        LoadTxKind::NativeTransferType2,
                        target_tps,
                        LOAD_ROUNDS_PER_WORKER,
                    )
                    .await
                }
            })
            .await,
        );
    }

    for &target_tps in LOAD_SWEEP_TPS {
        let case_name = format!("erc20_transfer_load_{}_tps", target_tps);
        cases.push(
            run_case(&case_name, || {
                let load_senders = load_senders.clone();
                async move {
                    run_load_profile(
                        &load_senders,
                        LoadTxKind::Erc20Transfer,
                        target_tps,
                        LOAD_ROUNDS_PER_WORKER,
                    )
                    .await
                }
            })
            .await,
        );
    }

    let deployed = json!({
        "rollup_proxy": eth_node.deployed().rollup_proxy,
        "erc20": eth_node.deployed().erc20,
        "funder_address": format!("{:#x}", funder_address),
        "load_sender_pool_size": LOAD_SENDER_POOL_SIZE,
        "load_sender_pool_setup_ms": load_sender_pool_setup_ms,
    });

    let report = BenchmarkReport {
        started_at_unix_s,
        rpc_url,
        startup_and_deploy_ms,
        deployed,
        fee_snapshot,
        cases,
    };

    fs::create_dir_all(report_dir())?;
    fs::write(report_json_path(), serde_json::to_string_pretty(&report)?)?;
    fs::write(report_md_path(), benchmark_markdown(&report))?;

    println!(
        "Citrea benchmark report written to {} and {}",
        report_json_path().display(),
        report_md_path().display()
    );

    Ok(())
}
