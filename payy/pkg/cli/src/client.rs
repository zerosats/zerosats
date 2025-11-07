//! Node client for communicating with Pay RPC nodes

use color_eyre::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HealthResponse {
    pub height: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeightResponse {
    pub height: u64,
}

#[derive(Debug)]
pub struct NodeClient {
    base_url: String,
    timeout: Duration,
}

impl NodeClient {
    /// Create a new NodeClient
    ///
    /// # Arguments
    /// * `host` - The hostname or IP address of the Pay node
    /// * `port` - The RPC server port (default: 8091)
    /// * `timeout_secs` - Request timeout in seconds
    pub fn new(host: &str, port: u16, timeout_secs: u64) -> Self {
        let base_url = format!("http://{}:{}/v0", host, port);
        let timeout = Duration::from_secs(timeout_secs);
        Self { base_url, timeout }
    }

    /// Get the base URL of the node
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub async fn check_health(&self) -> Result<HealthResponse> {
        let url = format!("{}/health", self.base_url);

        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()?;

        debug!("Sending health check request to: {}", url);

        let response = client
            .get(&url)
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

    pub async fn get_height(&self) -> Result<u64> {
        let url = format!("{}/height", self.base_url);

        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()?;

        debug!("Sending height request to: {}", url);

        #[derive(Deserialize)]
        struct HeightResponse {
            height: u64,
        }

        let response = client
            .get(&url)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_client_creation() {
        let client = NodeClient::new("localhost", 8091, 10).unwrap();
        assert_eq!(client.base_url(), "http://localhost:8091/v0");
        assert_eq!(client.timeout(), Duration::from_secs(10));
    }

    #[test]
    fn test_node_client_custom_host() {
        let client = NodeClient::new("192.168.1.1", 9000, 5).unwrap();
        assert_eq!(client.base_url(), "http://192.168.1.1:9000/v0");
    }

    #[test]
    fn test_node_client_default_port() {
        let client = NodeClient::new("127.0.0.1", 8091, 30).unwrap();
        assert!(client.base_url().contains("8091"));
    }
}