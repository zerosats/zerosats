use clap::{Parser, Subcommand};
use color_eyre::Result;
use tracing::{error, debug};
use web3::types::H160;

use cli::NodeClient;
use cli::Wallet;

use std::fs;
use std::path::Path;
use zk_primitives::{Utxo, Note};
use zk_primitives::InputNote;
use element::Element;
use barretenberg::Prove;

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

    /// RPC server host
    #[arg(global = true, long, default_value = "127.0.0.1")]
    host: String,

    /// RPC server port
    #[arg(global = true, short, long, default_value = "8091")]
    port: u16,

    /// Request timeout in seconds
    #[arg(global = true, short, long, default_value = "10")]
    timeout: u64,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Connect to a Pay node and check its health
    Connect {},
    Mint {
        #[arg(required = true, long, short)]
        geth_rpc: String,

        #[arg(required = true, long, short)]
        secret: String,

        #[arg(required = true, short, long)]
        amount: u64,

        #[arg(required = true, long, short)]
        recipient: String,
    },
    Spend {
        /// Request amount to spend
        #[arg(required = true, short, long)]
        amount: u64,
    },
    Send {
        #[arg(long, default_value = "note.json")]
        note: String,
    },
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("File not found: {0}")]
    FileNotFound(String),
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

async fn handle_spend(name: &str, amount: u64) -> Result<()> {
    // Build client with fluent API
    let client = NodeClient::builder()
        .name(name)
        .build()?;

    // Check health
    let chain = 5115 as u64; // Citrea chain

    let token =
        H160::from_slice(&hex::decode("52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6").unwrap()); // Token Contract

    let note = client.get_wallet().new_input_note(amount, chain, token);
    let json_str = serde_json::to_string_pretty(&note)?;

    std::fs::write("note.json", &json_str)?;

    println!("\nSaved {:?}", note);

    Ok(())
}

async fn handle_send(name: &str, host: &str, port: u16, timeout_secs: u64, note: &str) -> Result<()> {
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

    let file = format!("{}", note);
    let notefile_path = Path::new(&file);

    if notefile_path.is_file() {
        println!("\n🗝 Found note file!");
        let json_str = fs::read_to_string(&notefile_path)?;
        let json: serde_json::Value = serde_json::from_str(&json_str)?;
        let input_note: InputNote = serde_json::from_str(&json_str)?;

        // Check health
        let chain = 5115 as u64; // Citrea chain        let token =
            H160::from_slice(&hex::decode("52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6").unwrap()); // Token Contract


        let note = Note {
            kind: input_note.note.kind,
            contract: input_note.note.contract,
            address: client.get_wallet().address(),
            psi: Element::new(0),
            value: input_note.note.value,
        };

        let utxo = Utxo::new_send(
            [input_note.clone(), InputNote::padding_note()],
            [note, Note::padding_note()],
        );

        let snark = utxo.prove().unwrap();

        match client.transaction(&snark).await {
            Ok(tx) => {
                println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
                println!("   Height: {}", tx.height);
                println!("   Root hash: {}", tx.root_hash);
                Ok(())
            }
            Err(e) => {
                eprintln!("\n❌ Could not send transaction!");
                return Err(e);
            }
        }
    } else {
        Err(AppError::FileNotFound(note.to_owned()).into())
    }
}

async fn handle_mint(name: &str, geth_rpc: &str, secret: &str, amount: u64, recipient: &str) -> Result<()> {
    // Build client with fluent API
    let client = NodeClient::builder()
        .name(name)
        .build()?;

    // Check health
    let chain = 5115 as u64; // Citrea chain
    let token =
        H160::from_slice(&hex::decode("52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6").unwrap()); // Token Contract
    let rollup = "b26db42b0cb837010752d7c371ec727141045438";


    client.admin_mint(geth_rpc, chain, secret, rollup, token, amount).await?;

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
        Commands::Connect {} => {
            handle_connect(&cli.name, &cli.host, cli.port, cli.timeout).await?;
        }
        Commands::Spend { amount } => {
            handle_spend(&cli.name, amount).await?;
        }
        Commands::Send { note } => {
            handle_send(&cli.name, &cli.host, cli.port, cli.timeout, &note).await?;
        }
        Commands::Mint { geth_rpc, secret, amount, recipient } => {
            handle_mint(&cli.name, &geth_rpc, &secret, amount, &recipient).await?;
        }
    }

    println!("\n");
    Ok(())
}