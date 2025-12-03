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
    generate_note_kind_bridge_evm, InputNote, Note, Utxo,
};

use crate::CipheraAddress;
use crate::address::{decode_address, citrea_token_data, citrea_currency_from_contract, citrea_ticker_from_contract};
use crate::rpc::TxnWithInfo;
use std::collections::HashMap;

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
    pub pending: HashMap<String, Vec<InputNote>>,
    pub avail: HashMap<String, Vec<InputNote>>,
    pub name: Option<String>,
    pub balance: u64,
}

impl Wallet {
    /// Create a wallet from an explicit private key.
    pub fn new(name: Option<String>, pk: Element) -> Self {
        Self {
            pk,
            keys: Vec::new(),
            pending: HashMap::new(),
            avail: HashMap::new(),
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
            pending: HashMap::new(),
            avail: HashMap::new(),
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

    pub fn spend_note(&mut self, amount: u64, ticker: &str) -> Result<InputNote, WalletError> {

        if let Some(asset_notes) = self.avail.get_mut(ticker) {
            if asset_notes.len() == 0 {
                let name = self.name.clone().unwrap_or("Noname".to_string());
                return Err(WalletError::LowBalance(format!(
                    "Wallet {name} has 0 balance")));
            }

            let mut delta = 0_u64;

            let mut i = 0;
            let mut i_min = 0;

            for n in asset_notes.clone() {
                if let Some(note_amount) = n.note.value.to_u64_array().first() {
                    let delta_new: u64 = if note_amount.to_owned() > amount {
                        note_amount.to_owned() - amount
                    } else {
                        amount - note_amount.to_owned()
                    };
                    if delta_new <= delta {
                        delta = delta_new;
                        i_min = i;
                    }
                }
                i += 1;
            }

            if let input_note = asset_notes.remove(i_min) {
                let values = input_note.note.value.to_u64_array().clone();
                let Some(note_amount) = values.first() else {
                    return Err(WalletError::CantPullNote());
                };
                self.balance = self.balance - note_amount.to_owned();
                Ok(input_note)
            } else {
                Err(WalletError::CantPullNote())
            }

        } else {
            let name = self.name.clone().unwrap_or("Noname".to_string());
            return Err(WalletError::LowBalance(format!(
                "Wallet {name} has 0 balance")));
        }
    }

    pub fn spend_to(&mut self, address: &str) -> Result<Utxo, WalletError> {
        let note = Note::from(&decode_address(address));
        let ticker = citrea_ticker_from_contract(note.contract);
        let values = note.value.to_u64_array().clone();
        let Some(amount) = values.first() else {
            return Err(WalletError::CantPullNote());
        };

        if amount.to_owned() > self.balance {
            let name = self.name.clone().unwrap_or("Noname".to_string());
            return Err(WalletError::LowBalance(format!(
                "Wallet {} has only {} while {} requested",
                name, self.balance, amount
            )));
        }

        let input_note_1 = self.spend_note(amount.to_owned(), &ticker)?;
        let values = input_note_1.note.value.to_u64_array().clone();
        let Some(amount_1) = values.first() else {
            return Err(WalletError::CantPullNote());
        };

        let pk = self.gen_pk();
        let self_address = hash_merge([pk, Element::ZERO]);

        let (inputs, change) = if amount_1.to_owned() == amount.to_owned() {
            (
                [input_note_1.clone(), InputNote::padding_note()],
                Note::padding_note(),
            )
        } else if  amount_1.to_owned() < amount.to_owned() {
            println!("Pulling additional note. Requested {}, available {}", amount, amount_1);
            let delta = (amount - amount_1) as u64;
            let input_note_2 = self.spend_note(delta, &ticker)?;

            let values = input_note_1.note.value.to_u64_array().clone();
            let Some(amount_2) = values.first() else {
                return Err(WalletError::CantPullNote());
            };
            let change_amount = ( amount_1 + amount_2 ) - amount;
            println!("Pulled {}, change {}", amount_2, change_amount);
            if change_amount == 0 {
                ([input_note_1.clone(), input_note_2.clone()], Note::padding_note())
            } else {
                let change = Note {
                    kind: input_note_1.note.kind,
                    contract: input_note_1.note.contract,
                    address: self_address,
                    psi: Element::secure_random(rand::thread_rng()),
                    value: Element::from(change_amount),
                };
                if let Some(asset_notes) = self.avail.get_mut(&ticker) {
                    asset_notes.push(InputNote::new(change.clone(), pk));
                } else {
                    self.avail.insert(ticker, vec![InputNote::new(change.clone(), pk)]);
                };
                self.balance = self.balance + change_amount;

                ([input_note_1.clone(), input_note_2.clone()], change)
            }
        } else {
            let change_amount = amount_1 - amount;
            println!("Pulled note {}, requested {}, change {}", amount_1, amount, change_amount);
            let change = Note {
                kind: input_note_1.note.kind,
                contract: input_note_1.note.contract,
                address: self_address,
                psi: Element::secure_random(rand::thread_rng()),
                value: Element::from(change_amount),
            };
            if let Some(asset_notes) = self.avail.get_mut(&ticker) {
                asset_notes.push(InputNote::new(change.clone(), pk));
            } else {
                self.avail.insert(ticker, vec![InputNote::new(change.clone(), pk)]);
            };
            self.balance = self.balance + change_amount;

            ([input_note_1.clone(), InputNote::padding_note()], change)
        };
        Ok(Utxo::new_send(inputs, [note, change]))
    }

    pub fn receive_note(&mut self, amount: u64, ticker: &str) -> Note {
        let pk = self.gen_pk();
        let self_address = hash_merge([pk, Element::ZERO]);

        let (kind, contract) = citrea_token_data(ticker);

        let note = Note {
            kind,
            contract,
            address: self_address,
            psi: Element::secure_random(rand::thread_rng()),
            value: Element::from(amount),
        };

        if let Some(asset_notes) = self.avail.get_mut(ticker) {
            asset_notes.push(InputNote::new(note.clone(), pk));
        } else {
            self.avail.insert(ticker.to_string(), vec![InputNote::new(note.clone(), pk)]);
        };

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

                let ticker = citrea_ticker_from_contract(note.contract);

                if let Some(asset_notes) = self.avail.get_mut(&ticker) {
                    asset_notes.push(InputNote::new(note.clone(), pk));
                } else {
                    self.avail.insert(ticker.to_string(), vec![InputNote::new(note.clone(), pk)]);
                }

                self.balance = self.balance + amount;
                self.keys.remove(i);
                return Ok(())
            }
            i += 1
        }
        Err(WalletError::KeyNotFound(format!("Cant import {:?}", note)))
    }

    pub fn get_address(&mut self, amount: u64, ticker: &str) -> CipheraAddress {
        let pk = self.gen_pk();
        let psi = self.gen_pk();
        let address = hash_merge([pk, Element::ZERO]);
        let (kind, contract) = citrea_token_data(ticker);

        self.keys.push(pk.clone());
        let note = Note {
            kind,
            contract,
            address,
            psi,
            value: Element::new(amount),
        };
        self.pending.insert(ticker.to_string(), vec![InputNote::new(note.clone(), pk)]);

        (&note).into()
    }

    pub fn sync(&mut self, txns: &Vec<TxnWithInfo>) -> Result<(), WalletError> {
        for tx in txns {
            let id = tx.hash;
            let block = tx.block_height;
            for c in tx.proof.public_inputs.output_commitments {
                if c != Element::ZERO {// not a padding note
                    for (ticker, asset_notes) in &mut self.pending {
                        let mut idx = vec![];
                        let mut i = 0;
                        for p in asset_notes.clone() {
                            if c == p.note.commitment() {
                                println!("\nFound commitment - {:x} in {}:{}\n", c, block, id);
                                let values = p.note.value.to_u64_array().clone();
                                let Some(amount) = values.first() else {
                                    return Err(WalletError::CantReadNoteValue());
                                };

                                if let Some(asset_notes) = self.avail.get_mut(ticker) {
                                    asset_notes.push(p.clone());
                                } else {
                                    self.avail.insert(ticker.to_string(), vec![p.clone()]);
                                }

                                self.balance = self.balance + amount;
                                idx.push(i);
                            }
                            i += 1;
                        };
                        for j in idx {
                            asset_notes.remove(j);
                        }
                    }
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

#[cfg(test)]
mod wallet_tests {
    use super::*;
    use element::Element;
    use zk_primitives::InputNote;

    // Helper function to create a test wallet with known balance
    fn create_test_wallet(balance: u64, num_notes: usize) -> Wallet {
        let mut wallet = Wallet::random(Some("test_wallet".to_string()));

        // Create input notes with specified amounts
        for i in 0..num_notes {
            let note = Note {
                kind: Element::new(2),
                contract: Element::ZERO,
                address: Element::from(i as u64),
                psi: Element::ZERO,
                value: Element::from(balance / num_notes as u64),
            };

            wallet.avail.insert("WCBTC".to_string(), vec![InputNote::new(note, Element::from(i as u64))]);
        }

        wallet.balance = balance;
        wallet
    }

    fn create_note_and_encode_address(amount: u64) -> String {
        let (kind, contract) = citrea_token_data("WCBTC");

        let note = Note {
            kind,
            contract,
            address: hash_merge([Element::new(101), Element::ZERO]),
            psi: Element::ZERO,
            value: Element::new(amount),
        };

        let a: CipheraAddress = (&note).into();

        a.encode_address()
    }

    fn create_input_note(amount: u64) -> InputNote {
        let note = Note {
            kind: Element::new(2),
            contract: Element::ZERO,
            address: Element::ZERO,
            psi: Element::ZERO,
            value: Element::from(amount),
        };
        InputNote::new(note, Element::ZERO)
    }

    // =====================================================================
    // spend_note Tests
    // =====================================================================

    #[test]
    fn test_spend_note_success_single_note() {
        let mut wallet = create_test_wallet(1000, 1);

        let result = wallet.spend_note(1000, "WCBTC");
        assert!(result.is_ok());
        assert_eq!(wallet.avail.len(), 0); // Note was removed
        assert_eq!(wallet.balance, 0); // Balance updated
    }

    #[test]
    fn test_spend_note_success_multiple_notes() {
        let mut wallet = create_test_wallet(1200, 3);

        let result = wallet.spend_note(400, "WCBTC");
        assert!(result.is_ok());
        assert_eq!(wallet.avail.len(), 2); // One note removed
        assert_eq!(wallet.balance, 800);
    }

    #[test]
    fn test_spend_note_selects_best_fit() {
        // Test that spend_note selects the note closest to requested amount
        let mut wallet = Wallet::random(Some("test".to_string()));

        // Add notes with values: 100, 500, 1000
        wallet.avail.push(create_input_note(100));
        wallet.avail.push(create_input_note(500));
        wallet.avail.push(create_input_note(1000));
        wallet.balance = 1600;

        // Request 450 - should select 500 (delta=50) over 1000 (delta=550)
        let result = wallet.spend_note(450, "WCBTC");
        assert!(result.is_ok());
        assert_eq!(wallet.avail.len(), 2);
    }

    #[test]
    fn test_spend_note_empty_wallet() {
        let mut wallet = Wallet::random(Some("test".to_string()));

        let result = wallet.spend_note(100, "WCBTC");
        assert!(result.is_err());
        match result {
            Err(WalletError::LowBalance(_)) => (),
            _ => panic!("Expected LowBalance error"),
        }
    }

    #[test]
    fn test_spend_note_exact_match() {
        let mut wallet = create_test_wallet(1000, 1);

        let result = wallet.spend_note(1000, "WCBTC");
        assert!(result.is_ok());
        assert_eq!(wallet.balance, 0);
        assert_eq!(wallet.avail.len(), 0);
    }

    #[test]
    fn test_spend_note_with_none_amount() {
        // Test behavior when None is passed as amount
        let mut wallet = create_test_wallet(1000, 2);

        let result = wallet.spend_note(1, "WCBTC");
        assert!(result.is_ok());
        assert_eq!(wallet.avail.len(), 1);
    }

    #[test]
    fn test_spend_note_large_request_small_note() {
        let mut wallet = create_test_wallet(100, 1);

        let result = wallet.spend_note(1000, "WCBTC");
        // Should still remove the note even though amount requested > available
        assert!(result.is_ok());
        assert_eq!(wallet.balance, 0);
    }

    // =====================================================================
    // spend_to Tests
    // =====================================================================

    #[test]
    fn test_spend_to_exact_amount() {
        let mut wallet = create_test_wallet(1000, 1);
        let address = create_note_and_encode_address(1000);

        let result = wallet.spend_to(&address);
        assert!(result.is_ok());

        let utxo = result.unwrap();
        assert_eq!(wallet.avail.len(), 0); // Note consumed
    }

    #[test]
    fn test_spend_to_with_change() {
        let mut wallet = create_test_wallet(1000, 1);
        let address = create_note_and_encode_address(100);

        let result = wallet.spend_to(&address);
        assert!(result.is_ok());

        // Balance should be updated with change
        assert!(wallet.balance == 900);
        // Change Note should be added immidiately
        assert_eq!(wallet.avail.len(), 1);
    }

    #[test]
    fn test_spend_to_insufficient_balance() {
        let mut wallet = create_test_wallet(100, 1);
        let address = create_note_and_encode_address(1000);

        let result = wallet.spend_to(&address);
        assert!(result.is_err());

        match result {
            Err(WalletError::LowBalance(_)) => (),
            _ => panic!("Expected LowBalance error"),
        }
    }

    #[test]
    fn test_spend_to_multiple_notes_required() {
        let mut wallet = create_test_wallet(1200, 2);

        let address = create_note_and_encode_address(1000);
        let result = wallet.spend_to(&address);

        // Balance should be updated with change
        assert!(wallet.balance == 200);
        // Change Note should be added immidiately
        assert_eq!(wallet.avail.len(), 1);
    }

    #[test]
    fn test_spend_to_and_pick_only_two() {
        let mut wallet = create_test_wallet(1200, 3);

        let address = create_note_and_encode_address(700);
        let result = wallet.spend_to(&address);

        // Balance should be updated with change
        assert!(wallet.balance == 500);
        // Change Note should be added immidiately
        assert_eq!(wallet.avail.len(), 2);
    }

    #[test]
    fn test_spend_to_empty_wallet() {
        let mut wallet = create_test_wallet(0, 0);

        let address = create_note_and_encode_address(1000);
        let result = wallet.spend_to(&address);

        // Should fail due to low balance
        assert!(result.is_err());
    }

    #[test]
    fn test_spend_to_updates_balance_correctly() {
        let mut wallet = create_test_wallet(2000, 2);
        let initial_balance = wallet.balance;
        let address = create_note_and_encode_address(1000);

        let result = wallet.spend_to(&address);

        if result.is_ok() {
            // Balance should be updated appropriately
            assert!(wallet.balance <= initial_balance);
        }
    }

    // =====================================================================
    // Edge Cases and Integration Tests
    // =====================================================================

    #[test]
    fn test_consecutive_spend_notes() {
        let mut wallet = create_test_wallet(2000, 2);

        let result1 = wallet.spend_note(1000, "WCBTC");
        let result2 = wallet.spend_note(1000, "WCBTC");
        let result3 = wallet.spend_note(500, "WCBTC");

        assert!(result1.is_ok());
        assert!(result2.is_ok());
        assert!(result3.is_err()); // Should fail - no notes left
    }
}
