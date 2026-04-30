use std::path::{Path, PathBuf};

use eyre::{Context, Result, eyre};
use web3::signing::SecretKey;

/// CLI options for unlocking a geth-format keystore.
///
/// The password is resolved in this order: a file (`--keystore-password-file`),
/// an environment variable (`--keystore-password-env`), then an interactive
/// prompt on the controlling TTY.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct KeystoreOpts {
    /// Path to a geth-format encrypted JSON keystore file
    #[arg(long, env = "ETH_KEYSTORE_PATH")]
    pub keystore_path: Option<PathBuf>,

    /// Path to a file whose first line is the keystore password
    #[arg(long, env = "ETH_KEYSTORE_PASSWORD_FILE")]
    pub keystore_password_file: Option<PathBuf>,

    /// Name of an environment variable that holds the keystore password
    #[arg(long, env = "ETH_KEYSTORE_PASSWORD_ENV")]
    pub keystore_password_env: Option<String>,

    /// Console password input flag
    #[arg(long, default_value = "false")]
    pub allow_password_input: bool,
}

impl KeystoreOpts {
    pub fn is_set(&self) -> bool {
        if self.allow_password_input {
            self.keystore_path.is_some()
        } else {
            self.keystore_path.is_some()
                && (self.keystore_password_file.is_some() || self.keystore_password_env.is_some())
        }
    }

    /// Decrypt the configured keystore and return a `web3::signing::SecretKey`.
    pub fn unlock(&self) -> Result<SecretKey> {
        let path = self
            .keystore_path
            .as_ref()
            .ok_or_else(|| eyre!("keystore path is not set"))?;
        let password = self.read_password(path)?;
        unlock_keystore(path, password.as_str())
    }

    fn read_password(&self, path: &Path) -> Result<String> {
        if let Some(file) = &self.keystore_password_file {
            let raw = std::fs::read_to_string(file)
                .with_context(|| format!("reading keystore password file {}", file.display()))?;
            return Ok(raw.trim_end_matches(['\n', '\r']).to_string());
        }

        if let Some(name) = &self.keystore_password_env {
            return std::env::var(name)
                .with_context(|| format!("reading keystore password from env var {name}"));
        }

        if self.allow_password_input {
            return rpassword::prompt_password(format!(
                "Enter password to unlock keystore {}: ",
                path.display()
            ))
            .context("reading keystore password from terminal");
        }

        Err(eyre!(
            "impossible to read unlock password. Check your launch parameters"
        ))
    }
}

/// Decrypt a geth-format keystore file and return its secret key.
pub fn unlock_keystore<P: AsRef<Path>>(path: P, password: &str) -> Result<SecretKey> {
    let path = path.as_ref();
    let bytes = eth_keystore::decrypt_key(path, password)
        .with_context(|| format!("decrypting keystore at {}", path.display()))?;
    SecretKey::from_slice(&bytes)
        .map_err(|e| eyre!("decrypted keystore is not a valid secp256k1 key: {e}"))
}
