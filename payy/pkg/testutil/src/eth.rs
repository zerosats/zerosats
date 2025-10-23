use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
};

use once_cell::sync::Lazy;

use serde_json::json;
use std::fs;
use std::path::Path;

use crate::PortPool;

fn find_eth() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../citrea");
    path
}

static PORT_POOL: Lazy<Mutex<PortPool>> =
    once_cell::sync::Lazy::new(|| Mutex::new(PortPool::new(12345..12346)));

#[derive(Debug)]
pub struct EthNode {
    process: Option<std::process::Child>,
    port: u16,
    options: EthNodeOptions,
}

impl Drop for EthNode {
    fn drop(&mut self) {
        self.stop();
        PORT_POOL.lock().unwrap().release(self.port);
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
        let port = PORT_POOL.lock().unwrap().get();

        Self {
            process: None,
            port,
            options,
        }
    }

    pub fn run(&mut self) {
        // This must be the actual Citrea dev bin instead of running it through yarn,
        // because we send a SIGKILL which yarn can't forward to the Citrea dev node.
        let mut command = Command::new("/citrea");

        command.current_dir(find_eth());

        command.arg("--dev");
        command.arg("--da-layer").arg("mock");
        command.arg("--rollup-config-path").arg("/configs/mock/sequencer_rollup_config.toml");
        command.arg("--sequencer").arg("/configs/mock/sequencer_config.toml");
        command.arg("--genesis-paths").arg("/genesis/mock/");

        let should_log = std::env::var("LOG_CITREA_OUTPUT")
            .map(|v| v == "1")
            .unwrap_or(false);
        if !should_log {
            command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }

        let process = command.spawn().expect("Failed to start Citrea dev node");
        self.process = Some(process);
    }

    fn stop(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.kill().expect("Failed to kill Citrea dev node");
            process
                .wait()
                .expect("Failed to wait for Citrea dev node to exit");
        }

        let resources_dir = Path::new("/app/citrea/resources");
        if resources_dir.exists() {
            match fs::remove_dir_all(resources_dir) {
                Ok(_) => println!("Successfully removed /app/citrea/resources"),
                Err(e) => println!("Failed to remove /app/citrea/resources: {}", e),
            }
        }
    }

    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    async fn wait_for_healthy(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let time_between_requests = std::time::Duration::from_millis(100);
        let max_retries = 1_000 / time_between_requests.as_millis() as usize;

        let client = reqwest::Client::new();

        for retry in 0..max_retries {
            let is_last_retry = retry == max_retries - 1;

            let req = client
                .post(self.rpc_url())
                .json(&json!({
                "jsonrpc": "2.0",
                "method": "eth_blockNumber",
                "params": [],
                "id": 1
            }))
                .build()?;

            match client.execute(req).await {
                Ok(res) if res.status().is_success() => {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        if let Some(result) = body.get("result").and_then(|r| r.as_str()) {
                            let block_height = u64::from_str_radix(result.trim_start_matches("0x"), 16)?;
                            return Ok(block_height);
                        }
                    }
                }
                Ok(res) if is_last_retry => {
                    return Err(format!("Failed to get block height: {}", res.status()).into());
                }
                Err(err) if is_last_retry => return Err(err.into()),
                _ => {}
            }

            tokio::time::sleep(time_between_requests).await;
        }

        Err("Max retries exceeded".into())
    }

    async fn wait_for_next_block(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let current_block = self.wait_for_healthy().await?;
        loop {
            let block_height = self.wait_for_healthy().await?;
            if block_height > current_block {
                return Ok(block_height);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    fn deploy(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut command = Command::new("node_modules/.bin/hardhat");

        command.current_dir(find_eth());

        command.env(
            "SECRET_KEY",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        );
        command.env("TESTING_URL", self.rpc_url());

        if self.options.use_noop_verifier {
            command.env("DEV_USE_NOOP_VERIFIER", "1");
        }

        if self.options.use_deployer_as_pool_rollup {
            command.env("DEV_USE_DEPLOYER_AS_POOL_ROLLUP", "1");
        }

        if let Some(validators) = &self.options.validators {
            command.env("VALIDATORS", validators.join(","));
        }

        command.arg("run");
        command.arg("scripts/deploy.ts");

        let should_log = std::env::var("LOG_HARDHAT_DEPLOY_OUTPUT")
            .map(|v| v == "1")
            .unwrap_or(false);
        if !should_log {
            command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }

        let mut process = command.spawn().expect("Failed to start Citrea deploy");
        let status = process.wait()?;

        if !status.success() {
            Err("Citrea deploy returned a non-zero exit code".into())
        } else {
            Ok(())
        }
    }

    pub async fn run_and_deploy(mut self) -> Arc<Self> {
        let eth_node = tokio::task::spawn_blocking(move || {
            self.run();
            self
        })
        .await
        .unwrap();

        eth_node.wait_for_next_block().await.expect("Failed to wait for Citrea node");

        let eth_node = tokio::task::spawn_blocking(move || {
            // Deploy is flaky
            for i in 0..3 {
                match eth_node.deploy() {
                    Ok(_) => break,
                    Err(err) => {
                        if i == 2 {
                            panic!("Failed to deploy contracts: {err:?}; Run with LOG_HARDHAT_DEPLOY_OUTPUT=1 to see the output");
                        } else {
                            std::thread::sleep(std::time::Duration::from_secs(5));
                        }
                    }
                }
            }

            eth_node
        })
        .await
        .unwrap();

        Arc::new(eth_node)
    }
}
