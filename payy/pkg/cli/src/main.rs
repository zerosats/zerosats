use clap::{Parser, Subcommand};
use color_eyre::Result;
use tracing::{debug, error};
use web3::types::H160;

use cli::NodeClient;
use cli::Wallet;

use barretenberg::Prove;
use contracts::util::convert_h160_to_element;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use zk_primitives::InputNote;
use zk_primitives::{Note, NoteURLPayload, Utxo, decode_activity_url_payload};

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

    #[arg(global = true, short, long, default_value = "5115")] // Citrea testnet default
    chain: u64,

    #[arg(
        global = true,
        long,
        default_value = "0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93"
    )] // WCBTC Testnet
    token: String,

    #[arg(
        global = true,
        long,
        default_value = "0x40f811540041401bd07f37fa45ef2d769c9ca977"
    )]
    rollup: String,
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

        #[arg(required = false, long, short, action=clap::ArgAction::SetTrue)]
        only_snark: bool,
    },
    Burn {
        #[arg(required = true, long, short)]
        geth_rpc: String,

        #[arg(required = true, long, short)]
        secret: String,

        #[arg(required = true, long)]
        address: String,

        #[arg(required = true, short, long)]
        amount: u64,
    },
    Spend {
        /// Request amount to spend
        #[arg(required = true, short, long)]
        amount: u64,
    },
    Receive {
        #[arg(long, default_value = None)]
        note: Option<String>,

        #[arg(long)]
        link: Option<String>,
    },
    Contract {
        #[arg(required = true, long, short)]
        geth_rpc: String,

        #[arg(required = true, long, short)]
        secret: String,
    },
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Builder error")]
    CantBuildClient(),
    #[error("Wallet error: {0}")]
    WalletError(#[from] cli::wallet::WalletError),
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Not enough balance")]
    NotEnoughBalance(),
    #[error("Feature is not implemented")]
    NotSupportedYet(),
}

/// Handle the connect command
///
/// Connects to a Pay node and performs health checks
async fn handle_connect(name: &str, host: &str, port: u16, timeout_secs: u64) -> Result<()> {
    debug!(
        "Connecting wallet {} to Payy node at {}:{}",
        name, host, port
    );

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
            eprintln!("   Error: {e}");
            return Err(e);
        }
    }

    // Also fetch height for confirmation
    match client.get_height().await {
        Ok(height) => {
            println!("   Height (verified): {height}");
        }
        Err(e) => {
            eprintln!("   Warning: Could not verify height: {e}");
            tracing::warn!("Height verification failed: {}", e);
        }
    }

    println!("\n✨ Successfully connected to Pay node at {host}:{port}");
    Ok(())
}

async fn handle_spend(name: &str, amount: u64) -> Result<(), AppError> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .build()
        .map_err(|_| AppError::CantBuildClient())?;

    let wallet = Wallet::init(name)?;
    let balance = client.get_wallet().balance;

    if amount == balance {
        let input_note = client.get_wallet_mut().spend_note()?;
        let payload: NoteURLPayload = (&input_note).into();

        // Encode
        let encoded = payload.encode_activity_url_payload();
        let json_str = serde_json::to_string_pretty(&input_note)?;

        std::fs::write(format!("{name}-note.json"), &json_str)?;

        println!("\nSaved {input_note:?}");
        println!("\nEncoded: {encoded}");

        Ok(())
    } else if amount < balance {
        /*

        let self_address = hash_merge([self.pk, Element::ZERO]);
        let change = Note::new(
            self_address,
            Element::from(1_u64)
        );
        let payee = Note::new(
            //Element::from(address),
            self_address,
            Element::from(1_u64)
        );

        Ok(Utxo::new_send(
            [input_note.clone(), InputNote::padding_note()],
            [payee, change],
        ))

        */

        Err(AppError::NotSupportedYet())
    } else {
        Err(AppError::NotEnoughBalance())
    }
}

async fn handle_receive(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    chain: u64,
    token: &str,
    notefile: Option<String>,
    notelink: Option<String>,
) -> Result<()> {
    debug!(
        "Connecting wallet {} to Payy node at {}:{}",
        name, host, port
    );

    // Build client with fluent API
    let mut client = NodeClient::builder()
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
            eprintln!("   Error: {e}");
            return Err(e);
        }
    }

    // Also fetch height for confirmation
    match client.get_height().await {
        Ok(height) => {
            println!("   Height (verified): {height}");
        }
        Err(e) => {
            eprintln!("   Warning: Could not verify height: {e}");
            tracing::warn!("Height verification failed: {}", e);
        }
    }

    println!("\n✨ Successfully connected to Pay node at {host}:{port}");

    let input_note = match (notefile, notelink) {
        (Some(path), None) => {
            let notefile_path = Path::new(&path);
            if notefile_path.is_file() {
                println!("\n🗝 Found note file!");
                let json_str = fs::read_to_string(notefile_path)?;
                let json: serde_json::Value = serde_json::from_str(&json_str)?;
                let input_note: InputNote = serde_json::from_str(&json_str)?;
                input_note
            } else {
                return Err(AppError::FileNotFound(path.to_owned()).into());
            }
        }
        (None, Some(link)) => {
            let input_note = InputNote::new_from_link(&link);
            println!("\n🗝 Decoded note: {:?}", input_note);
            input_note
        }
        _ => return Err(AppError::NotEnoughBalance().into()),
    };

    let note: Note = client.get_wallet_mut().receive_note(1_u64, chain, token);

    /*
    let note = Note {
        kind: input_note.note.kind,
        contract: input_note.note.contract,
        address: client.get_wallet().address(),
        psi: Element::new(0),
        value: input_note.note.value,
    };
    */

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
            Err(e)
        }
    }
}

async fn handle_mint(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    geth_rpc: &str,
    chain: u64,
    token: &str,
    rollup: &str,
    secret: &str,
    amount: u64,
    only_snark: bool,
) -> Result<()> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build()?;

    let note: Note = client.get_wallet_mut().receive_note(amount, chain, token);
    let output_notes = [note.clone(), Note::padding_note()];
    let utxo = zk_primitives::Utxo::new_mint(output_notes.clone());

    let snark = utxo.prove().unwrap();

    let _ = client.get_wallet().save();

    if !only_snark {
        client
            .admin_mint(geth_rpc, chain, secret, rollup, token, &note, &snark)
            .await?;
    }

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

async fn handle_burn(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    geth_rpc: &str,
    chain: u64,
    token: &str,
    rollup: &str,
    secret: &str,
    address: &str,
    amount: u64,
) -> Result<()> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build()?;

    let note = client.get_wallet_mut().spend_note()?;
    let evm_address = convert_h160_to_element(&H160::from_str(address).unwrap()); // TODO
    let input_notes = [note.clone(), InputNote::padding_note()];
    let utxo = zk_primitives::Utxo::new_burn(input_notes, evm_address);

    let snark = utxo.prove().unwrap();

    let _ = client.get_wallet().save();

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

async fn handle_rollup(geth_rpc: &str, secret: &str, chain: u64, rollup: &str) -> Result<()> {
    // Build client with fluent API
    let client = NodeClient::builder().build()?;
    let _ = client.state(geth_rpc, chain, secret, rollup).await?;

    Ok(())
}

/// Initialize logging based on verbosity level
fn init_logging(verbose: bool) {
    let log_level = if verbose { "debug" } else { "info" };

    tracing_subscriber::fmt().with_env_filter(log_level).init();
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
        Commands::Receive { note, link } => {
            handle_receive(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                cli.chain,
                &cli.token,
                note,
                link,
            )
            .await?;
        }
        Commands::Mint {
            geth_rpc,
            secret,
            amount,
            only_snark,
        } => {
            handle_mint(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                &geth_rpc,
                cli.chain,
                &cli.token,
                &cli.rollup,
                &secret,
                amount,
                only_snark,
            )
            .await?;
        }
        Commands::Burn {
            geth_rpc,
            secret,
            address,
            amount,
        } => {
            handle_burn(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                &geth_rpc,
                cli.chain,
                &cli.token,
                &cli.rollup,
                &secret,
                &address,
                amount,
            )
            .await?;
        }
        Commands::Contract { geth_rpc, secret } => {
            handle_rollup(&geth_rpc, &secret, cli.chain, &cli.rollup).await?;
        }
    }

    println!("\n");
    Ok(())
}
