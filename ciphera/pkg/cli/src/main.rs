use clap::{Parser, Subcommand};
use cli::NodeClient;
use cli::Wallet;
use cli::address::citrea_ticker_from_contract;
use cli::address::decode_address;
use cli::note_url::{CipheraURL, decode_url};

use color_eyre::Result;
use tracing::{debug, error};
use web3::types::{H160, H256};

use barretenberg::Prove;
use contracts::util::convert_h160_to_element;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use zk_primitives::{InputNote, Note};

#[derive(Parser, Debug)]
#[command(name = "ciphera-cli")]
#[command(about = "Ciphera Network CLI - Connect to and interact with Ciphera nodes", long_about = None)]
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
    #[arg(global = true, long, default_value = "ciphera.satsbridge.com")]
    host: String,

    /// RPC server port
    #[arg(global = true, short, long, default_value = "443")]
    port: u16,

    /// Request timeout in seconds
    #[arg(global = true, long, default_value = "10")]
    timeout: u64,

    /// Use plain HTTP instead of HTTPS when talking to the Ciphera node
    #[arg(global = true, long, action = clap::ArgAction::SetTrue)]
    no_tls: bool,

    #[arg(global = true, short, long, default_value = "5115")] // Citrea testnet default
    chain: u64,

    #[arg(
        global = true,
        long,
        default_value = "0xbd57b7d47d66934509f9ca31248598eb6cb3fafd"
    )]
    rollup: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Create {},
    /// Connect to a Ciphera node and check its health
    Sync {},
    Address {
        #[arg(required = true, short, long)]
        amount: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,
    },
    Mint {
        #[arg(required = true, long, short)]
        geth_rpc: String,

        #[arg(required = true, long, short)]
        secret: String,

        #[arg(required = true, short, long)]
        amount: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,

        #[arg(required = false, long, short, action=clap::ArgAction::SetTrue)]
        only_snark: bool,
    },
    Burn {
        #[arg(required = true, long)]
        address: String,

        #[arg(required = true, short, long)]
        amount: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,
    },
    Spend {
        /// Request amount to spend
        #[arg(required = true, short, long)]
        amount: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,
    },
    SpendTo {
        #[arg(required = true, long)]
        address: String,
    },
    Receive {
        #[arg(long, default_value = None)]
        note: Option<String>,

        #[arg(long)]
        link: Option<String>,
    },
    Import {
        #[arg(required = true, long)]
        note: String,
    },
    Contract {
        #[arg(required = true, long, short)]
        geth_rpc: String,
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
    #[error("Cant parse address: {0}")]
    InvalidAddress(String),
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Not enough balance")]
    NotEnoughBalance(),
    #[error("Feature is not implemented")]
    NotSupportedYet(),
    #[error("Cant convert Element")]
    ConversionError(),
    #[error("Wallet load error: {0}")]
    WalletLoadError(#[from] color_eyre::Report),
}

async fn handle_create(chain: u64, name: &str) -> Result<(), AppError> {
    let wallet_file = format!("{name}.json");

    // Check if wallet already exists
    if Path::new(&wallet_file).exists() {
        println!("\n⚠️  Wallet '{name}' already exists!");
        println!("   Location: {wallet_file}");
    };

    let wallet = Wallet::create(chain, name)?;

    println!("\n✅ Wallet created successfully!");
    println!("\n📋 Wallet Details:");
    println!("   Name: {name}");
    println!("   File: {wallet_file}");
    println!("   Private Key: {}", wallet.pk);
    println!("   Balance: {} sats", wallet.balance);

    println!("\n⚠️  IMPORTANT: Keep your private key safe!");
    println!("   Your private key is stored in {wallet_file}");
    println!("   Never share it with anyone.");

    println!("\n🚀 Next Steps:");
    println!("   1. Connect to network:  ciphera-cli --name {name} sync");
    println!(
        "   2. Mint tokens:         ciphera-cli --name {name} mint --amount <AMOUNT> --secret <YOUR_ETH_KEY> --geth-rpc <RPC_URL>"
    );
    println!("   3. Check balance:       cat {wallet_file}");

    Ok(())
}

/// Handle the connect command
///
/// Connects to a Ciphera node and performs health checks
async fn handle_sync(
    chain: u64,
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    use_tls: bool,
) -> Result<()> {
    debug!(
        "Connecting wallet {} to Ciphera node at {}:{}",
        name, host, port
    );

    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, use_tls, false)?;

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

    match client.list_transactions(&Default::default()).await {
        Ok(list) => {
            println!("   Obtained transactions list size: {}", list.txns.len());
            let (mut synced_wallet, _) = client.get_wallet().prepare_sync(&list.txns)?;
            synced_wallet.chain_id = Some(chain);
            synced_wallet.save()?;
            client.replace_wallet(synced_wallet);
        }
        Err(e) => {
            eprintln!("   Warning: Could not obtain transactions: {e}");
            tracing::warn!("Failed to request transactions: {}", e);
        }
    }

    println!("\n✨ Successfully connected to Ciphera node at {host}:{port}");
    Ok(())
}

async fn handle_address(name: &str, amount: u64, ticker: &str) -> Result<()> {
    let wallet = Wallet::load(name)?;
    let b = wallet.balance;
    let (wallet, a) = wallet.prepare_get_address(amount, ticker);
    wallet.save()?;

    println!("\nWallet {name} has been found:");
    println!("\tBalance: {b:?}");
    println!("\tAddress: {a:?}");

    let encoded = a.encode_address();
    println!("\nEncoded: {encoded}");
    let id = a.commitment();
    println!("\nCommitment: {id}");
    Ok(())
}

async fn handle_note_spend(
    chain: u64,
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    use_tls: bool,
    amount: u64,
    ticker: &str,
) -> Result<()> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, use_tls, false)?;

    // Prepare transfer. A case, when wallet already has exactly matching note, will be ignored
    let (wallet_with_transfer_note, transfer_note) =
        client.get_wallet().prepare_receive_note(amount, ticker);
    let (prepared_wallet, transfer_utxo) =
        wallet_with_transfer_note.prepare_spend_to(&transfer_note.note)?;
    let snark = transfer_utxo.prove().unwrap();

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            let payload: CipheraURL = (&transfer_note).into();

            // Encode
            let encoded = payload.encode_url();
            let json_str = serde_json::to_string_pretty(&transfer_note)?;

            std::fs::write(format!("{name}-note.json"), &json_str)?;

            println!("\nSaved {transfer_note:?}");
            println!("\nEncoded: {encoded}");

            let b = client.get_wallet().balance;
            println!("\nBalance {b} {ticker}");
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

async fn handle_spend_to(
    chain: u64,
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    use_tls: bool,
    address: &str,
) -> Result<()> {
    debug!(
        "Connecting wallet {} to Ciphera node at {}:{}",
        name, host, port
    );

    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, use_tls, false)?;

    // Spend to UX leverages a variant of NoteURL encoding for providing an "address" with address
    // and amount needed for UTXO construction
    let note = Note::from(&decode_address(address));
    let ticker = citrea_ticker_from_contract(note.contract);

    let (prepared_wallet, utxo) = client.get_wallet().prepare_spend_to(&note)?;
    let snark = utxo.prove().unwrap();

    let recipient_note = utxo.output_notes[0].clone();

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            let json_str = serde_json::to_string_pretty(&recipient_note)?;
            std::fs::write(format!("from-{name}-note.json"), &json_str)?;

            println!("\nSaved {recipient_note:?}");

            let b = client.get_wallet().balance;
            println!("\nBalance {b} {ticker}");
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

async fn handle_receive(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    chain: u64,
    use_tls: bool,
    notefile: Option<String>,
    notelink: Option<String>,
) -> Result<()> {
    debug!(
        "Connecting wallet {} to Ciphera node at {}:{}",
        name, host, port
    );

    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, use_tls, false)?;

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

    println!("\n✨ Successfully connected to Ciphera node at {host}:{port}");

    let input_note = match (notefile, notelink) {
        (Some(path), None) => {
            let notefile_path = Path::new(&path);
            if notefile_path.is_file() {
                println!("\n🗝 Found note file!");
                let json_str = fs::read_to_string(notefile_path)?;
                let input_note: InputNote = serde_json::from_str(&json_str)?;
                input_note
            } else {
                return Err(AppError::FileNotFound(path.to_owned()).into());
            }
        }
        (None, Some(link)) => {
            let input_note = InputNote::from(&decode_url(&link));
            println!("\n🗝 Decoded note: {input_note:?}");
            input_note
        }
        _ => return Err(AppError::NotEnoughBalance().into()),
    };

    let ticker = citrea_ticker_from_contract(input_note.note.contract);

    let (prepared_wallet, utxo) = client.get_wallet().prepare_receive(&input_note)?;
    let snark = utxo.prove().unwrap();

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            let b = client.get_wallet().balance;
            println!("\nBalance {b} {ticker}");
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

async fn handle_import(name: &str, notefile: &str) -> Result<()> {
    let json_path = Path::new(&notefile);
    if json_path.is_file() {
        println!("\n🗝 Found note file!");
        let json_str = fs::read_to_string(json_path)?;
        let note: Note = serde_json::from_str(&json_str)?;

        let wallet = Wallet::load(name)?;
        let (wallet, _) = wallet.prepare_import_note(&note)?;
        wallet.save()?;
        Ok(())
    } else {
        Err(AppError::FileNotFound(notefile.to_owned()).into())
    }
}

async fn handle_mint(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    use_tls: bool,
    geth_rpc: &str,
    chain: u64,
    rollup: &str,
    secret: &str,
    amount: u64,
    ticker: &str,
    only_snark: bool,
) -> Result<()> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, use_tls, false)?;

    let (prepared_wallet, utxo) = client.get_wallet().prepare_mint(amount, ticker)?;
    let snark = utxo.prove().unwrap();

    if !only_snark {
        client
            .admin_mint(
                geth_rpc,
                chain,
                secret,
                rollup,
                &utxo.output_notes[0],
                &snark,
            )
            .await?;
    }

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            let b = client.get_wallet().balance;
            println!("\nBalance {b} {ticker}");
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
    chain: u64,
    use_tls: bool,
    address: &str,
    amount: u64,
    ticker: &str,
) -> Result<(), AppError> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, use_tls, false)?;

    // Prepare burn
    let (wallet_with_burner_key, burner_note) =
        client.get_wallet().prepare_receive_note(amount, ticker);
    let (wallet_after_burner_transfer, burner_utxo) =
        wallet_with_burner_key.prepare_spend_to(&burner_note.note)?;
    let snark = burner_utxo.prove().unwrap();

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            wallet_after_burner_transfer.save()?;
            client.replace_wallet(wallet_after_burner_transfer);
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            return Err(AppError::WalletLoadError(e));
        }
    }

    let (wallet_with_burner_note, _) =
        client.get_wallet().prepare_import_note(&burner_note.note)?;
    wallet_with_burner_note.save()?;
    client.replace_wallet(wallet_with_burner_note);

    let evm_address = match H160::from_str(address) {
        Ok(a) => convert_h160_to_element(&a),
        Err(e) => return Err(AppError::InvalidAddress(e.to_string())),
    };

    let (wallet_after_burn, burner_utxo) = client
        .get_wallet()
        .prepare_burn(&burner_note, &evm_address)?;
    let snark = burner_utxo.prove().unwrap();

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);
            wallet_after_burn.save()?;
            client.replace_wallet(wallet_after_burn);
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            return Err(AppError::WalletLoadError(e));
        }
    }
    Ok(())
}

async fn handle_rollup(geth_rpc: &str, chain: u64, rollup: &str) -> Result<()> {
    let client = contracts::Client::new(geth_rpc, None);
    let rollup = contracts::ReadonlyRollupContract::load(client, rollup).await?;

    let rh = rollup.root_hash().await?;
    let b = rollup.block_height().await?;
    let kind_wcbtc = H256::from_slice(
        &hex::decode("000200000000000013fb8d0c9d1c17ae5e40fff9be350f57840e9e66cd930000").unwrap(),
    );

    let kind_usdc = H256::from_slice(
        &hex::decode("000200000000000013fb52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b60000").unwrap(),
    );

    let token_wbtc = rollup.token(kind_wcbtc).await?;
    let token_usdc = rollup.token(kind_usdc).await?;

    println!("\nRollup State Info\n");
    println!("\tChain                :{chain} ");
    println!("\tToken kind WBTC      :{token_wbtc:#x} ");
    println!("\tToken kind USDC      :{token_usdc:#x} ");

    println!("\tBlock                :{b} ");
    println!("\tRoot hash            :{rh:#x} ");

    Ok(())
}

/// Initialize logging based on verbosity level
fn init_logging(verbose: bool) {
    let log_level = if verbose { "debug" } else { "error" };

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
    let use_tls = !cli.no_tls;

    debug!("Starting Ciphera CLI");

    // Execute command
    match cli.command {
        Commands::Create {} => {
            handle_create(cli.chain, &cli.name).await?;
        }
        Commands::Sync {} => {
            handle_sync(
                cli.chain,
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                use_tls,
            )
            .await?;
        }
        Commands::Address { amount, ticker } => {
            let ticker_normalized = ticker.to_uppercase();
            handle_address(&cli.name, amount, &ticker_normalized).await?;
        }
        Commands::Spend { amount, ticker } => {
            let ticker_normalized = ticker.to_uppercase();
            handle_note_spend(
                cli.chain,
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                use_tls,
                amount,
                &ticker_normalized,
            )
            .await?;
        }
        Commands::SpendTo { address } => {
            handle_spend_to(
                cli.chain,
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                use_tls,
                &address,
            )
            .await?;
        }
        Commands::Receive { note, link } => {
            handle_receive(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                cli.chain,
                use_tls,
                note,
                link,
            )
            .await?;
        }
        Commands::Import { note } => {
            handle_import(&cli.name, &note).await?;
        }
        Commands::Mint {
            geth_rpc,
            secret,
            amount,
            ticker,
            only_snark,
        } => {
            let ticker_normalized = ticker.to_uppercase();
            handle_mint(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                use_tls,
                &geth_rpc,
                cli.chain,
                &cli.rollup,
                &secret,
                amount,
                &ticker_normalized,
                only_snark,
            )
            .await?;
        }
        Commands::Burn {
            address,
            amount,
            ticker,
        } => {
            let ticker_normalized = ticker.to_uppercase();
            handle_burn(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                cli.chain,
                use_tls,
                &address,
                amount,
                &ticker_normalized,
            )
            .await?;
        }
        Commands::Contract { geth_rpc } => {
            handle_rollup(&geth_rpc, cli.chain, &cli.rollup).await?;
        }
    }

    println!("\n");
    Ok(())
}
