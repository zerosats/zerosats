use element::Element;
use hash::hash_merge;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs;
use std::num::ParseIntError;
use std::path::Path;
use std::str::FromStr;
use web3::types::H160;
use zk_primitives::{
    generate_note_kind_bridge_evm, get_address_for_private_key, InputNote, Note, Utxo,
};

use crate::CipheraAddress;
use crate::address::{decode_address, citrea_wcbtc_note_kind};
use crate::rpc::TxnWithInfo;

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
    LowBalance(String),

    #[error("Unable to pull note")]
    CantPullNote(),

    #[error("Unable to convert note value")]
    CantReadNoteValue(),

    #[error("Unable to find a secret key")]
    NoKey(),
}

// =====================================================================
// Wallet & helpers
// =====================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    /// *Private* key in the zk‑Primitive sense – **NOT** an ECDSA key!
    pub pk: Element,
    pub keys: Vec<Element>,
    pub pending: Vec<InputNote>,
    pub avail: Vec<InputNote>,
    pub name: Option<String>,
    pub balance: u64,
}

impl Wallet {
    /// Create a wallet from an explicit private key.
    pub fn new(name: Option<String>, pk: Element) -> Self {
        Self {
            pk,
            keys: Vec::new(),
            pending: Vec::new(),
            avail: Vec::new(),
            name,
            balance: 0,
        }
    }

    /// Create a wallet with a random 256‑bit private key.
    pub fn random(name: Option<String>) -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self {
            pk: Element::from_be_bytes(bytes),
            keys: Vec::new(),
            pending: Vec::new(),
            avail: Vec::new(),
            name,
            balance: 0,
        }
    }

    pub fn gen_pk(&self) -> Element {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Element::from_be_bytes(bytes)
    }

    /// Load wallet from JSON file
    pub fn init(name: &str) -> Result<Self, WalletError> {
        let file = format!("{name}.json");
        let wallet_file = Path::new(&file);

        if wallet_file.is_file() {
            let json_str = fs::read_to_string(wallet_file)?;
            Ok(serde_json::from_str(&json_str)?)
        } else {
            let wallet = Self::random(Some(name.to_string()));
            wallet.save()?;
            Ok(wallet)
        }
    }

    /// Save wallet to JSON file (uses configured path or provided path)
    pub fn save(&self) -> Result<(), WalletError> {
        if let Some(name) = &self.name {
            let file = format!("{name}.json");
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

    pub fn spend_note(&mut self, delta: Option<u64>) -> Result<InputNote, WalletError> {
        if let Some(note) = self.avail.pop() {
            if let Some(amount) = note.note.value.to_u64_array().first() {
                self.balance = self.balance - amount;
                return Ok(note);
            }
            Err(WalletError::CantPullNote())
        } else {
            let name = self.name.clone().unwrap_or("Noname".to_string());
            Err(WalletError::LowBalance(format!(
                "Wallet {} has only {}",
                name, self.balance
            )))
        }
    }

    pub fn spend_to(&mut self, amount: u64, address: &str) -> Result<Utxo, WalletError> {
        let input_note_1 = self.spend_note(None)?;

        let values = input_note_1.note.value.to_u64_array().clone();
        let Some(amount_1) = values.first() else {
            return Err(WalletError::CantPullNote());
        };

        let delta = amount_1 - amount;

        let pk = self.gen_pk();
        let self_address = hash_merge([pk, Element::ZERO]);

        let (inputs, change) = if delta < 0 {
            println!("Pulling additional note. Requested {}, available {}", amount, amount_1);
            let input_note_2 = self.spend_note(Some(amount - amount_1))?;

            let values = input_note_1.note.value.to_u64_array().clone();
            let Some(amount_2) = values.first() else {
                return Err(WalletError::CantPullNote());
            };
            let change_amount = delta + amount_2;
            println!("Pulled {}, change {}", amount_2, change_amount);
            let change = Note {
                kind: input_note_1.note.kind,
                contract: input_note_1.note.contract,
                address: self_address,
                psi: Element::secure_random(rand::thread_rng()),
                value: Element::from(change_amount),
            };

            self.avail.push(InputNote::new(change.clone(), pk));
            self.balance = self.balance + change_amount;

            ([input_note_1.clone(), input_note_2.clone()], change)
        } else if delta == 0 {
            (
                [input_note_1.clone(), InputNote::padding_note()],
                Note::padding_note(),
            )
        } else {
            println!("Pulled note {}, requested {}, change {}", amount_1, amount, delta);
            let change = Note {
                kind: input_note_1.note.kind,
                contract: input_note_1.note.contract,
                address: self_address,
                psi: Element::secure_random(rand::thread_rng()),
                value: Element::from(delta),
            };

            self.avail.push(InputNote::new(change.clone(), pk));
            self.balance = self.balance + delta;

            ([input_note_1.clone(), InputNote::padding_note()], change)
        };

        let note = Note::from(&decode_address(address));

        /*
        let note = Note {
            kind: input_note_1.note.kind,
            contract: input_note_1.note.contract,
            address: Element::from_str(address)?,
            psi: Element::new(0),
            value: Element::new(amount),
        };
        */

        Ok(Utxo::new_send(inputs, [note, change]))
    }

    pub fn receive_note(&mut self, amount: u64, chain: u64, token: &str) -> Note {
        let pk = self.gen_pk();
        let self_address = hash_merge([pk, Element::ZERO]);
        let token = H160::from_str(token).unwrap(); // TODO: remove unwrap

        let note = Note {
            kind: Element::new(2),
            contract: generate_note_kind_bridge_evm(chain, token),
            address: self_address,
            psi: Element::secure_random(rand::thread_rng()),
            value: Element::from(amount),
        };

        self.avail.push(InputNote::new(note.clone(), pk));
        self.balance = self.balance + amount;
        note
    }

    pub fn import_note(&mut self, note: &Note) -> Result<(), WalletError> {
        let mut i = 0;
        for pk in self.keys.clone() {
            let self_address = hash_merge([pk, Element::ZERO]);
            if note.address == self_address {
                let values = note.value.to_u64_array().clone();
                let Some(amount) = values.first() else {
                    return Err(WalletError::CantReadNoteValue());
                };

                self.avail.push(InputNote::new(note.clone(), pk));
                self.balance = self.balance + amount;
                self.keys.remove(i);
                return Ok(())
            }
            i += 1
        }
        Err(WalletError::KeyNotFound(format!("Cant import {:?}", note)))
    }

    pub fn get_address(&mut self, amount: u64) -> CipheraAddress {
        let pk = self.gen_pk();
        let psi = self.gen_pk();
        let address = hash_merge([pk, Element::ZERO]);
        let kind = Element::new(2);
        let contract = citrea_wcbtc_note_kind();

        self.keys.push(pk.clone());
        let note = Note {
            kind,
            contract,
            address,
            psi,
            value: Element::new(amount),
        };
        self.pending.push(InputNote::new(note.clone(), pk));

        (&note).into()
    }

    pub fn sync(&mut self, txns: &Vec<TxnWithInfo>) -> Result<(), WalletError> {
        for tx in txns {
            let id = tx.hash;
            let block = tx.block_height;
            for c in tx.proof.public_inputs.output_commitments {
                if c != Element::ZERO {// not a padding note
                    let mut i = 0;
                    for p in &self.pending {
                        if c == p.note.commitment() {
                            println!("\nFound commitment - {:x} in {}:{}\n", c, block, id);
                            let values = p.note.value.to_u64_array().clone();
                            let Some(amount) = values.first() else {
                                return Err(WalletError::CantReadNoteValue());
                            };
                            self.avail.push(p.clone());
                            self.balance = self.balance + amount;
                            self.pending.remove(i);
                            return Ok(())
                        }
                        i += 1;
                    };
                }
            }
        };
        Ok(())
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
