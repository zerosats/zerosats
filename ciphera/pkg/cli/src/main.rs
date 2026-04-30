use clap::{Parser, Subcommand};
use cli::NodeClient;
use cli::Wallet;
use cli::address::citrea_ticker_from_contract;
use cli::address::decode_address;
use cli::note_url::{CipheraURL, decode_url};

use color_eyre::Result;
use tracing::{debug, error};
use web3::types::{H160, H256, U256};

use barretenberg::Prove;
use contracts::util::{convert_element_to_h256, convert_h160_to_element};
use hash::hash_merge;
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
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

    /// RPC server host (include scheme for TLS: https://host or http://host)
    #[arg(global = true, long, default_value = "https://ciphera.satsbridge.com")]
    host: String,

    /// RPC server port
    #[arg(global = true, short, long, default_value = "443")]
    port: u16,

    /// Request timeout in seconds
    #[arg(global = true, long, default_value = "10")]
    timeout: u64,

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
    /// List all mint hashes stored in the rollup contract with their values
    Mints {
        #[arg(required = true, long, short)]
        geth_rpc: String,
    },
    /// Deposit via Lightning Network using an external onramp service
    DepoLn {
        /// Amount to deposit in satoshis
        #[arg(required = true, short, long)]
        amount_sat: u64,

        /// Onramp service base URI
        #[arg(long, default_value = "https://testnet.lx.dev")]
        onramp_uri: String,
    },
    /// Withdraw via Lightning Network by burning cBTC through an offramp service
    WithdrawLn {
        /// BOLT11 Lightning invoice to pay out
        #[arg(required = true, long)]
        invoice: String,

        /// Burn substitutor EVM address (middleware)
        #[arg(required = true, long)]
        substitutor: String,

        /// Address to be encoded into burn note
        #[arg(required = true, long)]
        address: String,

        /// Offramp service base URI
        #[arg(long, default_value = "https://testnet.lx.dev")]
        offramp_uri: String,
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
        .build(chain, false)?;

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
    amount: u64,
    ticker: &str,
) -> Result<()> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, false)?;

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
        .build(chain, false)?;

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
        .build(chain, false)?;

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

async fn handle_depo_ln(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    chain: u64,
    amount_sat: u64,
    onramp_uri: &str,
) -> Result<()> {
    // 1. Generate preimage and payment_hash
    let mut preimage = [0u8; 32];
    OsRng.fill_bytes(&mut preimage);
    let payment_hash: [u8; 32] = Sha256::digest(preimage).into();
    let preimage_hex = hex::encode(preimage);
    let payment_hash_hex = hex::encode(payment_hash);

    // 2. Build NodeClient and prepare mint note
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, false)?;

    // 4. Init swap: GET /onramp/{amount}/{payment_hash}
    let http = reqwest::Client::new();
    let init_url = format!("{onramp_uri}/onramp/{amount_sat}/{payment_hash_hex}");
    let init_resp = http
        .get(&init_url)
        .send()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to reach onramp service: {}", e))?;

    if !init_resp.status().is_success() {
        return Err(color_eyre::eyre::eyre!(
            "Onramp service error: {}",
            init_resp.status()
        ));
    }

    let init: serde_json::Value = init_resp
        .json()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse onramp response: {}", e))?;

    let invoice = init["invoice"]
        .as_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing 'invoice' in onramp response"))?
        .to_string();
    let swap_id = init["id"]
        .as_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing 'id' in onramp response"))?
        .to_string();

    // 5. Print invoice for user to pay
    println!("\n⚡ Lightning Invoice:");
    println!("   {invoice}");
    println!("\n   Swap ID: {swap_id}");
    println!("\nPay the invoice and wait...\n");

    use serde::{Deserialize, Serialize};
    #[derive(Deserialize, Serialize, Debug)]
    struct OnrampResponse {
        state: u32,
        amount: u64,
        #[serde(rename = "stateDescription")]
        state_description: String,
    }

    // Conversion factor: 1 sat = 10^10 token base units (WCBTC has 10 decimals).
    const SATS_TO_TOKEN_UNITS: u64 = 10_000_000_000;
    // Maximum number of status-poll attempts before giving up (~10 minutes).
    const MAX_POLL_ATTEMPTS: u32 = 150;
    // Seconds between each status poll.
    const POLL_INTERVAL_SECS: u64 = 4;

    // 7. Poll for payment
    let status_url = format!("{onramp_uri}/onramp/{swap_id}");

    let amount_out;
    let mut attempts = 0u32;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;

        attempts += 1;
        if attempts > MAX_POLL_ATTEMPTS {
            return Err(color_eyre::eyre::eyre!(
                "Timed out waiting for onramp payment after {} attempts",
                MAX_POLL_ATTEMPTS
            ));
        }

        let resp = http
            .get(&status_url)
            .send()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Poll error: {}", e))?;

        let http_status = resp.status();
        if !http_status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(color_eyre::eyre::eyre!(
                "Onramp status check failed with HTTP {}: {}",
                http_status,
                body
            ));
        }

        let response: OnrampResponse = resp
            .json()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse status response: {}", e))?;

        println!("State: {} - {}", response.state, response.state_description);

        match response.state {
            2 => {
                amount_out = response.amount.saturating_mul(SATS_TO_TOKEN_UNITS);
                break;
            }
            // States > 2 are terminal failure states (e.g. refunded, expired, failed).
            s if s > 2 => {
                return Err(color_eyre::eyre::eyre!(
                    "Onramp swap reached terminal failure state {}: {}",
                    s,
                    response.state_description
                ));
            }
            _ => {} // Still pending/in-progress; keep polling.
        }
    }

    let (prepared_wallet, utxo) = client.get_wallet().prepare_mint(amount_out, "WCBTC")?;

    let note = &utxo.output_notes[0];
    let mint_hash = hash_merge([note.psi, Note::padding_note().psi]);
    let note_kind = note.contract;

    let mint_hash_h256 = convert_element_to_h256(&mint_hash);
    let note_kind_h256 = convert_element_to_h256(&note_kind);

    println!("Note amount: {}, {:x}", amount_out, note.value);

    println!("Generating zero-knowledge proof...");
    let snark = utxo.prove().unwrap();
    println!("✅ Proof ready.\n");

    // 8. Reveal preimage to claim the deposit
    println!("\nClaiming deposit...");
    let claim_resp = http
        .post(&status_url)
        .json(&serde_json::json!({
            "preimage": preimage_hex,
            "mint_hash": format!("{:x}", mint_hash_h256),
            "note_kind": format!("{:x}", note_kind_h256),
        }))
        .send()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to send claim: {}", e))?;

    if !claim_resp.status().is_success() {
        return Err(color_eyre::eyre::eyre!(
            "Claim failed with status: {}",
            claim_resp.status()
        ));
    }
    println!("✅ Preimage revealed, onramp mint triggered.");

    // 9. Submit ZK proof to the Ciphera node
    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Transaction {} has been sent!", tx.txn_hash);
            println!("   Height: {}", tx.height);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            let b = client.get_wallet().balance;
            println!("\nBalance {b} WCBTC");
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

async fn handle_withdraw_ln(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
    chain: u64,
    invoice: &str,
    substitutor: &str,
    address: &str,
    offramp_uri: &str,
) -> Result<()> {
    /*    let client = NodeClient::builder()
            .name(name)
            .host(host)
            .port(port)
            .timeout_secs(timeout_secs)
            .build(chain, false)?;

        let b = client.get_wallet().balance;
        TODO: balance check before everything even starts
    */
    // Step 1 — GET /offramp/{lnInvoice}/{substitutorAddress}
    // Returns the swap quote: swap ID and the cBTC amount the user must burn.
    let http = reqwest::Client::new();
    let quote_url = format!("{offramp_uri}/offramp/{invoice}/{substitutor}");

    println!("\n⚡ Requesting offramp quote...");

    let quote_resp = http
        .get(&quote_url)
        .send()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to reach offramp service: {}", e))?;

    if !quote_resp.status().is_success() {
        let status = quote_resp.status();
        let body = quote_resp.text().await.unwrap_or_default();
        return Err(color_eyre::eyre::eyre!(
            "Offramp service error {}: {}",
            status,
            body
        ));
    }

    let quote: serde_json::Value = quote_resp
        .json()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse offramp quote: {}", e))?;

    let swap_id = quote["id"]
        .as_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing 'id' in offramp quote"))?
        .to_string();

    // inputAmountWei is the burn amount in ERC-20 wei (cBTC has 18 decimals).
    // Convert to satoshis: 1 sat = 1e10 wei  (1 BTC = 1e8 sats = 1e18 wei).
    // Use u128 for the intermediate wei value: amounts above ~18 BTC overflow u64.
    // The API may return this field as a decimal string or as a JSON number.
    let input_amount_wei: u128 = {
        let v = &quote["inputAmountWei"];
        if let Some(s) = v.as_str() {
            s.parse::<u128>()
                .map_err(|_| color_eyre::eyre::eyre!("Invalid 'inputAmountWei' in offramp quote"))?
        } else if let Some(n) = v.as_u64() {
            n as u128
        } else {
            return Err(color_eyre::eyre::eyre!(
                "Missing or invalid 'inputAmountWei' in offramp quote"
            ));
        }
    };

    let input_amount: u64 = u64::try_from(input_amount_wei)
        .map_err(|_| color_eyre::eyre::eyre!("Converted Wei amount exceeds u64 maximum"))?;

    let quote_expiry = quote["quoteExpiry"].as_u64().unwrap_or(0);

    println!("\n✅ Offramp quote received!");
    println!("   Swap ID:      {swap_id}");
    println!("   Burn amount:  {input_amount} wei");
    println!("   Quote expiry: {quote_expiry} (unix timestamp)");
    println!("\n   Burning {input_amount} cBTC to substitutor {substitutor}...");

    // Step 2 — Create a burn note for inputAmount and submit it to the Ciphera node.
    // The user address is the burn target for refunds. The offramp service claims the burned
    // cBTC on the EVM side and settles the Lightning invoice.
    handle_burn(
        name,
        host,
        port,
        timeout_secs,
        chain,
        address, // refund address
        input_amount,
        "WCBTC",
        false,
    )
    .await?;

    println!("\n✅ Burn submitted. Waiting for the substitutor to settle the Lightning invoice...");

    // Step 3 — Poll /offramp/{swapId} until the swap reaches a terminal state.
    use serde::Deserialize;
    #[derive(Deserialize, Debug)]
    struct OfframpStatusResponse {
        state: i32,
        description: String,
    }

    const MAX_POLL_ATTEMPTS: u32 = 150; // ~10 minutes at 4 s intervals
    const POLL_INTERVAL_SECS: u64 = 4;

    let status_url = format!("{offramp_uri}/offramp/{swap_id}");
    let mut attempts = 0u32;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;

        attempts += 1;
        if attempts > MAX_POLL_ATTEMPTS {
            return Err(color_eyre::eyre::eyre!(
                "Timed out waiting for offramp swap to complete after {} attempts",
                MAX_POLL_ATTEMPTS
            ));
        }

        let resp = http
            .get(&status_url)
            .send()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Poll error: {}", e))?;

        let http_status = resp.status();
        if !http_status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(color_eyre::eyre::eyre!(
                "Offramp status check failed with HTTP {}: {}",
                http_status,
                body
            ));
        }

        let response: OfframpStatusResponse = resp
            .json()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse offramp status: {}", e))?;

        println!("State: {} - {}", response.state, response.description);

        match response.state {
            2 | 3 => {
                println!("\n✅ Lightning invoice settled! Swap complete.");
                println!("   Swap ID: {swap_id}");
                break;
            }
            -2 | -3 => {
                return Err(color_eyre::eyre::eyre!(
                    "Offramp swap reached terminal failure state {}: {}",
                    response.state,
                    response.description
                ));
            }
            _ => {} // CREATED, COMMITED, or other in-progress states; keep polling.
        }
    }

    Ok(())
}

async fn handle_mint(
    name: &str,
    host: &str,
    port: u16,
    timeout_secs: u64,
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
        .build(chain, false)?;

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
    address: &str,
    amount: u64,
    ticker: &str,
    natively_substitute: bool,
) -> Result<(), AppError> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .port(port)
        .timeout_secs(timeout_secs)
        .build(chain, false)?;

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

    let (wallet_with_burner_note, _) = client
        .get_wallet()
        .prepare_add_to_avail(burner_note.clone())?;
    wallet_with_burner_note.save()?;
    client.replace_wallet(wallet_with_burner_note);

    let evm_address = match H160::from_str(address) {
        Ok(a) => convert_h160_to_element(&a),
        Err(e) => return Err(AppError::InvalidAddress(e.to_string())),
    };

    let (wallet_after_burn, burner_utxo) =
        client
            .get_wallet()
            .prepare_burn(&burner_note, &evm_address, natively_substitute)?;

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
    let version = rollup.version().await?;
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
    println!("\tVersion              :{version} ");
    println!("\tToken kind WBTC      :{token_wbtc:#x} ");
    println!("\tToken kind USDC      :{token_usdc:#x} ");
    println!("\tBlock                :{b} ");
    println!("\tRoot hash            :{rh:#x} ");

    // Enumerate zkVerifierKeys array and look up each entry in the zkVerifiers mapping
    println!("\nZK Verifiers\n");
    let mut index = 0u64;
    loop {
        match rollup.zk_verifier_keys(U256::from(index)).await {
            Ok(key_hash) => {
                if let Ok((address, circuit_id, enabled)) = rollup.zk_verifiers(key_hash).await {
                    println!(
                        "\t[{index}]\n\tkey={key_hash:#x}\n\taddress={address:#x}\n\t\
                        circuit_id={circuit_id}  enabled={enabled}"
                    );
                }
                index += 1;
            }
            Err(_) => break,
        }
    }
    if index == 0 {
        println!("\tNo ZK verifiers found.");
    }

    // Last mint events
    println!("\nLast Mint Events\n");
    let mint_events = rollup.get_all_mint_added_events().await?;
    println!("\tTotal mints: {}\n", mint_events.len());
    if mint_events.is_empty() {
        println!("\tNo mints found.");
    } else {
        println!(
            "\t{:<66}  {:>20}  {:<66}  Block",
            "Mint Hash", "Value", "Note Kind"
        );
        for event in &mint_events {
            println!(
                "\t{:#x}  {:>20}  {:#x}  {}",
                event.mint_hash, event.value, event.note_kind, event.block_number
            );
        }
    }

    Ok(())
}

async fn handle_mints(geth_rpc: &str, chain: u64, rollup: &str) -> Result<()> {
    let client = contracts::Client::new(geth_rpc, None);
    let rollup = contracts::ReadonlyRollupContract::load(client, rollup).await?;

    let events = rollup.get_all_mint_added_events().await?;

    println!("\nMint Hashes in Contract\n");
    println!("\tChain: {chain}");
    println!("\tTotal mints: {}\n", events.len());

    if events.is_empty() {
        println!("\tNo mints found.");
    } else {
        println!(
            "\t{:<66}  {:>20}  {:<66}  Block",
            "Mint Hash", "Value", "Note Kind"
        );
        println!("\t{}", "-".repeat(160));
        for event in &events {
            println!(
                "\t{:#x}  {:>20}  {:#x}  {}",
                event.mint_hash, event.value, event.note_kind, event.block_number
            );
        }
    }

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

    debug!("Starting Ciphera CLI");

    // Execute command
    match cli.command {
        Commands::Create {} => {
            handle_create(cli.chain, &cli.name).await?;
        }
        Commands::Sync {} => {
            handle_sync(cli.chain, &cli.name, &cli.host, cli.port, cli.timeout).await?;
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
                &address,
                amount,
                &ticker_normalized,
                true,
            )
            .await?;
        }
        Commands::Contract { geth_rpc } => {
            handle_rollup(&geth_rpc, cli.chain, &cli.rollup).await?;
        }
        Commands::Mints { geth_rpc } => {
            handle_mints(&geth_rpc, cli.chain, &cli.rollup).await?;
        }
        Commands::DepoLn {
            amount_sat,
            onramp_uri,
        } => {
            handle_depo_ln(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                cli.chain,
                amount_sat,
                &onramp_uri,
            )
            .await?;
        }
        Commands::WithdrawLn {
            invoice,
            substitutor,
            address,
            offramp_uri,
        } => {
            handle_withdraw_ln(
                &cli.name,
                &cli.host,
                cli.port,
                cli.timeout,
                cli.chain,
                &invoice,
                &substitutor,
                &address,
                &offramp_uri,
            )
            .await?;
        }
    }

    println!("\n");
    Ok(())
}
