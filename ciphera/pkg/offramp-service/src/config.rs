use clap::Parser;
use element::Element;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(
    name = "offramp-service",
    about = "Lightning offramp gateway: hands out HTLC note skeletons, pays bolt11s, redeems via SlowBurn"
)]
pub struct Config {
    /// TCP bind address for the HTTP server.
    #[arg(long, env = "BIND", default_value = "127.0.0.1:7080")]
    pub bind: String,

    /// External rollup node base URL (must serve /v0/transaction, /v0/elements, /v0/transactions).
    #[arg(long, env = "ROLLUP_URL", default_value = "http://localhost:8080")]
    pub rollup_url: String,

    /// phoenixd HTTP API base URL.
    #[arg(long, env = "PHOENIXD_URL", default_value = "http://localhost:9740")]
    pub phoenixd_url: String,

    /// phoenixd API password (HTTP basic auth secret).
    #[arg(long, env = "PHOENIXD_API_PASSWORD")]
    pub phoenixd_api_password: String,

    /// mempool.space base URL. Default targets mainnet.
    #[arg(long, env = "MEMPOOL_URL", default_value = "https://mempool.space")]
    pub mempool_url: String,

    /// sqlite database file path.
    #[arg(long, env = "DB_PATH", default_value = "offramp.sqlite")]
    pub db_path: String,

    /// Default note kind to use when the client does not specify one
    /// (e.g. Citrea wrapped BTC). Plain hex Element.
    #[arg(long, env = "CIPHERA_BTC_NOTE_KIND", value_parser = parse_element)]
    pub ciphera_btc_note_kind: Element,

    /// EVM address (20-byte hex, with or without 0x prefix) that SlowBurn
    /// payouts will be routed to.
    #[arg(long, env = "SERVICE_EVM_ADDRESS", value_parser = parse_element)]
    pub service_evm_address: Element,

    /// Number of Bitcoin blocks after the anchor before the user's refund
    /// branch opens. Plan §2 pins this to 2 for MVP.
    #[arg(long, env = "TIMELOCK_N_BLOCKS", default_value_t = 2)]
    pub timelock_n_blocks: u64,

    /// Quote TTL in seconds; quotes that never see escrow are auto-cancelled
    /// after this window.
    #[arg(long, env = "QUOTE_TTL_SECONDS", default_value_t = 3600)]
    pub quote_ttl_seconds: i64,

    /// Maximum bolt11 amount accepted per quote, in sats.
    #[arg(long, env = "MAX_AMOUNT_SAT", default_value_t = 1_000_000)]
    pub max_amount_sat: u64,

    /// Settlement worker tick interval in milliseconds.
    #[arg(long, env = "WORKER_TICK_MS", default_value_t = 2_000)]
    pub worker_tick_ms: u64,
}

fn parse_element(s: &str) -> Result<Element, String> {
    Element::from_str(s).map_err(|e| format!("invalid Element {s:?}: {e}"))
}
