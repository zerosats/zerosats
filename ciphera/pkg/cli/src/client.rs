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
use web3::types::{Address, H256};

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
    pub fn build(self) -> Result<NodeClient> {
        let base_url = format!("http://{}:{}/v0", self.host, self.port);
        debug!("Building NodeClient for: {}", base_url);

        let wallet = Wallet::init(&self.name)?;

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
    ///
    /// # Example
    /// ```no_run
    /// use cli::NodeClient;
    ///
    /// let client = NodeClient::builder()
    ///     .host("localhost")
    ///     .port(8091)
    ///     .timeout_secs(10)
    ///     .build()?;
    /// # Ok::<(), color_eyre::eyre::Error>(())
    /// ```
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
            .build()
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
        erc20_contract: &str,
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
        note: &Note,
        utxo: &UtxoProof,
    ) -> Result<()> {
        //let eth_node = EthNode::default().run_and_deploy().await;
        let sk = SecretKey::from_str(secret)?;
        let client = contracts::Client::new(geth_rpc, None);

        let erc20_contract =
            ERC20Contract::load(client.clone(), erc20_contract, sk).await?;
        let rollup = RollupContract::load(client, &chain_id, rollup, sk).await?;

        let secp = secp256k1::Secp256k1::new();
        let secret_key = secp256k1::SecretKey::from_slice(&sk.secret_bytes()).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let serialized_public_key = public_key.serialize_uncompressed();
        let address_bytes = &keccak256(&serialized_public_key[1..])[12..];
        let admin = Address::from_slice(address_bytes);

        if mint_erc20 {
            let tx_mint_erc20 = erc20_contract.mint(admin, 10000000).await?;
            println!(
                "\nRequested ERC20 mint {tx_mint_erc20:#x}. Approving next\n"
            );
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

    pub async fn state(
        &self,
        geth_rpc: &str,
        chain_id: u64,
        secret: &str,
        rollup: &str,
    ) -> Result<()> {
        let sk = SecretKey::from_str(secret)?;
        let client = contracts::Client::new(geth_rpc, None);
        let rollup = RollupContract::load(client, &chain_id, rollup, sk).await?;
        let rh = rollup.root_hash().await?;
        let b = rollup.block_height().await?;
        let kind_wcbtc = H256::from_slice(
            &hex::decode("000200000000000013fb8d0c9d1c17ae5e40fff9be350f57840e9e66cd930000")
                .unwrap(),
        );

        let kind_usdc = H256::from_slice(
            &hex::decode("000200000000000013fb52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b60000")
                .unwrap(),
        );

        let token_wbtc = rollup.token(kind_wcbtc).await?;
        let token_usdc = rollup.token(kind_usdc).await?;

        println!("\nRollup State Info\n");
        println!("\tChain                :{chain_id} ");
        println!("\tToken kind WBTC      :{token_wbtc:#x} ");
        println!("\tToken kind USDC      :{token_usdc:#x} ");

        println!("\tBlock                :{b} ");
        println!("\tRoot hash            :{rh:#x} ");

        Ok(())
    }
}
