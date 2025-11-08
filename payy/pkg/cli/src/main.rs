use clap::{Parser, Subcommand};
use color_eyre::Result;
use tracing::{error, debug};

use cli::NodeClient;
use cli::Wallet;

#[derive(Parser, Debug)]
#[command(name = "pay-cli")]
#[command(about = "Pay Network CLI - Connect to and interact with Pay nodes", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(global = true, short, long)]
    verbose: bool,

    /// Enable verbose logging
    #[arg(global = true, default_value = "alice", short, long)]
    name: String,
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

/// Handle the connect command
///
/// Connects to a Pay node and performs health checks
async fn handle_connect(name: &str, host: &str, port: u16, timeout_secs: u64) -> Result<()> {
    debug!("Connecting wallet {} to Payy node at {}:{}", name, host, port);

    // Build client with fluent API
    let client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build()?;

    // Check health
    match client.check_health().await {
        Ok(health) => {
            println!("\n✅ Node Health Check Passed!");
            println!("   Current Height: {}", health.height);
            debug!("Node is healthy at height: {}", health.height);
        }
        Err(e) => {
            error!("Health check failed: {}", e);
            eprintln!("\n❌ Health Check Failed!");
            eprintln!("   Error: {}", e);
            return Err(e);
        }
    }

    // Also fetch height for confirmation
    match client.get_height().await {
        Ok(height) => {
            println!("   Height (verified): {}", height);
        }
        Err(e) => {
            eprintln!("   Warning: Could not verify height: {}", e);
            tracing::warn!("Height verification failed: {}", e);
        }
    }

    println!("\n✨ Successfully connected to Pay node at {}:{}", host, port);
    Ok(())
}

/// Initialize logging based on verbosity level
fn init_logging(verbose: bool) {
    let log_level = if verbose { "debug" } else { "info" };

    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .init();
}

/// Initialize error handling with color-eyre
fn init_error_handling() -> Result<()> {
    color_eyre::install()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize error handling first
    init_error_handling()?;

    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging
    init_logging(cli.verbose);

    debug!("Starting Pay CLI");

    // Execute command
    match cli.command {
        Commands::Connect {
            host,
            port,
            timeout,
        } => {
            handle_connect(&cli.name, &host, port, timeout).await?;
        }
    }

    println!("\n");
    Ok(())
}