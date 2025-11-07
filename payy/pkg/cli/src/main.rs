use clap::{Parser, Subcommand};
use color_eyre::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;
use cli::client::{HeightResponse, HealthResponse, NodeClient};

#[derive(Parser, Debug)]
#[command(name = "pay-cli")]
#[command(about = "Pay Network CLI - Connect to and interact with Pay nodes", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(global = true, short, long)]
    verbose: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Connect to a Pay node and check its health
    Connect {
        /// RPC server host
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// RPC server port
        #[arg(short, long, default_value = "8091")]
        port: u16,

        /// Request timeout in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },
}

async fn handle_connect(host: &str, port: u16, timeout: u64) -> Result<()> {
    debug!("Connecting to Pay node at {}:{}", host, port);

    let client = NodeClient::new(host, port, timeout);

    // Check health
    match client.check_health().await {
        Ok(health) => {
            println!("\n✅ Node Health Check Passed!");
            println!("   Current Height: {}", health.height);
        }
        Err(e) => {
            eprintln!("\n❌ Health Check Failed!");
            eprintln!("   Error: {}", e);
            return Err(e);
        }
    }

    // Also fetch height for confirmation
    match client.get_height().await {
        Ok(height) => {
            println!("   Height (verified): {}", height);
            debug!("Height verified: {}", height);
        }
        Err(e) => {
            eprintln!("   Warning: Could not verify height: {}", e);
        }
    }

    println!("\n✨ Successfully connected to Pay node at {}:{}", host, port);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    // Setup logging
    let log_level = if cli.verbose {
        "debug"
    } else {
        "info"
    };

    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .init();

    match cli.command {
        Commands::Connect {
            host,
            port,
            timeout,
        } => {
            handle_connect(&host, port, timeout).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_client_url_formation() {
        let client = NodeClient::new("localhost", 8091, 10);
        assert_eq!(client.base_url, "http://localhost:8091/v0");
    }

    #[test]
    fn test_node_client_custom_host() {
        let client = NodeClient::new("192.168.1.1", 9000, 5);
        assert_eq!(client.base_url, "http://192.168.1.1:9000/v0");
    }
}