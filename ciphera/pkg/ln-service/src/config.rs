use clap::{Parser, ValueEnum};
use element::Element;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use zk_primitives::{CitreaNetwork, citrea_wcbtc_note_kind};

/// CLI-friendly mirror of [`CitreaNetwork`]. Kept separate so clap can derive
/// `ValueEnum` without leaking that derive into the shared primitives crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum CitreaNetworkArg {
    Testnet,
    Mainnet,
}

impl From<CitreaNetworkArg> for CitreaNetwork {
    fn from(value: CitreaNetworkArg) -> Self {
        match value {
            CitreaNetworkArg::Testnet => Self::Testnet,
            CitreaNetworkArg::Mainnet => Self::Mainnet,
        }
    }
}

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(
    name = "ln-service",
    about = "Lightning offramp gateway: hands out HTLC note skeletons, pays bolt11s, redeems via SlowBurn"
)]
pub struct Config {
    /// TCP bind address for the HTTP server.
    #[arg(long, env = "BIND", default_value = "127.0.0.1:7080")]
    pub bind: String,

    /// External Ciphera node base URL (must serve /v0/transaction, /v0/elements, /v0/transactions).
    #[arg(long, env = "CIPHERA_URL", default_value = "http://localhost:8080")]
    pub ciphera_url: String,

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

    /// Enable the offramp endpoints (`/v0/offramp*`) and its settlement
    /// worker. Pass `--offramp false` to run an onramp-only gateway.
    #[arg(long, env = "OFFRAMP", action = clap::ArgAction::Set, default_value_t = true)]
    pub offramp: bool,

    /// Enable the onramp endpoints (`/v0/onramp*`) and its settlement
    /// worker. Pass `--onramp false` to run an offramp-only gateway.
    #[arg(long, env = "ONRAMP", action = clap::ArgAction::Set, default_value_t = true)]
    pub onramp: bool,

    /// Base-layer Citrea chain id, matching the node binary's `--chain-id`
    /// / `CIPHERA_BASE_CHAIN_ID` (testnet 5115, mainnet 4114). When set,
    /// the default WCBTC note kind is derived from it. Takes precedence
    /// over `--citrea-network`; an explicit `--ciphera-btc-note-kind`
    /// still wins over both.
    #[arg(long, env = "CIPHERA_BASE_CHAIN_ID")]
    pub chain_id: Option<u64>,

    /// Citrea network the service is bound to. Used to derive the default
    /// WCBTC note kind when `--ciphera-btc-note-kind` is not provided.
    /// Mutually exclusive with passing the raw note kind, but harmless when
    /// both are set (the explicit value wins).
    #[arg(long, env = "CITREA_NETWORK", value_enum)]
    pub citrea_network: Option<CitreaNetworkArg>,

    /// Explicit override for the default note kind. Plain hex Element. If
    /// unset, derived from `--citrea-network` via
    /// `zk_primitives::citrea_wcbtc_note_kind`. Exactly one of the two must
    /// be supplied.
    #[arg(long, env = "CIPHERA_BTC_NOTE_KIND", value_parser = parse_element)]
    pub ciphera_btc_note_kind: Option<Element>,

    /// EVM address (20-byte hex, with or without 0x prefix) that SlowBurn
    /// payouts will be routed to.
    #[arg(long, env = "SERVICE_EVM_ADDRESS", value_parser = parse_element)]
    pub service_evm_address: Element,

    /// Secret key (32-byte hex Element) used to claim HTLC escrow notes.
    /// `Poseidon([service_secret_key, 0])` is the value users must bake
    /// into the HTLC note address at quote/lock time -- the offramp's
    /// public claim key, distributed alongside `service_evm_address`.
    /// Keep this secret: anyone who learns it can claim any HTLC bound
    /// to this service.
    #[arg(long, env = "SERVICE_SECRET_KEY", value_parser = parse_element)]
    pub service_secret_key: Element,

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

impl Config {
    /// Resolve the WCBTC note kind the service should hand out by default.
    /// Resolution order: explicit `--ciphera-btc-note-kind`, then
    /// `--chain-id` (mapped via `CitreaNetwork::try_from_chain_id`), then
    /// `--citrea-network`. Returns an error if none is configured, or if a
    /// `--chain-id` is given that isn't a known Citrea chain.
    pub fn resolved_default_note_kind(&self) -> Result<Element, String> {
        if let Some(explicit) = self.ciphera_btc_note_kind {
            return Ok(explicit);
        }
        if let Some(chain_id) = self.chain_id {
            let net = CitreaNetwork::try_from_chain_id(chain_id).ok_or_else(|| {
                format!(
                    "unsupported --chain-id {chain_id}; expected a known Citrea chain \
                     (testnet 5115 / mainnet 4114)"
                )
            })?;
            return Ok(citrea_wcbtc_note_kind(net));
        }
        match self.citrea_network {
            Some(net) => Ok(citrea_wcbtc_note_kind(net.into())),
            None => Err(
                "one of --ciphera-btc-note-kind, --chain-id or --citrea-network must be supplied"
                    .to_string(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton(
        chain_id: Option<u64>,
        net: Option<CitreaNetworkArg>,
        explicit: Option<Element>,
    ) -> Config {
        Config {
            bind: "x".into(),
            ciphera_url: "x".into(),
            phoenixd_url: "x".into(),
            phoenixd_api_password: "x".into(),
            mempool_url: "x".into(),
            db_path: "x".into(),
            offramp: true,
            onramp: true,
            chain_id,
            citrea_network: net,
            ciphera_btc_note_kind: explicit,
            service_evm_address: Element::ZERO,
            service_secret_key: Element::ZERO,
            timelock_n_blocks: 2,
            quote_ttl_seconds: 600,
            max_amount_sat: 1,
            worker_tick_ms: 1,
        }
    }

    #[test]
    fn resolved_note_kind_prefers_explicit_override() {
        let cfg = skeleton(Some(4114), Some(CitreaNetworkArg::Mainnet), Some(Element::new(42)));
        assert_eq!(cfg.resolved_default_note_kind().unwrap(), Element::new(42));
    }

    #[test]
    fn resolved_note_kind_derives_from_chain_id() {
        let cfg = skeleton(Some(5115), None, None);
        let expected = citrea_wcbtc_note_kind(CitreaNetwork::Testnet);
        assert_eq!(cfg.resolved_default_note_kind().unwrap(), expected);
    }

    #[test]
    fn resolved_note_kind_chain_id_takes_precedence_over_network() {
        // chain id maps to testnet even though the network arg says mainnet.
        let cfg = skeleton(Some(5115), Some(CitreaNetworkArg::Mainnet), None);
        let expected = citrea_wcbtc_note_kind(CitreaNetwork::Testnet);
        assert_eq!(cfg.resolved_default_note_kind().unwrap(), expected);
    }

    #[test]
    fn resolved_note_kind_errors_on_unknown_chain_id() {
        let cfg = skeleton(Some(1), None, None);
        assert!(cfg.resolved_default_note_kind().is_err());
    }

    #[test]
    fn resolved_note_kind_derives_from_testnet_network() {
        let cfg = skeleton(None, Some(CitreaNetworkArg::Testnet), None);
        let expected = citrea_wcbtc_note_kind(CitreaNetwork::Testnet);
        assert_eq!(cfg.resolved_default_note_kind().unwrap(), expected);
    }

    #[test]
    fn resolved_note_kind_derives_from_mainnet_network() {
        let cfg = skeleton(None, Some(CitreaNetworkArg::Mainnet), None);
        let expected = citrea_wcbtc_note_kind(CitreaNetwork::Mainnet);
        assert_eq!(cfg.resolved_default_note_kind().unwrap(), expected);
    }

    #[test]
    fn resolved_note_kind_errors_when_neither_supplied() {
        let cfg = skeleton(None, None, None);
        assert!(cfg.resolved_default_note_kind().is_err());
    }
}
