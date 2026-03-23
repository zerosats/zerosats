use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "autoplayer")]
#[command(about = "Ciphera test environment auto-player — generates randomized transaction traffic")]
pub struct Args {
    /// Ciphera node RPC host
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Ciphera node RPC port
    #[arg(long, default_value = "8091")]
    pub port: u16,

    /// Citrea EVM RPC URL (for mint bridge operations)
    #[arg(long, default_value = "http://127.0.0.1:12345")]
    pub evm_rpc_url: String,

    /// Chain ID
    #[arg(long, default_value = "5655")]
    pub chain_id: u64,

    /// Rollup contract address
    #[arg(long)]
    pub rollup_contract: String,

    /// EVM private key for mints (Hardhat account 0 — unlimited funds)
    #[arg(long, default_value = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")]
    pub evm_secret: String,

    /// Number of wallets in the pool
    #[arg(long, default_value = "4")]
    pub wallet_count: usize,

    /// RNG seed for deterministic replay. 0 = random seed.
    #[arg(long, default_value = "0")]
    pub seed: u64,

    /// Minimum delay between actions (ms)
    #[arg(long, default_value = "5000")]
    pub min_delay_ms: u64,

    /// Maximum delay between actions (ms)
    #[arg(long, default_value = "30000")]
    pub max_delay_ms: u64,

    /// Weight for mint actions (out of 100)
    #[arg(long, default_value = "25")]
    pub weight_mint: u32,

    /// Weight for spend+receive actions (always paired)
    #[arg(long, default_value = "35")]
    pub weight_spend: u32,

    /// Weight for burn actions
    #[arg(long, default_value = "20")]
    pub weight_burn: u32,

    /// Weight for fault injection actions
    #[arg(long, default_value = "10")]
    pub weight_fault: u32,

    /// Weight for self-spend (UTXO churn)
    #[arg(long, default_value = "10")]
    pub weight_self_spend: u32,

    /// Minimum transaction amount
    #[arg(long, default_value = "1000")]
    pub min_amount: u64,

    /// Maximum transaction amount
    #[arg(long, default_value = "100000")]
    pub max_amount: u64,

    /// Directory to store wallet files
    #[arg(long, default_value = "/tmp/autoplayer-wallets")]
    pub wallet_dir: String,
}
