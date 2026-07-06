//! Integration tests for the CLI module.
//!
//! These tests connect to the live Ciphera testnet node.
//! TLS is inferred from the scheme prefix of the host URI:
//!   "https://…" → HTTPS
//!   "http://…" or bare host → HTTP
//! Live-node tests use an https:// host for the remote endpoint.
//!
//! Run all integration tests:
//!   cargo test --test integration -- --nocapture
//!
//! Run a single test:
//!   cargo test --test integration test_node_health -- --nocapture
//!
//! Skip integration tests (unit tests only):
//!   cargo test --lib

use cli::NodeClient;
use cli::Wallet;
use cli::rpc::ListTxnsQuery;
use std::path::Path;
use tempdir::TempDir;

const NODE_HOST: &str = "https://ciphera.satsbridge.com";
const NODE_PORT: u16 = 80;
const CHAIN_ID: u64 = 5115; // Citrea testnet

fn wallet_name(suffix: &str) -> String {
    format!("integration-test-{suffix}")
}

/// Build a fresh NodeClient for each test, creating the wallet if absent.
/// Uses a unique name per test to avoid cross-test file conflicts.
fn temp_wallet_dir() -> TempDir {
    TempDir::new("ciphera-cli-wallet").unwrap()
}

fn build_client_in(wallet_dir: &Path, name: &str) -> cli::NodeClient {
    let wallet_path = Wallet::wallet_path_in(wallet_dir, name);
    let create = !wallet_path.exists();
    let builder = NodeClient::builder()
        .name(name)
        .host(NODE_HOST)
        .port(NODE_PORT)
        .wallet_dir(wallet_dir);
    let built = if create {
        builder.build_create(CHAIN_ID)
    } else {
        builder.build_load()
    };
    built.unwrap_or_else(|e| panic!("NodeClient::build failed for '{name}': {e}"))
}

fn build_client(name: &str) -> (TempDir, cli::NodeClient) {
    let wallet_dir = temp_wallet_dir();
    let client = build_client_in(wallet_dir.path(), name);
    (wallet_dir, client)
}

// =====================================================================
// Connectivity — live node
// =====================================================================

/// Smoke test: node is reachable and returns a valid health response.

#[ignore]
#[tokio::test]
async fn test_node_health() {
    let name = wallet_name("health");
    let (_wallet_dir, client) = build_client(&name);

    let health = client
        .check_health()
        .await
        .expect("health check should succeed against live node");

    assert!(
        health.height > 0,
        "live node should report height > 0, got {}",
        health.height
    );
}

/// Two consecutive height polls should return non-decreasing values.
/// Guards against the node reporting height = 0 (stuck / wrong endpoint).
#[ignore]
#[tokio::test]
async fn test_height_is_nonzero_and_advances() {
    let name = wallet_name("height");
    let (_wallet_dir, client) = build_client(&name);

    let h1 = client.get_height().await.expect("first height call");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let h2 = client.get_height().await.expect("second height call");

    assert!(h1 > 0, "first height must be > 0");
    assert!(h2 >= h1, "height must not decrease: h1={h1} h2={h2}");
}

// =====================================================================
// Transaction list — live node
// =====================================================================

/// The transaction list endpoint returns without error and has correct shape.
#[ignore]
#[tokio::test]
async fn test_list_transactions_returns_valid_shape() {
    let name = wallet_name("list");
    let (_wallet_dir, client) = build_client(&name);

    let resp = client
        .list_transactions(&ListTxnsQuery::default())
        .await
        .expect("list_transactions should succeed");

    let _ = resp.txns.len(); // accessible without panic → shape is correct
}

/// Requesting a limited page of transactions should honour the limit.
#[ignore]
#[tokio::test]
async fn test_list_transactions_with_limit() {
    let name = wallet_name("list-limit");
    let (_wallet_dir, client) = build_client(&name);

    let resp = client
        .list_transactions(&ListTxnsQuery {
            limit: Some(5),
            ..Default::default()
        })
        .await
        .expect("list_transactions with limit=5 should succeed");

    assert!(
        resp.txns.len() <= 5,
        "response must respect limit=5, got {}",
        resp.txns.len()
    );
}

// =====================================================================
// Wallet sync — live node
// =====================================================================

/// Sync must not reduce the wallet balance (only adds confirmed notes).
#[ignore]
#[tokio::test]
async fn test_sync_does_not_corrupt_empty_wallet() {
    let name = wallet_name("sync-empty");
    let (_wallet_dir, mut client) = build_client(&name);
    let initial_balance = client.get_wallet().balance;

    let resp = client
        .list_transactions(&ListTxnsQuery::default())
        .await
        .expect("list_transactions");

    let (synced_wallet, _) = client
        .get_wallet()
        .prepare_sync(&resp.txns)
        .expect("sync should not fail on a fresh wallet");
    client.replace_wallet(synced_wallet);

    assert!(
        client.get_wallet().balance >= initial_balance,
        "sync must not reduce balance"
    );
}

/// Syncing the same transaction list twice must not double-credit the wallet.
#[ignore]
#[tokio::test]
async fn test_sync_is_idempotent() {
    let name = wallet_name("sync-idempotent");
    let (_wallet_dir, mut client) = build_client(&name);

    let resp = client
        .list_transactions(&ListTxnsQuery::default())
        .await
        .expect("list_transactions");

    let (synced_wallet, _) = client
        .get_wallet()
        .prepare_sync(&resp.txns)
        .expect("first sync");
    client.replace_wallet(synced_wallet);
    let balance_after_first = client.get_wallet().balance;

    let (synced_wallet, _) = client
        .get_wallet()
        .prepare_sync(&resp.txns)
        .expect("second sync");
    client.replace_wallet(synced_wallet);
    let balance_after_second = client.get_wallet().balance;

    assert_eq!(
        balance_after_first, balance_after_second,
        "syncing the same transactions twice must not double-credit the wallet"
    );
}

// =====================================================================
// build() — wallet create / load (offline)
// =====================================================================

/// create_wallet=true fails with a descriptive error when the file already exists.
/// Wallet::create no longer silently overwrites an existing wallet.
#[test]
fn test_build_create_fails_when_wallet_exists() {
    let name = "build-create-exists-test";
    let wallet_dir = temp_wallet_dir();

    // First creation must succeed.
    NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_create(CHAIN_ID)
        .expect("first create should succeed");

    // Second creation on the same name must fail.
    let result = NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_create(CHAIN_ID);

    let err = result.expect_err("create_wallet=true must fail when wallet already exists");
    let msg = format!("{err}");
    assert!(
        msg.contains("exists") || msg.contains("Exists"),
        "error should mention the file already exists; got: {msg}"
    );
}

/// create_wallet=false loads an existing wallet successfully.
#[test]
fn test_build_load_succeeds_when_wallet_exists() {
    let name = "build-load-exists-test";
    let wallet_dir = temp_wallet_dir();

    NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_create(CHAIN_ID)
        .expect("pre-create");

    let result = NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_load();

    assert!(
        result.is_ok(),
        "create_wallet=false should load existing wallet: {:?}",
        result.err()
    );
}

/// create_wallet=false fails when the wallet file is absent.
#[test]
fn test_build_load_fails_when_wallet_absent() {
    let name = "build-load-absent-test";
    let wallet_dir = temp_wallet_dir();

    let result = NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_load();

    let err = result.expect_err("create_wallet=false must fail when file is absent");
    let msg = format!("{err}");
    assert!(
        msg.contains("not found") || msg.contains("NotFound") || msg.contains("FileNotFound"),
        "error should mention file not found; got: {msg}"
    );
}

/// build_load is chain-agnostic: the wallet file is authoritative, so a
/// loaded wallet keeps its own bound chain_id (no `--chain` to match).
#[test]
fn test_build_load_preserves_wallet_chain_id() {
    let name = "build-chain-preserve-test";
    let wallet_dir = temp_wallet_dir();

    NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_create(CHAIN_ID)
        .expect("pre-create with CHAIN_ID=5115");

    let client = NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_load()
        .expect("build_load should succeed for an existing wallet");

    assert_eq!(client.get_wallet().chain_id, Some(CHAIN_ID));
}

// =====================================================================
// Error propagation — regression for handle_note_spend bug
// =====================================================================

/// Regression: build() with a malformed wallet file must surface a
/// serialization error, not be swallowed into a generic "Builder error".
///
/// Catches the bug in handle_note_spend where
///   .map_err(|_| AppError::CantBuildClient()) discards the WalletError.
#[test]
fn test_build_propagates_serialization_error_on_bad_json() {
    let name = "malformed-wallet-integration-test";
    let wallet_dir = temp_wallet_dir();
    let file = Wallet::wallet_path_in(wallet_dir.path(), name);
    std::fs::write(&file, b"not valid json").unwrap();

    let result = NodeClient::builder()
        .name(name)
        .wallet_dir(wallet_dir.path())
        .build_load(); // load, not create

    let err = result.expect_err("build must fail with malformed wallet file");
    let msg = format!("{err}");
    assert!(
        msg.contains("Serialization")
            || msg.contains("JSON")
            || msg.contains("json")
            || msg.contains("parse")
            || msg.contains("deserializ"),
        "error should mention deserialization, not just 'Builder error': {msg}"
    );
}
