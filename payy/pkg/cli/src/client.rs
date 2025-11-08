//! Node client for communicating with Pay RPC nodes
//!
//! Uses a builder pattern with a singleton HTTP client for efficient
//! connection pooling and session reuse across multiple requests.

use serde_json::json;
use std::fs;
use color_eyre::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::str::FromStr;
use tracing::debug;
use element::Element;
use crate::wallet::Wallet;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HealthResponse {
    pub height: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeightResponse {
    pub height: u64,
}

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

        let file = format!("{}.json", self.name);
        let keyfile_path = Path::new(&file);

        let wallet = if keyfile_path.is_file() {
            println!("\n🗝 Found keyfile!");
            let json_str = fs::read_to_string(&keyfile_path)?;
            let json: serde_json::Value = serde_json::from_str(&json_str)?;

            let pk_hex = json["pk"]
                .as_str()
                .or_else(|| json["public_key"].as_str())
                .or_else(|| json["public_key"]["value"].as_str())
                .ok_or_else(|| color_eyre::eyre::eyre!("PK not found in JSON"))?;

            Wallet::new(Element::from_str(pk_hex)?)
        } else {
            println!("\n🎲 Keyfile not found. Generating new secret!");
            let w = Wallet::random();
            let json_data = json!({
                "pk": w.pk.to_string(),
                "format": "Element hex representation"
            });

            // Write to file
            let json_string = serde_json::to_string_pretty(&json_data)?;
            fs::write(&keyfile_path, json_string)?;

            debug!("Public key dumped to: {}", keyfile_path.display());
            w
        };

        Ok(NodeClient {
            base_url,
            timeout: self.timeout,
            http_client: Arc::new(HTTP_CLIENT.clone()),
            wallet: Arc::new(wallet),
        })
    }
}

impl Default for NodeClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Node client for communicating with Pay RPC nodes
///
/// Uses a shared singleton HTTP client for efficient connection pooling.
/// Instances can be created via the builder pattern.
#[derive(Debug, Clone)]
pub struct NodeClient {
    base_url: String,
    timeout: Duration,
    http_client: Arc<reqwest::Client>,
    wallet: Arc<Wallet>
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
    /// * `host` - The hostname or IP address of the Pay node
    /// * `port` - The RPC server port
    /// * `timeout_secs` - Request timeout in seconds
    ///
    /// # Example
    /// ```no_run
    /// use cli::NodeClient;
    ///
    /// let client = NodeClient::new("localhost", 8091, 10)?;
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

        #[derive(Deserialize)]
        struct HeightResponseInner {
            height: u64,
        }

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
            .json::<HeightResponseInner>()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse height response: {}", e))?;

        Ok(height_resp.height)
    }
}
