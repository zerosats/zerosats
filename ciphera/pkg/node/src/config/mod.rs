use std::path::PathBuf;

use self::cli::CliArgs;
use crate::Mode;
use color_eyre::{Result, eyre::eyre};
use dirs::home_dir;
use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use primitives::peer::PeerIdSigner;
use secp256k1::SecretKey;
use serde::Deserialize;
use std::io::Read;
use std::{fs::File, str::FromStr};

pub mod cli;

// TODO: should we use kebab-case? Currently _ is used to split into
// multiple level dictionaries
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    /// Sentry DSN URL
    pub sentry_dsn: Option<String>,

    /// Sentry environment name
    pub env_name: String,

    /// Maximum number of txns to include in a block
    pub block_txns_count: usize,

    /// Minimum block duration in seconds
    pub min_block_duration: usize,

    pub sync_chunk_size: u64,

    pub sync_timeout_ms: u64,

    pub fast_sync_threshold: u64,

    pub mode: Mode,

    /// Private key of validator. Resolved at startup from either a geth
    /// keystore (preferred), the `--secret-key` CLI flag, or the
    /// `--secret-key-path` file. Always `Some` once `Config::from_env`
    /// returns successfully — use `Config::signer()` to access it.
    #[serde(default)]
    pub secret_key: Option<PeerIdSigner>,

    /// RPC config
    pub rpc_laddr: String,

    /// P2P config
    pub p2p: ::p2p2::Config,

    /// Path to the database
    pub db_path: PathBuf,

    /// Path to Smirk
    pub smirk_path: PathBuf,

    pub evm_rpc_url: String,

    pub chain_id: u64,

    pub rollup_contract_addr: String,

    /// If the last commit is older than this, health check will fail
    pub health_check_commit_interval_sec: u64,

    pub rollup_wait_time_ms: u64,

    /// Optional postgres database for synchronization between provers
    pub prover_database_url: Option<String>,

    /// Blocks that should not be validated or rolled up
    pub bad_blocks: Vec<u64>,

    /// The minimum amount of gas (in gwei) to use for transactions
    pub minimum_gas_price_gwei: Option<u64>,

    pub safe_eth_height_offset: u64,
}

impl Config {
    /// The text of the default config string
    pub const DEFAULT_STR: &str = include_str!("./default_config.toml");

    /// Get the resolved validator signing key.
    ///
    /// `Config::from_env` always populates `secret_key` (or returns an error),
    /// so this is infallible after a successful `from_env`.
    pub fn signer(&self) -> &PeerIdSigner {
        self.secret_key
            .as_ref()
            .expect("secret_key must be resolved by Config::from_env")
    }

    /// Load a [`Config`] from a file and environment
    ///
    /// `config_path` doesn't need to point to an actual file
    pub fn from_env(args: CliArgs) -> Result<Self> {
        let mut config: Config = Figment::new()
            .merge(Toml::file(args.config_path))
            .merge(
                Env::prefixed("CIPHERA_")
                    .split("__")
                    .map(|k| k.as_str().replace('_', "-").into()),
            )
            .join(Toml::string(Self::DEFAULT_STR))
            .extract()?;

        if let Some(mode) = args.mode {
            config.mode = mode;
        }

        if let Some(p2p_laddr) = args.p2p_laddr {
            config.p2p.laddr = p2p_laddr;
        }

        if let Some(p2p_dial) = args.p2p_dial {
            config.p2p.dial = p2p_dial;
        }

        if let Some(secret_key_path) = args.secret_key_path {
            let mut file = File::open(secret_key_path)?;
            let mut key = String::new();
            file.read_to_string(&mut key)?;
            config.secret_key = Some(PeerIdSigner::from_str(key.trim())?);
        }

        if let Some(secret_key) = args.secret_key {
            config.secret_key = Some(secret_key);
        }

        if args.keystore.is_set() {
            let unlocked = args.keystore.unlock()?;
            // web3's signing::SecretKey wraps secp256k1@0.27, so convert via raw bytes
            // to the workspace's secp256k1@0.28 SecretKey that PeerIdSigner expects.
            let bytes = unlocked.secret_bytes();
            let secret_key = SecretKey::from_slice(&bytes)
                .map_err(|e| eyre!("decrypted keystore is not a valid secp256k1 key: {e}"))?;
            config.secret_key = Some(PeerIdSigner::new(secret_key));
        }

        if config.secret_key.is_none() {
            return Err(eyre!(
                "no signing key configured: pass --keystore-path (with a password file/env, \
                 or be prompted interactively), --secret-key, or --secret-key-path"
            ));
        }

        if let Some(rpc_laddr) = args.rpc_laddr {
            config.rpc_laddr = rpc_laddr;
        }

        if let Some(db_path) = args.db_path {
            config.db_path = db_path;
        }

        if config.db_path.starts_with("~") {
            config.db_path = home_dir()
                .unwrap()
                .join(config.db_path.strip_prefix("~").unwrap());
        }

        if let Some(smirk_path) = args.smirk_path {
            config.smirk_path = smirk_path;
        }

        if config.smirk_path.starts_with("~") {
            config.smirk_path = home_dir()
                .unwrap()
                .join(config.smirk_path.strip_prefix("~").unwrap());
        }

        if let Some(evm_rpc_url) = args.evm_rpc_url {
            config.evm_rpc_url = evm_rpc_url;
        }

        if let Some(chain_id) = args.chain_id {
            config.chain_id = chain_id;
        }

        if let Some(rollup_contract_addr) = args.rollup_contract_addr {
            config.rollup_contract_addr = rollup_contract_addr;
        }

        if let Some(sync_chunk_size) = args.sync_chunk_size {
            config.sync_chunk_size = sync_chunk_size;
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    /// Create a geth-format keystore file in `dir` containing `key_bytes`
    /// encrypted with `password`. Returns the full path to the keystore file.
    fn make_keystore(dir: &tempdir::TempDir, key_bytes: &[u8], password: &str) -> PathBuf {
        let mut rng = rand::thread_rng();
        let filename = eth_keystore::new(dir.path(), &mut rng, key_bytes, password, None)
            .expect("failed to create test keystore");
        dir.path().join(filename)
    }

    #[test]
    fn can_parse_with_secret_key() {
        let args = CliArgs::try_parse_from([
            "node",
            "--secret-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        ])
        .unwrap();
        Config::from_env(args).unwrap();
    }

    #[test]
    fn errors_when_no_signing_key_provided() {
        let args = CliArgs::try_parse_from(["node"]).unwrap();
        let err = Config::from_env(args).unwrap_err();
        assert!(
            err.to_string().contains("no signing key configured"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn can_load_keystore_with_password_file() {
        // Hardhat account #0 key (well-known test value)
        const KEY_HEX: &str =
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let key_bytes = hex::decode(KEY_HEX).unwrap();
        let expected_key = SecretKey::from_slice(&key_bytes).unwrap();

        let dir = tempdir::TempDir::new("keystore-test").unwrap();
        let keystore_path = make_keystore(&dir, &key_bytes, "hunter2");

        let pass_file = dir.path().join("password.txt");
        std::fs::write(&pass_file, "hunter2").unwrap();

        let args = CliArgs::try_parse_from([
            "node",
            "--keystore-path",
            keystore_path.to_str().unwrap(),
            "--keystore-password-file",
            pass_file.to_str().unwrap(),
        ])
        .unwrap();

        let config = Config::from_env(args).unwrap();
        assert_eq!(
            config.signer().secret_key().secret_bytes(),
            expected_key.secret_bytes(),
        );
    }

    #[test]
    fn keystore_wrong_password_returns_error() {
        const KEY_HEX: &str =
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let key_bytes = hex::decode(KEY_HEX).unwrap();

        let dir = tempdir::TempDir::new("keystore-test").unwrap();
        let keystore_path = make_keystore(&dir, &key_bytes, "correct-password");

        let pass_file = dir.path().join("password.txt");
        std::fs::write(&pass_file, "wrong-password").unwrap();

        let args = CliArgs::try_parse_from([
            "node",
            "--keystore-path",
            keystore_path.to_str().unwrap(),
            "--keystore-password-file",
            pass_file.to_str().unwrap(),
        ])
        .unwrap();

        let err = Config::from_env(args).unwrap_err();
        assert!(
            err.to_string().contains("decrypting keystore"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn keystore_takes_precedence_over_secret_key_flag() {
        // Hardhat account #1 — the key stored in the keystore
        const KEYSTORE_KEY_HEX: &str =
            "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
        let key_bytes = hex::decode(KEYSTORE_KEY_HEX).unwrap();
        let expected_key = SecretKey::from_slice(&key_bytes).unwrap();

        let dir = tempdir::TempDir::new("keystore-test").unwrap();
        let keystore_path = make_keystore(&dir, &key_bytes, "pass");

        let pass_file = dir.path().join("password.txt");
        std::fs::write(&pass_file, "pass").unwrap();

        // Supply a different key via --secret-key; the keystore should win.
        let args = CliArgs::try_parse_from([
            "node",
            "--secret-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--keystore-path",
            keystore_path.to_str().unwrap(),
            "--keystore-password-file",
            pass_file.to_str().unwrap(),
        ])
        .unwrap();

        let config = Config::from_env(args).unwrap();
        assert_eq!(
            config.signer().secret_key().secret_bytes(),
            expected_key.secret_bytes(),
        );
    }
}
