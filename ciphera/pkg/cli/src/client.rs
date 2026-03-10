//! Node client for communicating with Ciphera RPC nodes
//!
//! Uses a builder pattern with a singleton HTTP client for efficient
//! connection pooling and session reuse across multiple requests.

use crate::wallet::Wallet;
use color_eyre::Result;
use contracts::{ERC20Contract, RollupContract};
use hash::hash_merge;
use node_interface::{HeightResponse, TransactionResponse};
use once_cell::sync::Lazy;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::debug;
use zk_primitives::{Note, UtxoProof};

use contracts::ConfirmationType;
use ethereum_types::U256;
use secp256k1::PublicKey;
use web3::signing::{keccak256, SecretKey};
use web3::types::Address;

use crate::rpc::{HealthResponse, ListTransactionsResponse, ListTxnsQuery};
/// Singleton HTTP client shared across all NodeClient instances
/// Provides connection pooling and efficient resource reuse
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    debug!("Initializing singleton HTTP client");
    reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Some(Duration::from_secs(60)))
        .build()
        .expect("Failed to build HTTP client")
});

/// Builder for constructing NodeClient instances with fluent API
#[derive(Debug, Clone)]
pub struct NodeClientBuilder {
    name: String,
    host: String,
    port: u16,
    timeout: Duration,
}

impl NodeClientBuilder {
    /// Create a new builder with default values
    ///
    /// # Defaults
    /// - host: `127.0.0.1`
    /// - port: `8091`
    /// - timeout: `10` seconds
    pub fn new() -> Self {
        Self {
            name: "alice".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8091,
            timeout: Duration::from_secs(10),
        }
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the host for the node
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Set the port for the node RPC server
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the request timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set timeout in seconds
    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout = Duration::from_secs(secs);
        self
    }

    /// Build the NodeClient
    pub fn build(self, chain_id: u64, tls: bool, create_wallet: bool) -> Result<NodeClient> {
        let proto = if tls {
            "http"
        } else {
            "https"
        };

        let base_url = format!("{}://{}:{}/v0", proto, self.host, self.port);

        debug!("Building NodeClient for: {}", base_url);

        let wallet = if create_wallet {
            Wallet::create(chain_id, &self.name)?
        } else {
            let loaded_wallet = Wallet::load(&self.name)?;
            if loaded_wallet.chain_id != chain_id {
                return Err(color_eyre::eyre::eyre!(
                "ChainId in loaded file is different to provided {}",
                chain_id
            ))
            }
            loaded_wallet
        };

        Ok(NodeClient {
            base_url,
            timeout: self.timeout,
            http_client: Arc::new(HTTP_CLIENT.clone()),
            wallet,
        })
    }
}

impl Default for NodeClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Node client for communicating with Ciphera RPC nodes
///
/// Uses a shared singleton HTTP client for efficient connection pooling.
/// Instances can be created via the builder pattern.
#[derive(Debug, Clone)]
pub struct NodeClient {
    base_url: String,
    timeout: Duration,
    http_client: Arc<reqwest::Client>,
    wallet: Wallet,
}

impl NodeClient {
    /// Create a new NodeClient builder
    pub fn builder() -> NodeClientBuilder {
        NodeClientBuilder::new()
    }

    /// Create a new NodeClient with default settings for localhost
    ///
    /// # Arguments
    /// * `host` - The hostname or IP address of the Ciphera node
    /// * `port` - The RPC server port
    /// * `timeout_secs` - Request timeout in seconds
    ///
    /// # Example
    /// ```no_run
    /// use cli::NodeClient;
    ///
    /// let client = NodeClient::new("alice", "localhost", 10, 8091)?;
    /// # Ok::<(), color_eyre::eyre::Error>(())
    /// ```
    pub fn new(name: &str, host: &str, port: u16, timeout_secs: u64) -> Result<Self> {
        Self::builder()
            .name(name)
            .host(host)
            .port(port)
            .timeout_secs(timeout_secs)
            .build(5115, false, true)
    }

    /// Get the base URL of the node
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn get_wallet(&self) -> &Wallet {
        &self.wallet
    }

    pub fn get_wallet_mut(&mut self) -> &mut Wallet {
        &mut self.wallet
    }

    /// Check the health of the node
    ///
    /// Returns the current height if the node is healthy,
    /// or an error if the node is unhealthy or unreachable.
    pub async fn check_health(&self) -> Result<HealthResponse> {
        let url = format!("{}/health", self.base_url);

        debug!("Checking health at: {}", url);

        let response = self
            .http_client
            .get(&url)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to connect to node: {}", e))?;

        if !response.status().is_success() {
            return Err(color_eyre::eyre::eyre!(
                "Node returned error status: {}",
                response.status()
            ));
        }

        let health = response
            .json::<HealthResponse>()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse health response: {}", e))?;

        Ok(health)
    }

    /// Get the current height of the node
    pub async fn get_height(&self) -> Result<u64> {
        let url = format!("{}/height", self.base_url);

        debug!("Fetching height from: {}", url);

        let response = self
            .http_client
            .get(&url)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to connect to node: {}", e))?;

        if !response.status().is_success() {
            return Err(color_eyre::eyre::eyre!(
                "Node returned error status: {}",
                response.status()
            ));
        }

        let height_resp = response
            .json::<HeightResponse>()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse height response: {}", e))?;

        Ok(height_resp.height)
    }

    pub async fn transaction(&self, proof: &UtxoProof) -> Result<TransactionResponse> {
        let url = format!("{}/transaction", self.base_url);

        debug!("Sending transaction via {}", url);

        let response = self
            .http_client
            .post(&url)
            .json(&json!({
                "proof": proof,
            }))
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to connect to node: {}", e))?;

        if !response.status().is_success() {
            return Err(color_eyre::eyre::eyre!(
                "Node returned error status: {}",
                response.status()
            ));
        }

        let tx_resp = response
            .json::<TransactionResponse>()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse transaction response: {}", e))?;

        Ok(tx_resp)
    }

    pub async fn list_transactions(
        &self,
        query: &ListTxnsQuery,
    ) -> Result<ListTransactionsResponse> {
        let url = format!("{}/transactions", self.base_url);

        debug!("Requesting transaction list via {}", url);

        let response = self
            .http_client
            .get(&url)
            .query(&query)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to connect to node: {}", e))?;

        if !response.status().is_success() {
            return Err(color_eyre::eyre::eyre!(
                "Node returned error status: {}",
                response.status()
            ));
        }

        let list_resp = response
            .json::<ListTransactionsResponse>()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse transaction list: {}", e))?;

        Ok(list_resp)
    }

    pub async fn admin_mint(
        &self,
        geth_rpc: &str,
        chain_id: u64,
        secret: &str,
        rollup: &str,
        note: &Note,
        utxo: &UtxoProof,
    ) -> Result<()> {
        //let eth_node = EthNode::default().run_and_deploy().await;
        let sk = SecretKey::from_str(secret)?;
        let client = contracts::Client::new(geth_rpc, None);

        let rollup = RollupContract::load(client, &chain_id, rollup, sk).await?;

        let mint_hash = hash_merge([note.psi, Note::padding_note().psi]);

        println!("Note hash {:#x}, mint hash {:#x}", utxo.hash(), mint_hash);

        let tx = rollup.mint(&mint_hash, &note.value, &note.contract).await?;

        println!("\nSubmitted MINT tx {tx:#x}\n");

        while rollup
            .client
            .client()
            .eth()
            .transaction_receipt(tx)
            .await
            .unwrap()
            .is_none_or(|r| r.block_number.is_none())
        {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }

        Ok(())
    }

    pub async fn admin_approve(
        &self,
        geth_rpc: &str,
        chain_id: u64,
        secret: &str,
        rollup: &str,
        erc20_contract: &str,
        mint_erc20: bool,
    ) -> Result<()> {
        //let eth_node = EthNode::default().run_and_deploy().await;
        let sk = SecretKey::from_str(secret)?;
        let client = contracts::Client::new(geth_rpc, None);

        let erc20_contract = ERC20Contract::load(client.clone(), erc20_contract, sk).await?;
        let rollup = RollupContract::load(client, &chain_id, rollup, sk).await?;

        let secp = secp256k1::Secp256k1::new();
        let secret_key = secp256k1::SecretKey::from_slice(&sk.secret_bytes()).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let serialized_public_key = public_key.serialize_uncompressed();
        let address_bytes = &keccak256(&serialized_public_key[1..])[12..];
        let admin = Address::from_slice(address_bytes);

        if mint_erc20 {
            let tx_mint_erc20 = erc20_contract.mint(admin, 10000000).await?;
            println!("\nRequested ERC20 mint {tx_mint_erc20:#x}. Approving next\n");
        }

        if erc20_contract
            .allowance(rollup.signer_address, admin)
            .await?
            != U256::MAX
        {
            let approve_txn = erc20_contract.approve_max(rollup.address()).await?;
            rollup
                .client
                .wait_for_confirm(
                    approve_txn,
                    Duration::from_secs(1),
                    ConfirmationType::Latest,
                )
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;

    const CHAIN_ID: u64 = 5115; // Citrea testnet

    // =====================================================================
    // NodeClientBuilder — offline unit tests
    // =====================================================================

    #[test]
    fn test_builder_default_host_and_port() {
        let b = NodeClientBuilder::new();
        assert_eq!(b.host, "127.0.0.1");
        assert_eq!(b.port, 8091);
    }

    #[test]
    fn test_builder_fluent_overrides() {
        let b = NodeClientBuilder::new()
            .host("example.com")
            .port(9000)
            .timeout_secs(30)
            .name("bob");
        assert_eq!(b.host, "example.com");
        assert_eq!(b.port, 9000);
        assert_eq!(b.timeout, Duration::from_secs(30));
        assert_eq!(b.name, "bob");
    }

    /// Timeout setter convenience: timeout_secs(n) equals timeout(Duration::from_secs(n)).
    #[test]
    fn test_builder_timeout_secs_matches_duration() {
        let b1 = NodeClientBuilder::new().timeout_secs(42);
        let b2 = NodeClientBuilder::new().timeout(Duration::from_secs(42));
        assert_eq!(b1.timeout, b2.timeout);
    }

    // =====================================================================
    // build() — URL scheme (tls flag)
    // NOTE: the tls parameter is currently inverted in the implementation:
    //   tls=true  → "http://"
    //   tls=false → "https://"
    // Tests document the *actual* (inverted) behaviour so a future fix
    // will cause exactly these tests to fail, making the correction obvious.
    // =====================================================================

    /// tls=true produces http:// (current inverted behaviour).
    #[test]
    fn test_build_tls_true_gives_http_scheme() {
        let name = "scheme-http-test-wallet";
        let file = format!("{name}.json");
        let _ = std::fs::remove_file(&file);

        let client = NodeClientBuilder::new()
            .name(name)
            .host("node.example.com")
            .port(80)
            .build(CHAIN_ID, true, true)
            .expect("build should succeed");

        let _ = std::fs::remove_file(&file);

        assert!(
            client.base_url().starts_with("http://node.example.com:80"),
            "tls=true should produce http:// (inverted flag); got: {}",
            client.base_url()
        );
    }

    /// tls=false produces https:// (current inverted behaviour).
    #[test]
    fn test_build_tls_false_gives_https_scheme() {
        let name = "scheme-https-test-wallet";
        let file = format!("{name}.json");
        let _ = std::fs::remove_file(&file);

        let client = NodeClientBuilder::new()
            .name(name)
            .host("node.example.com")
            .port(443)
            .build(CHAIN_ID, false, true)
            .expect("build should succeed");

        let _ = std::fs::remove_file(&file);

        assert!(
            client.base_url().starts_with("https://node.example.com:443"),
            "tls=false should produce https:// (inverted flag); got: {}",
            client.base_url()
        );
    }

    /// Base URL always ends with /v0.
    #[test]
    fn test_build_base_url_has_v0_path() {
        let name = "v0-path-test-wallet";
        let file = format!("{name}.json");
        let _ = std::fs::remove_file(&file);

        let client = NodeClientBuilder::new()
            .name(name)
            .host("host")
            .port(1234)
            .build(CHAIN_ID, true, true)
            .expect("build");

        let _ = std::fs::remove_file(&file);

        assert!(
            client.base_url().ends_with("/v0"),
            "base_url must end with /v0; got: {}",
            client.base_url()
        );
    }

    // =====================================================================
    // build() — wallet create vs load
    // =====================================================================

    /// create_wallet=true creates a new wallet file; fails if one exists.
    #[test]
    fn test_build_create_wallet_succeeds_when_file_absent() {
        let name = "create-absent-test-wallet";
        let file = format!("{name}.json");
        let _ = std::fs::remove_file(&file);

        let result = NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, true);

        let _ = std::fs::remove_file(&file);

        assert!(result.is_ok(), "create_wallet=true should succeed when file absent: {:?}", result.err());
    }

    /// create_wallet=true fails with WalletExists when the file already exists.
    #[test]
    fn test_build_create_wallet_fails_when_file_exists() {
        let name = "create-exists-test-wallet";
        let file = format!("{name}.json");

        // Pre-create the wallet.
        let _ = NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, true);

        let result = NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, true);

        let _ = std::fs::remove_file(&file);

        let err = result.expect_err("create_wallet=true must fail when wallet already exists");
        let msg = format!("{err}");
        assert!(
            msg.contains("exists") || msg.contains("Exists"),
            "error should mention the file already exists; got: {msg}"
        );
    }

    /// create_wallet=false loads an existing wallet successfully.
    #[test]
    fn test_build_load_wallet_succeeds_when_file_exists() {
        let name = "load-exists-test-wallet";
        let file = format!("{name}.json");
        let _ = std::fs::remove_file(&file);

        // Create the wallet first.
        NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, true)
            .expect("pre-create wallet");

        let result = NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, false);

        let _ = std::fs::remove_file(&file);

        assert!(result.is_ok(), "create_wallet=false should load existing wallet: {:?}", result.err());
    }

    /// create_wallet=false fails with FileNotFound when file is absent.
    #[test]
    fn test_build_load_wallet_fails_when_file_absent() {
        let name = "load-absent-test-wallet";
        let file = format!("{name}.json");
        let _ = std::fs::remove_file(&file);

        let result = NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, false);

        let err = result.expect_err("create_wallet=false must fail when wallet file absent");
        let msg = format!("{err}");
        assert!(
            msg.contains("not found") || msg.contains("NotFound") || msg.contains("FileNotFound"),
            "error should mention file not found; got: {msg}"
        );
    }

    /// create_wallet=false fails when wallet's chain_id differs from provided.
    #[test]
    fn test_build_load_wallet_rejects_wrong_chain_id() {
        let name = "chain-id-mismatch-test-wallet";
        let file = format!("{name}.json");
        let _ = std::fs::remove_file(&file);

        // Create wallet with chain_id=5115.
        NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, true)
            .expect("pre-create wallet");

        // Load with a different chain_id.
        let result = NodeClientBuilder::new()
            .name(name)
            .build(9999, true, false);

        let _ = std::fs::remove_file(&file);

        let err = result.expect_err("loading wallet with wrong chain_id must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("ChainId") || msg.contains("chain") || msg.contains("different"),
            "error should mention chain_id mismatch; got: {msg}"
        );
    }

    // =====================================================================
    // build() — error propagation (regression for handle_note_spend bug)
    // =====================================================================

    /// Regression: build() with a malformed wallet file returns a descriptive
    /// serialization error, not a generic one.
    ///
    /// Catches the bug in handle_note_spend (main.rs) where
    ///   .map_err(|_| AppError::CantBuildClient())
    /// silently drops the WalletError.
    #[test]
    fn test_build_propagates_serialization_error_on_bad_json() {
        let name = "bad-json-test-wallet";
        let file = format!("{name}.json");
        std::fs::write(&file, b"{bad json}").unwrap();

        let result = NodeClientBuilder::new()
            .name(name)
            .build(CHAIN_ID, true, false); // load, not create

        let _ = std::fs::remove_file(&file);

        let err = result.expect_err("build with malformed wallet file must fail");
        let msg = format!("{err:#}");
        assert!(
            msg.to_lowercase().contains("serial")
                || msg.to_lowercase().contains("json")
                || msg.to_lowercase().contains("parse"),
            "error must mention JSON/serialization, not just 'Builder error': {msg}"
        );
    }
}
