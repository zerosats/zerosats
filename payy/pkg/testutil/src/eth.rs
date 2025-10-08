use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
};

use once_cell::sync::Lazy;

use crate::PortPool;

#[derive(Debug)]
pub struct EthNode {
    process: Option<std::process::Child>,
    port: u16,
    options: EthNodeOptions,
}

impl Drop for EthNode {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, Default)]
pub struct EthNodeOptions {
    pub use_noop_verifier: bool,
    pub use_deployer_as_pool_rollup: bool,
    pub validators: Option<Vec<String>>,
}

impl Default for EthNode {
    fn default() -> Self {
        Self::new(EthNodeOptions::default())
    }
}

impl EthNode {
    pub fn new(options: EthNodeOptions) -> Self {
        let port = 12345;

        Self {
            process: None,
            port,
            options,
        }
    }

    pub fn run(&mut self) { }

    fn stop(&mut self) { }

    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    async fn wait_for_healthy(&self) -> Result<(), Box<dyn std::error::Error>> {
        tokio::time::sleep(std::time::Duration::from_secs(6)).await;
        Ok(())
    }

    async fn wait(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.wait_for_healthy().await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        self.wait_for_healthy().await?;
        Ok(())
    }

    pub async fn run_and_deploy(mut self) -> Arc<Self> {
        let eth_node = tokio::task::spawn_blocking(move || {
            self.run();
            self
        })
            .await
            .unwrap();

        eth_node.wait().await.expect("Failed to wait for eth node");

        Arc::new(eth_node)
    }
}
