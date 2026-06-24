use clap::{Parser, Subcommand};

use cli::NodeClient;
use cli::Wallet;
use cli::address::{
    citrea_ticker_from_contract, decode_address, normalize_citrea_ticker, supported_citrea_tokens,
};
use cli::note_url::{CipheraURL, decode_url};
use cli::units;

use color_eyre::Result;
use tracing::{debug, error};
use web3::types::{H160, H256, U256};

use barretenberg::{Prove, Verify};
use contracts::util::{convert_element_to_h256, convert_h160_to_element};
use element::Element;
use hash::hash_merge;
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use zk_primitives::{InputNote, Note, get_address_for_private_key};

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

    /// RPC server host. Include the scheme for TLS (`https://host`); a
    /// custom port can be appended inline as `https://host:7777`. The
    /// previous `--port` and `--timeout` flags were dropped: the scheme
    /// determines the default port (443 / 80) and the timeout has a
    /// sensible 60 s default baked into the client builder.
    #[arg(global = true, long, default_value = "https://ciphera.satsbridge.com")]
    host: String,

    #[arg(global = true, short, long, default_value = "5115")] // Citrea testnet default
    chain: u64,

    #[arg(
        global = true,
        long,
        default_value = "0xbd57b7d47d66934509f9ca31248598eb6cb3fafd"
    )]
    rollup: String,

    /// Mainnet mempool.space-compatible API base URL for Bitcoin block
    /// data, used by the HTLC escrow timelock (anchor at lock time, PoW
    /// headers at refund time). The timelock is measured in mainnet block
    /// work; point this at a mainnet instance.
    #[arg(global = true, long, default_value = "https://mempool.space")]
    btc_explorer: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Create {},
    /// Connect to a Ciphera node and check its health
    Sync {},
    Address {
        /// Amount in satoshis
        #[arg(required = true, short, long)]
        amount_sat: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,
    },
    Mint {
        #[arg(required = true, long, short)]
        geth_rpc: String,

        #[arg(required = true, long, short)]
        secret: String,

        /// Amount in satoshis
        #[arg(required = true, short, long)]
        amount_sat: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,

        #[arg(required = false, long, short, action=clap::ArgAction::SetTrue)]
        only_snark: bool,
    },
    Burn {
        #[arg(required = true, long)]
        address: String,

        /// Amount in satoshis
        #[arg(required = true, short, long)]
        amount_sat: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,
    },
    Spend {
        /// Amount in satoshis
        #[arg(required = true, short, long)]
        amount_sat: u64,

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

        /// Number of past blocks to scan for events (mints + slow burns).
        /// Paginated in 1000-block chunks under the hood.
        #[arg(long, short, default_value = "1000")]
        blocks: u64,
    },
    /// List all mint hashes stored in the rollup contract with their values
    Mints {
        #[arg(required = true, long, short)]
        geth_rpc: String,

        /// Number of past blocks to scan for MintAdded events.
        /// Paginated in 1000-block chunks under the hood.
        #[arg(long, short, default_value = "1000")]
        blocks: u64,
    },
    /// Release a queued slow burn by reading metadata from the
    /// SlowBurnQueued event and calling releaseSlowBurn on the rollup
    /// contract.
    ReleaseSlowBurn {
        #[arg(required = true, long, short)]
        geth_rpc: String,

        /// Private key used to send the releaseSlowBurn transaction.
        #[arg(required = true, long, short)]
        secret: String,

        /// `key` (indexed topic) of the SlowBurnQueued event to release.
        /// 0x-prefixed 32-byte hex.
        #[arg(required = true, long, short)]
        key: String,

        /// Number of past blocks to scan for the SlowBurnQueued event.
        /// Paginated in 1000-block chunks under the hood.
        #[arg(long, short, default_value = "1000")]
        blocks: u64,
    },
    /// Lock funds from this wallet into an HTLC escrow note. Spends one
    /// or two available notes through the standard Utxo Send circuit
    /// and emits an output note encumbered by `SHA256(preimage)` on the
    /// claim side and a PoW timelock on the refund side. Writes the
    /// resulting `EscrowInputNote` (note + secret_key + preimage) to
    /// `<name>-htlc.json` so it can be picked up by `escrow-redeem` /
    /// `escrow-refund`.
    EscrowLock {
        /// Amount to lock, in satoshis.
        #[arg(required = true, short, long)]
        amount_sat: u64,

        #[arg(short, long, default_value = "WCBTC")]
        ticker: String,

        /// 32-byte preimage that unlocks the claim path, given as
        /// 64-hex (optionally prefixed with 0x). The redeemer needs
        /// this value; share it via the `EscrowInputNote` JSON or out
        /// of band.
        #[arg(required = true, long)]
        preimage: String,
    },
    /// Redeem an HTLC escrow note by revealing the preimage. Reads the
    /// `EscrowInputNote` from the JSON written at lock time.
    EscrowRedeem {
        /// Path to the `EscrowInputNote` JSON produced by
        /// `escrow-lock`.
        #[arg(required = true, long, default_value = "alice-htlc.json")]
        note: String,
    },
    /// Refund an HTLC escrow note after the timelock has elapsed.
    /// Reuses the secret key embedded in the JSON written at lock
    /// time; the preimage is *not* required.
    EscrowRefund {
        /// Path to the `EscrowInputNote` JSON produced by
        /// `escrow-lock`.
        #[arg(required = true, long, default_value = "alice-htlc.json")]
        note: String,
    },
    /// Deposit via Lightning using the Atomiq exchange trade protocol:
    /// requests an onramp swap from the Atomiq service, prints the invoice
    /// to pay, then mints the resulting note via the reveal-preimage flow.
    AtomiqDepo {
        /// Amount to deposit in satoshis
        #[arg(required = true, short, long)]
        amount_sat: u64,

        /// Atomiq onramp service base URI
        #[arg(long, default_value = "https://testnet.lx.dev")]
        onramp_uri: String,
    },
    /// Deposit via Lightning through the ln-service onramp flow. Two
    /// steps: (1) request a bolt11 invoice from the ln-service and pay it,
    /// then (2) once the service funds the note on chain, redeem it into
    /// this wallet using the preimage as the note's spend key.
    DepoLn {
        /// Amount to deposit in satoshis
        #[arg(required = true, short, long)]
        amount_sat: u64,

        /// ln-service base URI (serves GET /v0/onramp)
        #[arg(long, default_value = "http://127.0.0.1:7080")]
        ln_service_uri: String,
    },
    /// Withdraw via Lightning using the Atomiq exchange trade protocol:
    /// fetches a swap quote from the Atomiq offramp service and burns
    /// cBTC through a burn substitutor middleware that settles the
    /// Lightning invoice.
    AtomiqWithdraw {
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
    /// Withdraw via Lightning through the ln-service escrow flow. Two
    /// steps: (1) request HTLC escrow data for a BOLT11 invoice from the
    /// ln-service, then (2) commit funds by spending wallet notes into
    /// the returned escrow note. The ln-service is the redeemer and
    /// settles the invoice once the escrow lands on chain.
    WithdrawLn {
        /// BOLT11 Lightning invoice to pay out
        #[arg(required = true, long)]
        invoice: String,

        /// ln-service base URI (serves POST /v0/offramp)
        #[arg(long, default_value = "http://127.0.0.1:7080")]
        ln_service_uri: String,
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
    #[error("Unsupported token '{0}'. Supported tokens: WCBTC, CUSD")]
    InvalidTicker(String),
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

fn parse_cli_ticker(ticker: &str) -> Result<&'static str, AppError> {
    normalize_citrea_ticker(ticker).ok_or_else(|| AppError::InvalidTicker(ticker.to_string()))
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
    println!("   Balance: {} sats", units::wei_to_sats(wallet.balance));

    println!("\n⚠️  IMPORTANT: Keep your private key safe!");
    println!("   Your private key is stored in {wallet_file}");
    println!("   Never share it with anyone.");

    println!("\n🚀 Next Steps:");
    println!("   1. Connect to network:  ciphera-cli --name {name} --chain {chain} sync");
    println!(
        "   2. Mint WCBTC:          ciphera-cli --name {name} --chain {chain} --rollup <ROLLUP_CONTRACT> mint --amount-sat <AMOUNT_SAT> --secret <YOUR_CITREA_KEY> --geth-rpc <RPC_URL>"
    );
    println!("   3. Check balance:       cat {wallet_file}");

    Ok(())
}

/// Handle the sync command
///
/// Connects to a Ciphera node and performs health checks
async fn handle_sync(
    chain: u64,
    name: &str,
    host: &str,
) -> Result<()> {
    debug!(
        "Connecting wallet {} to Ciphera node at {}",
        name, host
    );

    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
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

    println!("\n✨ Successfully connected to Ciphera node at {host}");
    Ok(())
}

async fn handle_address(name: &str, amount_wei: u64, ticker: &str) -> Result<()> {
    let wallet = Wallet::load(name)?;
    let b = wallet.balance;
    let (wallet, a) = wallet.prepare_get_address(amount_wei, ticker);
    wallet.save()?;

    println!("\nWallet {name} has been found:");
    println!("\tBalance: {} sats", units::wei_to_sats(b));
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
    amount_wei: u64,
    ticker: &str,
) -> Result<()> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;

    // Prepare transfer. A case, when wallet already has exactly matching note, will be ignored
    let (wallet_with_transfer_note, transfer_note) =
        client.get_wallet().prepare_receive_note(amount_wei, ticker);
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
            println!("\nBalance {} sats {ticker}", units::wei_to_sats(b));
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
    address: &str,
) -> Result<()> {
    debug!(
        "Connecting wallet {} to Ciphera node at {}",
        name, host
    );

    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;

    // Spend to UX leverages a variant of NoteURL encoding for providing an "address" with address
    // and amount needed for UTXO construction
    let note = Note::from(&decode_address(address));
    let ticker = citrea_ticker_from_contract(note.note_kind);

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
            println!("\nBalance {} sats {ticker}", units::wei_to_sats(b));
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
    chain: u64,
    notefile: Option<String>,
    notelink: Option<String>,
) -> Result<()> {
    debug!(
        "Connecting wallet {} to Ciphera node at {}",
        name, host
    );

    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
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

    println!("\n✨ Successfully connected to Ciphera node at {host}");

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

    let ticker = citrea_ticker_from_contract(input_note.note.note_kind);

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
            println!("\nBalance {} sats {ticker}", units::wei_to_sats(b));
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

// Parse a 32-byte preimage from `0x`-optional hex.
fn parse_preimage_hex(s: &str) -> Result<[u8; 32], AppError> {
    let trimmed = s.trim().trim_start_matches("0x");
    let bytes = hex::decode(trimmed)
        .map_err(|e| AppError::InvalidAddress(format!("preimage must be hex: {e}")))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| AppError::InvalidAddress("preimage must be 32 bytes".to_string()))
}

#[allow(clippy::too_many_arguments)]
async fn handle_escrow_lock(
    chain: u64,
    name: &str,
    host: &str,
    amount_wei: u64,
    ticker: &str,
    preimage_hex: &str,
    btc_explorer: &str,
) -> Result<()> {
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;

    let preimage = parse_preimage_hex(preimage_hex)?;

    // Anchor the HTLC refund timelock to the *current Bitcoin tip*. The
    // refund branch (later) must present a PoW chain extending this anchor
    // by `n_blocks`, so the timelock only opens once that many blocks are
    // mined. (Tests use a fixed fixture; production uses real blocks.)
    let lock = bitcoin_clock::BitcoinClock::new(btc_explorer)
        .tip_lock(2)
        .await?;
    println!(
        "\n🔒 Timelock anchored to Bitcoin tip {} (+{} blocks)",
        hex::encode(lock.zero_block),
        lock.n_blocks,
    );

    // For this single-key test flow the same secret key gates both the
    // claim and refund branches: convenient for a single-wallet
    // round-trip, equivalent to running Alice = Bob. A real two-party
    // HTLC would split this into a redeemer secret + a separate refund
    // secret; the wallet API can be extended trivially once the second
    // wallet is wired in.
    let htlc_secret_key = client.get_wallet().gen_pk();

    let (prepared_wallet, utxo, escrow_input_note) = client
        .get_wallet()
        .prepare_escrow_lock(amount_wei, ticker, htlc_secret_key, preimage, &lock)?;

    let snark = utxo.prove().unwrap();

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Lock transaction {} submitted", tx.txn_hash);
            println!("   Height:    {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            let note_path = format!("{name}-htlc.json");
            let json_str = serde_json::to_string_pretty(&escrow_input_note)?;
            std::fs::write(&note_path, json_str)?;
            println!("\nSaved HTLC EscrowInputNote to {note_path}");
            println!("  HTLC secret_key: {}", escrow_input_note.secret_key);
            println!("  preimage hex:    0x{}", hex::encode(preimage));
            println!("  note address:    {}", escrow_input_note.note.address);
            println!("  note psi:        {}", escrow_input_note.note.psi);

            let b = client.get_wallet().balance;
            println!("\nBalance {} sats {ticker}", units::wei_to_sats(b));
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send lock transaction!");
            Err(e)
        }
    }
}

async fn handle_escrow_redeem_or_refund(
    chain: u64,
    name: &str,
    host: &str,
    note_path: &str,
    refund: bool,
    btc_explorer: &str,
) -> Result<()> {
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;

    let json_str = fs::read_to_string(note_path).map_err(|e| {
        AppError::IoError(std::io::Error::new(e.kind(), format!("{note_path}: {e}")))
    })?;
    let htlc_input_note: zk_primitives::EscrowInputNote = serde_json::from_str(&json_str)?;

    let ticker = cli::address::citrea_ticker_from_contract(htlc_input_note.note.note_kind);
    let label = if refund { "Refund" } else { "Redeem" };

    let (prepared_wallet, escrow, _received) = if refund {
        // Build the refund PoW witness from real Bitcoin headers extending
        // the lock anchor committed at lock time. Fails clearly if fewer
        // than `n_blocks` blocks have been mined since the anchor (the
        // timelock has not elapsed yet).
        // The lock was persisted via TimeProof serialization when the note
        // was created; we recover it and use it to fetch the PoW headers.
        let lock = &htlc_input_note.time_proof.lock;
        if lock.zero_block == [0u8; 32] {
            return Err(color_eyre::eyre::eyre!(
                "refund requires a timelock anchor (not found in this note); \
                 the note may have been created by an old CLI version. \
                 Create a new escrow lock/refund pair."
            ));
        }
        let time_proof = bitcoin_clock::BitcoinClock::new(btc_explorer)
            .refund_proof(lock)
            .await?;
        client
            .get_wallet()
            .prepare_escrow_refund(&htlc_input_note, time_proof)?
    } else {
        client.get_wallet().prepare_escrow_redeem(&htlc_input_note)?
    };

    println!("\n🔓 Generating {label} EscrowProof locally...");
    let snark = escrow
        .prove()
        .map_err(|e| color_eyre::eyre::eyre!("escrow.prove() failed: {e}"))?;
    println!("✅ EscrowProof generated ({} bytes)", snark.proof.0.len());

    // Local self-verification before going on chain. Catches
    // witness-level bugs (wrong preimage, stale PoW chain,
    // secret_key mismatched against the baked-in commitment)
    // without burning a node round-trip.
    snark
        .verify()
        .map_err(|e| color_eyre::eyre::eyre!("escrow.verify() failed: {e}"))?;
    println!("✅ Local verification of EscrowProof succeeded");

    // Submit through the node's `/v0/transaction` endpoint as
    // `LeafProof::Escrow`. The endpoint accepts both leaf flavours
    // since the heterogeneous-aggregation refactor; the node routes
    // escrow proofs through the agg_escrow → agg_agg slot pair on
    // commit.
    match client.transaction_escrow(&snark).await {
        Ok(tx) => {
            println!("\n✅ {label} transaction {} has been submitted!", tx.txn_hash);
            println!("   Height:    {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            let b = client.get_wallet().balance;
            println!("\nBalance {} sats {ticker}", units::wei_to_sats(b));
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ {label} submission failed: {e}");
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

async fn handle_atomiq_depo(
    name: &str,
    host: &str,
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

    // Maximum number of status-poll attempts before giving up (~10 minutes).
    const MAX_POLL_ATTEMPTS: u32 = 150;
    // Seconds between each status poll.
    const POLL_INTERVAL_SECS: u64 = 4;

    // 7. Poll for payment
    let status_url = format!("{onramp_uri}/onramp/{swap_id}");

    let amount_out_wei;
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
                // `OnrampResponse.amount` is the settled amount in **satoshis** as specified
                // by the onramp API contract (field name `amount`, unit: sat).
                // Convert to wei (the on-chain ERC-20 base unit) before minting the note.
                amount_out_wei = units::sats_to_wei(response.amount);
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

    let (prepared_wallet, utxo) = client.get_wallet().prepare_mint(amount_out_wei, "WCBTC")?;

    let note = &utxo.output_notes[0];
    let mint_hash = hash_merge([note.psi, Note::padding_note().psi]);
    let note_kind = note.note_kind;

    let mint_hash_h256 = convert_element_to_h256(&mint_hash);
    let note_kind_h256 = convert_element_to_h256(&note_kind);

    println!(
        "Note amount: {} sats ({} wei), {:x}",
        units::wei_to_sats(amount_out_wei),
        amount_out_wei,
        note.value,
    );

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
            println!("\nBalance {} sats WCBTC", units::wei_to_sats(b));
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

/// Reduce a 32-byte Lightning preimage to a single BN254 field element by
/// taking its **low 16 bytes**. Must match the ln-service convention
/// (`settlement::onramp::preimage_field`): a full 32-byte value can exceed
/// the field modulus, so only the trailing 128 bits (big-endian low half)
/// are used as the note's spend key.
fn preimage_field(preimage: [u8; 32]) -> Element {
    let mut buf = [0u8; 32];
    buf[16..].copy_from_slice(&preimage[16..]);
    Element::from_be_bytes(buf)
}

/// ln-service onramp deposit. Step 1 requests a bolt11 invoice for
/// `amount_sat` and prints it to pay — the ln-service commits the
/// preimage-locked note on chain right away (funded from its own balance).
/// Step 2 polls until the invoice settles (`Settled`), at which point the
/// service reveals the preimage and the note fields; we then redeem the
/// note into this wallet using `preimage_field(preimage)` as the spend
/// key. If the invoice is never paid the service refunds the note itself.
async fn handle_depo_ln(
    name: &str,
    host: &str,
    chain: u64,
    amount_sat: u64,
    ln_service_uri: &str,
) -> Result<()> {
    let http = reqwest::Client::new();

    // Step 1 — GET /v0/onramp?amount_sat=N -> { payment_hash, bolt11 }.
    let create_url = format!("{ln_service_uri}/v0/onramp?amount_sat={amount_sat}");
    println!("\n⚡ Requesting onramp invoice from ln-service...");
    let resp = http
        .get(&create_url)
        .send()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to reach ln-service: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(color_eyre::eyre::eyre!("ln-service error {}: {}", status, body));
    }
    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse ln-service response: {}", e))?;

    let payment_hash = data["payment_hash"]
        .as_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing 'payment_hash' in ln-service response"))?
        .to_string();
    let bolt11 = data["bolt11"]
        .as_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing 'bolt11' in ln-service response"))?
        .to_string();

    // Poll until the onramp's own expiry (+ grace for the refund to land),
    // not a fixed short window: the service holds the committed note until
    // `expires_at`, then refunds it. We track that so a slow payment isn't
    // abandoned while the deposit is still live.
    const POLL_INTERVAL_SECS: u64 = 4;
    const POLL_GRACE_SECS: i64 = 300; // wait past expiry to catch the refund
    const FALLBACK_TTL_SECS: i64 = 3600; // if the service omits expires_at

    let deadline = data["expires_at"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc) + chrono::Duration::seconds(POLL_GRACE_SECS))
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::seconds(FALLBACK_TTL_SECS));

    println!("\n⚡ Pay this Lightning invoice:");
    println!("   {bolt11}");
    println!("\n   Payment hash: {payment_hash}");
    println!(
        "\nWaiting for the ln-service to settle the deposit (until {})...\n",
        deadline.to_rfc3339()
    );

    // Step 2 — poll /v0/onramp/{payment_hash} until the invoice settles.
    let status_url = format!("{ln_service_uri}/v0/onramp/{payment_hash}");
    let (preimage_hex, note_json, note_commitment_hex) = loop {
        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;

        if chrono::Utc::now() > deadline {
            return Err(color_eyre::eyre::eyre!(
                "Timed out waiting for onramp to settle (deadline {})",
                deadline.to_rfc3339()
            ));
        }

        let resp = http
            .get(&status_url)
            .send()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Poll error: {}", e))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(color_eyre::eyre::eyre!("Onramp status check failed: {}", body));
        }
        let s: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to parse onramp status: {}", e))?;

        let status = s["status"].as_str().unwrap_or("");
        println!("State: {status}");

        // Redeem as soon as the data allows: the deposit note is on chain
        // (`note_commitment` present) AND the service has revealed the
        // preimage. Keying off the payload rather than a specific status
        // string keeps this working across ln-service versions
        // (`Settled` / `NoteConfirmed` / ...).
        if let (Some(preimage), Some(note_commitment)) =
            (s["preimage"].as_str(), s["note_commitment"].as_str())
        {
            break (
                preimage.to_string(),
                s["note"].clone(),
                note_commitment.to_string(),
            );
        }

        // Terminal states where the deposit will not be credited.
        match status {
            "Refunded" | "Refunding" | "Failed" | "Expired" | "Cancelled" => {
                return Err(color_eyre::eyre::eyre!(
                    "Onramp reached terminal state {status} (deposit not credited)"
                ));
            }
            _ => {} // keep polling
        }
    };

    // Step 3 — reconstruct the deposit note and redeem it. The note's spend
    // key is the preimage's low-half field element, which we now know.
    let preimage = parse_preimage_hex(&preimage_hex)?;
    let unlock_key = preimage_field(preimage);

    let note = if note_json.is_object() {
        // Modern ln-service returns the authoritative note fields (same
        // shape as the offramp flow).
        Note {
            utxo_kind: parse_element_field(&note_json, "utxo_kind")?,
            note_kind: parse_element_field(&note_json, "note_kind")?,
            address: parse_element_field(&note_json, "address")?,
            psi: parse_element_field(&note_json, "psi")?,
            value: parse_element_field(&note_json, "value")?,
        }
    } else {
        // Older ln-service returns only a commitment; derive the note from
        // the preimage and this wallet's chain note kind.
        let (utxo_kind, note_kind) =
            cli::address::citrea_token_data(cli::address::network_for_chain(chain), "WCBTC");
        Note {
            utxo_kind,
            note_kind,
            address: get_address_for_private_key(unlock_key),
            psi: hash_merge([unlock_key, unlock_key]),
            value: Element::new(units::sats_to_wei(amount_sat)),
        }
    };

    // The revealed preimage must actually unlock this note: its derived key
    // has to reproduce the note's address.
    if get_address_for_private_key(unlock_key) != note.address {
        return Err(color_eyre::eyre::eyre!(
            "preimage does not unlock the service note (address mismatch); \
             preimage convention disagrees with the ln-service"
        ));
    }

    // Cross-check the reconstructed note against the commitment the service
    // reported -- catches a note_kind / amount mismatch (e.g. wrong --chain)
    // before persisting a note that isn't actually on chain.
    let expected = Element::from_str(&note_commitment_hex)
        .map_err(|e| color_eyre::eyre::eyre!("invalid note_commitment from service: {e}"))?;
    if note.commitment() != expected {
        return Err(color_eyre::eyre::eyre!(
            "reconstructed note commitment {} != service commitment {}; \
             note_kind/amount disagree (check --chain matches the service)",
            note.commitment(),
            expected
        ));
    }

    let amount_wei = note.value.to_u64_array()[0];
    let input_note = InputNote::new(note, unlock_key);

    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;
    let (prepared_wallet, ()) = client.get_wallet().prepare_add_to_avail(input_note)?;
    prepared_wallet.save()?;
    client.replace_wallet(prepared_wallet);

    println!("\n✅ Deposit note redeemed into wallet.");
    println!("   note value: {} sats", units::wei_to_sats(amount_wei));
    let b = client.get_wallet().balance;
    println!("\nBalance {} sats WCBTC", units::wei_to_sats(b));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_atomiq_withdraw(
    name: &str,
    host: &str,
    chain: u64,
    invoice: &str,
    substitutor: &str,
    address: &str,
    offramp_uri: &str,
) -> Result<()> {
    /*    let client = NodeClient::builder()
            .name(name)
            .host(host)
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

    // inputAmountWei is the burn amount in ERC-20 wei (WCBTC has 18 decimals).
    // 1 sat = 10^10 wei  (1 BTC = 10^8 sats = 10^18 wei).
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

    let input_amount_wei_u64: u64 = u64::try_from(input_amount_wei)
        .map_err(|_| color_eyre::eyre::eyre!("inputAmountWei exceeds u64 maximum"))?;

    let quote_expiry = quote["quoteExpiry"].as_u64().unwrap_or(0);

    println!("\n✅ Offramp quote received!");
    println!("   Swap ID:      {swap_id}");
    println!(
        "   Burn amount:  {} sats ({} wei)",
        units::wei_to_sats_u128(input_amount_wei),
        input_amount_wei,
    );
    println!("   Quote expiry: {quote_expiry} (unix timestamp)");
    println!(
        "\n   Burning {} sats to substitutor {substitutor}...",
        units::wei_to_sats(input_amount_wei_u64),
    );

    // Step 2 — Create a burn note for inputAmountWei and submit it to the Ciphera node.
    // The user address is the burn target for refunds. The offramp service claims the burned
    // cBTC on the EVM side and settles the Lightning invoice.
    handle_burn(
        name,
        host,
        chain,
        address, // refund address
        input_amount_wei_u64,
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

/// Parse an `Element` field out of the ln-service JSON response,
/// mirroring the `Element::to_string()` encoding the service emits.
fn parse_element_field(value: &serde_json::Value, field: &str) -> Result<Element> {
    let s = value
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| color_eyre::eyre::eyre!("missing '{field}' in ln-service response"))?;
    Element::from_str(s)
        .map_err(|e| color_eyre::eyre::eyre!("invalid '{field}' in ln-service response: {e}"))
}

/// Parse a Bitcoin block hash from the timelock response (stored as hex string).
fn parse_timelock_hash(value: &serde_json::Value, field: &str) -> Result<[u8; 32]> {
    let s = value
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| color_eyre::eyre::eyre!("missing '{field}' in timelock response"))?;
    let bytes = hex::decode(s)
        .map_err(|e| color_eyre::eyre::eyre!("invalid hex in timelock '{field}': {e}"))?;
    let mut hash = [0u8; 32];
    if bytes.len() != 32 {
        return Err(color_eyre::eyre::eyre!(
            "timelock '{field}' must be 32 bytes, got {}",
            bytes.len()
        ));
    }
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

/// Verify escrow note values against the Bolt11 invoice and wallet configuration.
/// Prevents the service from:
/// - Switching amounts (user requests 100k sats, service returns 50k)
/// - Switching tokens/chains (service must return a supported Citrea token)
/// - Using wrong refund address (funds wouldn't be recoverable)
/// - Mismatching payment hash (wouldn't settle the correct invoice)
async fn verify_escrow_note_values(
    invoice: &str,
    service_payment_hash: &str,
    escrow_note: &Note,
    expected_refund_address: &Element,
    chain: u64,
    timelock: &zk_primitives::TimeLock,
) -> Result<()> {
    // Parse the Bolt11 invoice to extract amount and payment hash
    let bolt11_invoice = lightning_invoice::Bolt11Invoice::from_str(invoice)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse Bolt11 invoice: {e}"))?;

    let invoice_amount_msats = bolt11_invoice
        .amount_milli_satoshis()
        .ok_or_else(|| color_eyre::eyre::eyre!("Bolt11 invoice has no amount specified"))?;
    let invoice_amount_sats = invoice_amount_msats / 1_000;
    let invoice_payment_hash = bolt11_invoice.payment_hash();

    // Verify payment hash matches
    let invoice_payment_hash_hex = format!("{:x}", invoice_payment_hash);
    if service_payment_hash.to_lowercase() != invoice_payment_hash_hex.to_lowercase() {
        return Err(color_eyre::eyre::eyre!(
            "payment hash mismatch: invoice has {} but service returned {}; \
             service may be trying to settle a different invoice",
            invoice_payment_hash_hex,
            service_payment_hash
        ));
    }

    // Verify amount matches
    let escrow_amount_wei = escrow_note.value.to_u64_array()[0];
    let escrow_amount_sats = units::wei_to_sats(escrow_amount_wei);
    if escrow_amount_sats != invoice_amount_sats {
        return Err(color_eyre::eyre::eyre!(
            "escrow amount mismatch: invoice specifies {} sats but service returned {} sats; \
             service may be attempting to short the withdrawal",
            invoice_amount_sats,
            escrow_amount_sats
        ));
    }

    // Verify token kind is one of the supported Citrea assets for this chain.
    let network = cli::address::network_for_chain(chain);
    let token = supported_citrea_tokens(network)
        .into_iter()
        .find(|(_, note_kind)| *note_kind == escrow_note.note_kind)
        .map(|(ticker, _)| ticker)
        .ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "unsupported note kind {} for chain {}; supported tokens: WCBTC, CUSD",
                escrow_note.note_kind,
                chain
            )
        })?;
    debug!(token, "verified escrow token kind");

    // Verify note commitment (psi) encodes the refund address and timelock.
    // For the HTLC refund branch: psi = Poseidon(key_hash, timelock.commitment())
    // where key_hash = get_address_for_private_key(refund_secret_key).
    // We can verify that the returned psi is consistent with our refund address and timelock.
    let expected_psi = hash::hash_merge([*expected_refund_address, timelock.commitment()]);
    if escrow_note.psi != expected_psi {
        return Err(color_eyre::eyre::eyre!(
            "note psi mismatch: expected {} but service returned {}; \
             service may be using a different refund address or timelock",
            expected_psi,
            escrow_note.psi
        ));
    }

    Ok(())
}

/// Verify that the service-returned timelock anchor matches the current Bitcoin tip.
/// Prevents the service from lying about what block the escrow is anchored to.
async fn verify_timelock_matches_tip(
    timelock_anchor: [u8; 32],
    btc_explorer: &str,
) -> Result<()> {
    let current_tip = bitcoin_clock::BitcoinClock::new(btc_explorer)
        .tip_lock(1)
        .await?;

    if timelock_anchor != current_tip.zero_block {
        return Err(color_eyre::eyre::eyre!(
            "timelock anchor {} does not match current Bitcoin tip {}; \
             service may be returning a stale or incorrect anchor",
            hex::encode(timelock_anchor),
            hex::encode(current_tip.zero_block)
        ));
    }

    Ok(())
}

/// ln-service escrow withdrawal. Step 1 requests HTLC escrow data for a
/// BOLT11 invoice; step 2 commits funds by spending wallet notes into
/// the returned escrow note. The ln-service holds the claim branch (it
/// is the redeemer and settles the invoice). The refund branch is bound
/// to a fresh wallet-owned key so the funds return to this wallet in the
/// fallback scenario where the service never claims; that key is
/// persisted alongside the escrow note for a later `escrow-refund`.
async fn handle_withdraw_ln(
    name: &str,
    host: &str,
    chain: u64,
    invoice: &str,
    ln_service_uri: &str,
    btc_explorer: &str,
) -> Result<()> {
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;

    // Refund key is the wallet's own: the escrow's refund branch binds
    // `psi` to `get_address_for_private_key(refund_secret_key)`, so only
    // this wallet can reclaim the funds if the service never settles.
    let refund_secret_key = client.get_wallet().gen_pk();
    let refund_address = get_address_for_private_key(refund_secret_key);

    // Step 1 — POST /v0/offramp { bolt11, address }. The service returns
    // the HTLC note skeleton (commitment fields), its timelock, the
    // payment hash and the quote expiry. `address` is our refund target.
    let http = reqwest::Client::new();
    let create_url = format!("{ln_service_uri}/v0/offramp");

    println!("\n⚡ Requesting escrow data from ln-service...");

    let resp = http
        .post(&create_url)
        .json(&serde_json::json!({
            "bolt11": invoice,
            "address": refund_address.to_string(),
        }))
        .send()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to reach ln-service: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(color_eyre::eyre::eyre!(
            "ln-service error {}: {}",
            status,
            body
        ));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse ln-service response: {}", e))?;

    let payment_hash = data["payment_hash"]
        .as_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing 'payment_hash' in ln-service response"))?
        .to_string();
    let expires_at = data["expires_at"].as_str().unwrap_or("unknown").to_string();

    // Reconstruct the escrow output note from the service's commitment
    // fields. We spend into this note verbatim -- its commitment must
    // match what the service expects to claim against.
    let note_json = &data["note"];
    let htlc_note = Note {
        utxo_kind: parse_element_field(note_json, "utxo_kind")?,
        note_kind: parse_element_field(note_json, "note_kind")?,
        address: parse_element_field(note_json, "address")?,
        psi: parse_element_field(note_json, "psi")?,
        value: parse_element_field(note_json, "value")?,
    };

    let timelock_json = &data["timelock"];
    let timelock = zk_primitives::TimeLock {
        zero_block: parse_timelock_hash(timelock_json, "zero_block")?,
        n_blocks: Element::new(
            timelock_json["n_blocks"]
                .as_u64()
                .ok_or_else(|| color_eyre::eyre::eyre!("missing or invalid 'n_blocks' in timelock"))?
        ),
    };

    println!("\n✅ Escrow data received from ln-service!");
    println!("   Payment hash: {payment_hash}");
    println!("   Note address: {}", htlc_note.address);
    println!("   Note psi:     {}", htlc_note.psi);
    println!(
        "   Note value:   {} sats",
        units::wei_to_sats(htlc_note.value.to_u64_array()[0])
    );
    println!("   Quote expiry: {expires_at}");
    println!("   Timelock anchor: {}", hex::encode(timelock.zero_block));
    println!("   Timelock blocks: {}", timelock.n_blocks);

    // Verify escrow note values against the Bolt11 invoice and wallet state
    println!("\n🔍 Verifying escrow note against Bolt11 invoice...");
    verify_escrow_note_values(
        invoice,
        &payment_hash,
        &htlc_note,
        &refund_address,
        chain,
        &timelock,
    ).await?;
    println!("✅ Escrow note values verified");

    println!("\n🔍 Verifying timelock matches current Bitcoin tip...");
    verify_timelock_matches_tip(timelock.zero_block, btc_explorer).await?;
    println!("✅ Timelock verified against Bitcoin tip");

    // Step 2 — commit funds by spending wallet notes into the escrow
    // note. Uses the Utxo Send circuit exactly like `escrow-lock`, but
    // the output note is dictated by the service rather than derived
    // locally, and we keep no preimage (the service is the redeemer).
    println!("\n   Committing funds to escrow...");

    let (prepared_wallet, utxo) = client
        .get_wallet()
        .prepare_escrow_lock_to_note(htlc_note.clone())?;

    let snark = utxo
        .prove()
        .map_err(|e| color_eyre::eyre::eyre!("utxo.prove() failed: {e}"))?;

    match client.transaction(&snark).await {
        Ok(tx) => {
            println!("\n✅ Escrow lock transaction {} submitted", tx.txn_hash);
            println!("   Height:    {}", tx.height);
            println!("   Root hash: {}", tx.root_hash);

            prepared_wallet.save()?;
            client.replace_wallet(prepared_wallet);

            // Persist the escrow note + refund key + timelock so the funds are
            // recoverable via `escrow-refund` if the service never claims.
            // The timelock is persisted via TimeProof serialization (only the lock,
            // not the headers which are filled in at refund time from real blocks).
            // Refund branch only: no preimage (zeroed).
            let refund_input_note = zk_primitives::EscrowInputNote {
                note: htlc_note.clone(),
                spend_type: 3,
                secret_key: refund_secret_key,
                preimage: [0u8; 32],
                time_proof: zk_primitives::TimeProof {
                    lock: timelock.clone(),
                    ..Default::default()
                },
            };
            let note_path = format!("{name}-htlc.json");
            let json_str = serde_json::to_string_pretty(&refund_input_note)?;
            std::fs::write(&note_path, json_str)?;
            println!("\nSaved refund EscrowInputNote to {note_path}");
            println!("  refund secret_key: {refund_secret_key}");
            println!("  refund address:    {refund_address}");
            println!("  timelock anchor:   {}", hex::encode(timelock.zero_block));
            println!("  timelock blocks:   {}", timelock.n_blocks);

            let ticker = citrea_ticker_from_contract(htlc_note.note_kind);
            let b = client.get_wallet().balance;
            println!("\nBalance {} sats {ticker}", units::wei_to_sats(b));
            println!(
                "\nThe ln-service will settle the Lightning invoice once the escrow confirms."
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not submit escrow lock transaction!");
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_mint(
    name: &str,
    host: &str,
    geth_rpc: &str,
    chain: u64,
    rollup: &str,
    secret: &str,
    amount_wei: u64,
    ticker: &str,
    only_snark: bool,
) -> Result<()> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;

    let (prepared_wallet, utxo) = client.get_wallet().prepare_mint(amount_wei, ticker)?;
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
            println!("\nBalance {} sats {ticker}", units::wei_to_sats(b));
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Could not send transaction!");
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_burn(
    name: &str,
    host: &str,
    chain: u64,
    address: &str,
    amount_wei: u64,
    ticker: &str,
    natively_substitute: bool,
) -> Result<(), AppError> {
    // Build client with fluent API
    let mut client = NodeClient::builder()
        .name(name)
        .host(host)
        .build(chain, false)?;

    // Prepare burn
    let (wallet_with_burner_key, burner_note) =
        client.get_wallet().prepare_receive_note(amount_wei, ticker);
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

async fn handle_release_slow_burn(
    geth_rpc: &str,
    chain: u64,
    rollup: &str,
    secret: &str,
    key_hex: &str,
    blocks: u64,
) -> Result<()> {
    use web3::ethabi::{Token, encode};
    use web3::signing::{SecretKey, keccak256};

    let key_bytes = hex::decode(key_hex.trim_start_matches("0x"))
        .map_err(|e| color_eyre::eyre::eyre!("invalid --key hex: {e}"))?;
    if key_bytes.len() != 32 {
        return Err(color_eyre::eyre::eyre!("--key must be 32 bytes"));
    }
    let target_key = H256::from_slice(&key_bytes);

    let client = contracts::Client::new(geth_rpc, None);
    let readonly = contracts::ReadonlyRollupContract::load(client.clone(), rollup).await?;

    let queued = readonly.get_all_slow_burn_queued_events(blocks).await?;
    let event = queued
        .into_iter()
        .find(|e| e.key == target_key)
        .ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "no SlowBurnQueued event found with key {target_key:#x} in the last scan window"
            )
        })?;

    println!("\nFound SlowBurnQueued");
    println!("\tKey         : {:#x}", event.key);
    println!("\tHash        : {:#x}", event.hash);
    println!("\tBurn Addr   : {:#x}", event.burn_addr);
    println!("\tAmount      : {} wei", event.amount);
    println!("\tReady At    : {} (unix)", event.ready_at);
    println!("\tEmitted Tx  : {:#x}", event.transaction_hash);

    let height = readonly
        .rollup_verified_height_in_tx(event.transaction_hash)
        .await?
        .ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "no RollupVerified event in tx {:#x}; cannot recover rollup height",
                event.transaction_hash
            )
        })?;
    println!("\tRollup Height (from RollupVerified): {height}");

    // The contract's `getSubstituteBurnKey` is
    //   keccak256(abi.encode(hash, burnAddr, noteKind, amount, height))
    // The event only carries hash, burnAddr, amount; brute-force the
    // remaining note_kind across the supported tokens by recomputing the
    // key and matching it against the indexed `key` topic.
    // Derive the network from the `--chain` argument so the WCBTC note
    // kind we brute-force against matches the chain we're actually talking
    // to, instead of a hardcoded enum.
    let network = cli::address::network_for_chain(chain);
    let candidate_kinds: Vec<(&str, H256)> = supported_citrea_tokens(network)
        .into_iter()
        .map(|(ticker, note_kind)| (ticker, convert_element_to_h256(&note_kind)))
        .collect();

    let mut matched: Option<(&str, H256)> = None;
    for (ticker, kind) in &candidate_kinds {
        let encoded = encode(&[
            Token::FixedBytes(event.hash.as_bytes().to_vec()),
            Token::Address(event.burn_addr),
            Token::FixedBytes(kind.as_bytes().to_vec()),
            Token::Uint(event.amount),
            Token::Uint(height),
        ]);
        let computed = H256::from(keccak256(&encoded));
        if computed == event.key {
            matched = Some((ticker, *kind));
            break;
        }
    }
    let (ticker, note_kind) = matched.ok_or_else(|| {
        color_eyre::eyre::eyre!(
            "could not match any known note_kind (WCBTC/CUSD) against event key {:#x}; \
             contract may be using an unsupported token",
            event.key
        )
    })?;
    println!("\tNote Kind   : {note_kind:#x} ({ticker})");

    let sk = SecretKey::from_str(secret)
        .map_err(|e| color_eyre::eyre::eyre!("invalid --secret: {e}"))?;
    let signed = contracts::SignedRollupContract::load(client, &chain, rollup, sk).await?;

    println!("\nCalling releaseSlowBurn ...");
    let tx = signed
        .release_slow_burn(event.hash, event.burn_addr, note_kind, event.amount, height)
        .await?;
    println!("\nSubmitted releaseSlowBurn tx {tx:#x}");

    Ok(())
}

async fn handle_rollup(geth_rpc: &str, chain: u64, rollup: &str, blocks: u64) -> Result<()> {
    let client = contracts::Client::new(geth_rpc, None);
    let rollup = contracts::ReadonlyRollupContract::load(client, rollup).await?;

    let rh = rollup.root_hash().await?;
    let b = rollup.block_height().await?;
    let tokens = supported_citrea_tokens(cli::address::network_for_chain(chain));

    println!("\nRollup State Info\n");
    println!("\tChain                :{chain} ");
    for (ticker, note_kind) in tokens {
        let token = rollup.token(convert_element_to_h256(&note_kind)).await?;
        println!("\tToken kind {ticker:<9}:{token:#x} ");
    }
    println!("\tBlock                :{b} ");
    println!("\tRoot hash            :{rh:#x} ");

    // Enumerate zkVerifierKeys array and look up each entry in the zkVerifiers mapping
    println!("\nZK Verifiers\n");
    let mut index = 0u64;
    while let Ok(key_hash) = rollup.zk_verifier_keys(U256::from(index)).await {
        if let Ok((address, circuit_id, enabled)) = rollup.zk_verifiers(key_hash).await {
            println!(
                "\t[{index}]\n\tkey={key_hash:#x}\n\taddress={address:#x}\n\t\
                        circuit_id={circuit_id}  enabled={enabled}"
            );
        }
        index += 1;
    }
    if index == 0 {
        println!("\tNo ZK verifiers found.");
    }

    // Last mint events
    println!("\nLast Mint Events\n");
    let mint_events = rollup.get_all_mint_added_events(blocks).await?;
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

    println!("\nSlow Burn Queued\n");
    let queued = rollup.get_all_slow_burn_queued_events(blocks).await?;
    println!("\tTotal queued: {}\n", queued.len());
    if queued.is_empty() {
        println!("\tNo SlowBurnQueued events found.");
    } else {
        println!(
            "\t{:<66}  {:<66}  {:<42}  {:>20}  {:>12}  Block",
            "Key", "Hash", "Burn Addr", "Amount (wei)", "Ready At"
        );
        for event in &queued {
            println!(
                "\t{:#x}  {:#x}  {:#x}  {:>20}  {:>12}  {}",
                event.key,
                event.hash,
                event.burn_addr,
                event.amount,
                event.ready_at,
                event.block_number
            );
        }
    }

    println!("\nSlow Burn Released\n");
    let released = rollup.get_all_slow_burn_released_events(blocks).await?;
    println!("\tTotal released: {}\n", released.len());
    if released.is_empty() {
        println!("\tNo SlowBurnReleased events found.");
    } else {
        println!(
            "\t{:<66}  {:<42}  {:>11}  {:>8}  Block",
            "Key", "Recipient", "Substituted", "Success"
        );
        for event in &released {
            println!(
                "\t{:#x}  {:#x}  {:>11}  {:>8}  {}",
                event.key, event.recipient, event.substituted, event.success, event.block_number
            );
        }
    }

    Ok(())
}

async fn handle_mints(geth_rpc: &str, chain: u64, rollup: &str, blocks: u64) -> Result<()> {
    let client = contracts::Client::new(geth_rpc, None);
    let rollup = contracts::ReadonlyRollupContract::load(client, rollup).await?;

    let events = rollup.get_all_mint_added_events(blocks).await?;

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
    let mut cli = Cli::parse();

    // Initialize logging
    init_logging(cli.verbose);

    debug!("Starting Ciphera CLI");

    // An empty `--rollup=` on the command line overrides clap's default
    // with the empty string and then blows up downstream in hex parsing.
    // Treat empty as "fall back to the default address".
    if cli.rollup.trim().is_empty() {
        cli.rollup = "0xbd57b7d47d66934509f9ca31248598eb6cb3fafd".to_string();
    }

    // Execute command
    match cli.command {
        Commands::Create {} => {
            handle_create(cli.chain, &cli.name).await?;
        }
        Commands::Sync {} => {
            handle_sync(cli.chain, &cli.name, &cli.host).await?;
        }
        Commands::Address { amount_sat, ticker } => {
            let ticker_normalized = parse_cli_ticker(&ticker)?;
            let amount_wei = units::sats_to_wei(amount_sat);
            handle_address(&cli.name, amount_wei, ticker_normalized).await?;
        }
        Commands::Spend { amount_sat, ticker } => {
            let ticker_normalized = parse_cli_ticker(&ticker)?;
            let amount_wei = units::sats_to_wei(amount_sat);
            handle_note_spend(
                cli.chain,
                &cli.name,
                &cli.host,
                amount_wei,
                ticker_normalized,
            )
            .await?;
        }
        Commands::SpendTo { address } => {
            handle_spend_to(
                cli.chain,
                &cli.name,
                &cli.host,
                &address,
            )
            .await?;
        }
        Commands::Receive { note, link } => {
            handle_receive(
                &cli.name,
                &cli.host,
                cli.chain,
                note,
                link,
            )
            .await?;
        }
        Commands::Import { note } => {
            handle_import(&cli.name, &note).await?;
        }
        Commands::EscrowLock {
            amount_sat,
            ticker,
            preimage,
        } => {
            let ticker_normalized = parse_cli_ticker(&ticker)?;
            let amount_wei = units::sats_to_wei(amount_sat);
            handle_escrow_lock(
                cli.chain,
                &cli.name,
                &cli.host,
                amount_wei,
                ticker_normalized,
                &preimage,
                &cli.btc_explorer,
            )
            .await?;
        }
        Commands::EscrowRedeem { note } => {
            handle_escrow_redeem_or_refund(
                cli.chain,
                &cli.name,
                &cli.host,
                &note,
                false,
                &cli.btc_explorer,
            )
            .await?;
        }
        Commands::EscrowRefund { note } => {
            handle_escrow_redeem_or_refund(
                cli.chain,
                &cli.name,
                &cli.host,
                &note,
                true,
                &cli.btc_explorer,
            )
            .await?;
        }
        Commands::Mint {
            geth_rpc,
            secret,
            amount_sat,
            ticker,
            only_snark,
        } => {
            let ticker_normalized = parse_cli_ticker(&ticker)?;
            let amount_wei = units::sats_to_wei(amount_sat);
            handle_mint(
                &cli.name,
                &cli.host,
                &geth_rpc,
                cli.chain,
                &cli.rollup,
                &secret,
                amount_wei,
                ticker_normalized,
                only_snark,
            )
            .await?;
        }
        Commands::Burn {
            address,
            amount_sat,
            ticker,
        } => {
            let ticker_normalized = parse_cli_ticker(&ticker)?;
            let amount_wei = units::sats_to_wei(amount_sat);
            handle_burn(
                &cli.name,
                &cli.host,
                cli.chain,
                &address,
                amount_wei,
                ticker_normalized,
                true,
            )
            .await?;
        }
        Commands::Contract { geth_rpc, blocks } => {
            handle_rollup(&geth_rpc, cli.chain, &cli.rollup, blocks).await?;
        }
        Commands::Mints { geth_rpc, blocks } => {
            handle_mints(&geth_rpc, cli.chain, &cli.rollup, blocks).await?;
        }
        Commands::ReleaseSlowBurn {
            geth_rpc,
            secret,
            key,
            blocks,
        } => {
            handle_release_slow_burn(&geth_rpc, cli.chain, &cli.rollup, &secret, &key, blocks)
                .await?;
        }
        Commands::AtomiqDepo {
            amount_sat,
            onramp_uri,
        } => {
            handle_atomiq_depo(
                &cli.name,
                &cli.host,
                cli.chain,
                amount_sat,
                &onramp_uri,
            )
            .await?;
        }
        Commands::DepoLn {
            amount_sat,
            ln_service_uri,
        } => {
            handle_depo_ln(
                &cli.name,
                &cli.host,
                cli.chain,
                amount_sat,
                &ln_service_uri,
            )
            .await?;
        }
        Commands::AtomiqWithdraw {
            invoice,
            substitutor,
            address,
            offramp_uri,
        } => {
            handle_atomiq_withdraw(
                &cli.name,
                &cli.host,
                cli.chain,
                &invoice,
                &substitutor,
                &address,
                &offramp_uri,
            )
            .await?;
        }
        Commands::WithdrawLn {
            invoice,
            ln_service_uri,
        } => {
            handle_withdraw_ln(
                &cli.name,
                &cli.host,
                cli.chain,
                &invoice,
                &ln_service_uri,
                &cli.btc_explorer,
            )
            .await?;
        }
    }

    println!("\n");
    Ok(())
}
