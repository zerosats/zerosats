use contracts::{util::convert_h160_to_element, Address, RollupContract, SecretKey, USDCContract};
use element::Element;
use hash::hash_merge;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use web3::types::H160;
use zk_primitives::Utxo;
use zk_primitives::{
    generate_note_kind_bridge_evm, get_address_for_private_key, InputNote, MerklePath, Note,
};

// Error types for wallet operations
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Wallet file not found: {0}")]
    FileNotFound(String),

    #[error("No data in wallet file: {0}")]
    KeyNotFound(String),

    #[error("Unable to read secret: {0}")]
    CouldNotReadKey(#[from] ParseIntError),

    #[error("No coins left in wallet {0}")]
    ZeroBalance(String),
}

// =====================================================================
// Wallet & helpers
// =====================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    /// *Private* key in the zk‑Primitive sense – **NOT** an ECDSA key!
    pub pk: Element,
    pub spent: Vec<Note>,
    pub avail: Vec<Note>,
    pub name: Option<String>,
    pub balance: u64,
}

impl Wallet {
    /// Create a wallet from an explicit private key.
    pub fn new(name: Option<String>, pk: Element) -> Self {
        Self {
            pk,
            spent: Vec::new(),
            avail: Vec::new(),
            name: None,
            balance: 0,
        }
    }

    /// Create a wallet with a random 256‑bit private key.
    pub fn random(name: Option<String>) -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self {
            pk: Element::from_be_bytes(bytes),
            spent: Vec::new(),
            avail: Vec::new(),
            name,
            balance: 0,
        }
    }

    /// Load wallet from JSON file
    pub fn init(name: &str) -> Result<Self, WalletError> {
        let file = format!("{}.json", name);
        let wallet_file = Path::new(&file);

        if wallet_file.is_file() {
            let json_str = fs::read_to_string(&wallet_file)?;
            Ok(serde_json::from_str(&json_str)?)
        } else {
            let mut wallet = Self::random(Some(name.to_string()));
            wallet.save()?;
            Ok(wallet)
        }
    }

    /// Save wallet to JSON file (uses configured path or provided path)
    pub fn save(&self) -> Result<(), WalletError> {
        if let Some(name) = &self.name {
            let file = format!("{}.json", name);
            let path = Path::new(&file);
            self.save_to(path)
        } else {
            Err(WalletError::FileNotFound(
                "Didn't create any file because wallet is unnamed".to_string(),
            ))
        }
    }

    /// Save wallet to specific JSON file
    pub fn save_to<P: AsRef<Path>>(&self, path: P) -> Result<(), WalletError> {
        let json_str = serde_json::to_string_pretty(&self)?;
        fs::write(path, json_str)?;
        Ok(())
    }

    /// Derive the *address* (Poseidon‑hashed) that the circuits use.
    pub fn address(&self) -> Element {
        get_address_for_private_key(self.pk)
    }

    pub fn spend_note(&mut self) -> Result<InputNote, WalletError> {
        if let Some(note) = self.avail.pop() {
            self.balance = 0;
            Ok(InputNote::new(note, self.pk))
        } else {
            Err(WalletError::ZeroBalance(
                self.name.clone().unwrap_or("Noname".to_string()),
            ))
        }
    }

    pub fn receive_note(&mut self, amount: u64) -> Note {
        //let alice_address = hash_merge([self.pk, Element::ZERO]);
        //Note::new_with_psi(alice_address, Element::from(amount), Element::secure_random(rand::thread_rng()))

        let self_address = hash_merge([self.pk, Element::ZERO]);

        //let chain = 5115 as u64; // Citrea chain
        //let token =
        //    H160::from_slice(&hex::decode("52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6").unwrap()); // Token Contract

        let chain = 137u64; // Polygon chain
        let token =
            H160::from_slice(&hex::decode("3c499c542cef5e3811e1192ce70d8cc03d5c3359").unwrap());

        let note = Note {
            kind: Element::new(2),
            contract: generate_note_kind_bridge_evm(chain, token),
            address: self_address,
            psi: Element::secure_random(rand::thread_rng()),
            value: Element::from(amount),
        };

        self.avail.push(note.clone());
        self.balance = amount;
        note
    }
}

// Implement Drop to auto-save on drop if configured
impl Drop for Wallet {
    fn drop(&mut self) {
        // Best effort save - ignore errors on drop
        if self.name.is_some() {
            let _ = self.save();
        }
    }
}
