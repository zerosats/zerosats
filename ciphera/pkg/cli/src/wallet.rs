use element::Element;
use hash::hash_merge;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::fs;
use std::num::ParseIntError;
use std::path::Path;
use zk_primitives::{InputNote, Note, Utxo};

use crate::CipheraAddress;
use crate::address::{citrea_ticker_from_contract, citrea_token_data};
use crate::rpc::TxnWithInfo;
use std::collections::HashMap;
use tracing::{debug, error, info};

// Error types for wallet operations
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Wallet file not found: {0}")]
    FileNotFound(String),

    #[error("Wallet already exists: {0}")]
    WalletExists(String),

    #[error("No data in wallet file: {0}")]
    KeyNotFound(String),

    #[error("Unable to read secret: {0}")]
    CouldNotReadKey(#[from] ParseIntError),

    #[error("No coins left in wallet {0}")]
    LowBalance(String),

    #[error("Unable to pull note")]
    CantPullNote,

    #[error("Unable to convert note value")]
    CantReadNoteValue,

    #[error("Unable to find a secret key")]
    NoKey,
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
    pub chain_id: u64,
}

impl Wallet {
    /// Create a wallet from an explicit private key.
    pub fn new(chain_id: u64, name: Option<String>, pk: Element) -> Self {
        Self {
            pk,
            keys: Vec::new(),
            pending: HashMap::new(),
            avail: HashMap::new(),
            name,
            balance: 0,
            chain_id,
        }
    }

    /// Create a wallet with a random 256‑bit private key.
    pub fn random(chain_id: u64, name: Option<String>) -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self {
            pk: Element::from_be_bytes(bytes),
            keys: Vec::new(),
            pending: HashMap::new(),
            avail: HashMap::new(),
            name,
            balance: 0,
            chain_id,
        }
    }

    pub fn gen_pk(&self) -> Element {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Element::from_be_bytes(bytes)
    }

    /// Creates wallet with random secret and saves JSON file
    pub fn create(chain_id: u64, name: &str) -> Result<Self, WalletError> {
        let file = format!("{name}.json");
        let wallet_file = Path::new(&file);

        if wallet_file.is_file() {
            Err(WalletError::WalletExists(file))
        } else {
            let wallet = Self::random(chain_id, Some(name.to_string()));
            wallet.save()?;
            Ok(wallet)
        }
    }

    /// Load wallet from JSON file
    pub fn load(name: &str) -> Result<Self, WalletError> {
        let file = format!("{name}.json");
        let wallet_file = Path::new(&file);

        if wallet_file.is_file() {
            let json_str = fs::read_to_string(wallet_file)?;
            Ok(serde_json::from_str(&json_str)?)
        } else {
            Err(WalletError::FileNotFound(file))
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

    fn push_to_avail(&mut self, ticker: &str, note: InputNote) -> Result<u64, WalletError> {
        self.avail
            .entry(ticker.to_string())
            .or_default()
            .push(note.clone());
        let note_amount = note
            .note
            .value
            .to_u64_array()
            .first()
            .copied()
            .ok_or(WalletError::CantReadNoteValue)?;
        self.balance += note_amount;
        Ok(self.balance)
    }

    fn pull_from_avail(&mut self, ticker: &str, note: InputNote) -> Result<u64, WalletError> {
        let opt_balance = self.avail.get_mut(ticker).and_then(|notes| {
            let pos = notes.iter().position(|n| n.secret_key == note.secret_key)?;
            let removed_note = notes.remove(pos);
            let note_amount = removed_note
                .note
                .value
                .to_u64_array()
                .first()
                .copied()
                .or(None)?;
            self.balance -= note_amount;
            Some(self.balance)
        });
        match opt_balance {
            Some(b) => Ok(b),
            None => Err(WalletError::CantPullNote),
        }
    }

    fn make_change_note(&self, origin_note: &Note, change_amount: u64) -> InputNote {
        let pk = self.gen_pk();
        let self_address = hash_merge([pk, Element::ZERO]);
        InputNote::new(
            Note {
                kind: origin_note.kind,
                contract: origin_note.contract,
                address: self_address,
                psi: hash_merge([pk, pk]),
                //psi: Element::secure_random(rand::thread_rng()),
                value: Element::from(change_amount),
            },
            pk,
        )
    }

    fn select_input_notes(
        &mut self,
        ticker: &str,
        amount: u64,
    ) -> Result<([InputNote; 2], Note), WalletError> {
        let input_note_1 = self.find_note(amount, ticker)?;
        let amount_1 = self.get_note_amount(&input_note_1.note)?;

        if amount_1 == amount {
            return Ok((
                [input_note_1.clone(), InputNote::padding_note()],
                Note::padding_note(),
            ));
        } else if amount_1 < amount {
            info!("Requested {amount}, found {amount_1}. Pulling additional note");
            let delta = amount - amount_1;

            let input_note_2 = self.find_note(delta, &ticker)?;
            let amount_2 = self.get_note_amount(&input_note_2.note)?;

            if delta == amount_2 {
                // 2 inputs, no change
                let b = self.pull_from_avail(&ticker, input_note_1.clone())?;
                debug!(balance = b, "pulled first input note");
                let b = self.pull_from_avail(&ticker, input_note_2.clone())?;
                debug!(balance = b, "pulled second input note");

                return Ok((
                    [input_note_1.clone(), input_note_2.clone()],
                    Note::padding_note(),
                ));
            } else if delta < amount_2 {
                // 2 inputs with our change
                let change_amount = (amount_1 + amount_2) - amount;
                let b = self.pull_from_avail(&ticker, input_note_1.clone())?;
                debug!(balance = b, "pulled first input note");
                let b = self.pull_from_avail(&ticker, input_note_2.clone())?;
                debug!(balance = b, "pulled second input note");

                let change_note = self.make_change_note(&input_note_1.note, change_amount);
                let b = self.push_to_avail(&ticker, change_note.clone())?;
                debug!(balance = b, "added change");
                return Ok((
                    [input_note_1.clone(), input_note_2.clone()],
                    change_note.note,
                ));
            } else {
                // 2 inputs, not enough money
                return Err(WalletError::LowBalance(
                    "Too many small notes, consolidate first".to_string(),
                ));
            }
        } else {
            // 1 input, out change
            let change_amount = amount_1 - amount;
            info!("requested {amount}, change {change_amount}");
            let b = self.pull_from_avail(&ticker, input_note_1.clone())?;
            debug!(balance = b, "pulled first input note");
            let change_note = self.make_change_note(&input_note_1.note, change_amount);
            let b = self.push_to_avail(&ticker, change_note.clone())?;
            debug!(balance = b, "updated wallet balance");
            return Ok((
                [input_note_1.clone(), InputNote::padding_note()],
                change_note.note,
            ));
        }
    }

    pub fn find_note(&mut self, amount: u64, ticker: &str) -> Result<InputNote, WalletError> {
        let asset_notes = self
            .avail
            .get_mut(ticker)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                WalletError::LowBalance(format!(
                    "Wallet {} has 0 balance",
                    self.name.as_deref().unwrap_or("Noname")
                ))
            })?;

        let best_idx = asset_notes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.note.value.to_u64_array().first().copied().map(|v| (i, v)))
            .min_by_key(|(_, v)| v.abs_diff(amount))
            .map(|(i, _)| i)
            .ok_or(WalletError::LowBalance("No notes found".to_string()))?;

        Ok(asset_notes[best_idx].clone())
    }

    pub fn get_note_amount(&self, note: &Note) -> Result<u64, WalletError> {
        let values = note.value.to_u64_array();
        let Some(amount) = values.first() else {
            return Err(WalletError::CantPullNote);
        };
        Ok(amount.to_owned())
    }

    pub fn spend_to(&mut self, note: &Note) -> Result<Utxo, WalletError> {
        let ticker = citrea_ticker_from_contract(note.contract);
        let amount = self.get_note_amount(&note)?;

        if amount > self.balance {
            let name = self.name.clone().unwrap_or("Noname".to_string());
            return Err(WalletError::LowBalance(format!(
                "Wallet {} has only {} while {} requested",
                name, self.balance, amount
            )));
        }

        let (inputs, change) = self.select_input_notes(&ticker, amount)?;

        Ok(Utxo::new_send(inputs, [note.to_owned(), change]))
    }

    pub fn receive(&mut self, input_note: &InputNote) -> Result<Utxo, WalletError> {
        let ticker = citrea_ticker_from_contract(input_note.note.contract);
        let amount = self.get_note_amount(&input_note.note)?;
        let received_note: InputNote = self.receive_note(amount, &ticker);

        let b = self.push_to_avail(&ticker, received_note.clone())?;
        debug!(balance = b, "updated wallet balance");

        Ok(Utxo::new_send(
            [input_note.clone(), InputNote::padding_note()],
            [received_note.note, Note::padding_note()],
        ))
    }

    pub fn mint(&mut self, amount: u64, ticker: &str) -> Result<Utxo, WalletError> {
        let received_note: InputNote = self.receive_note(amount, ticker);

        let b = self.push_to_avail(&ticker, received_note.clone())?;
        debug!(balance = b, "updated wallet balance");

        Ok(Utxo::new_mint([
            received_note.note.clone(),
            Note::padding_note(),
        ]))
    }

    pub fn burn(
        &mut self,
        burner_note: &InputNote,
        evm_address: &Element,
    ) -> Result<Utxo, WalletError> {
        let ticker = citrea_ticker_from_contract(burner_note.note.contract);

        let b = self.pull_from_avail(&ticker, burner_note.to_owned())?;
        debug!(balance = b, "pulled first input note");

        Ok(Utxo::new_burn(
            [burner_note.to_owned(), InputNote::padding_note()],
            evm_address.to_owned(),
        ))
    }

    pub fn receive_note(&mut self, amount: u64, ticker: &str) -> InputNote {
        let pk = self.gen_pk();
        let self_address = hash_merge([pk, Element::ZERO]);
        self.keys.push(pk);

        let (kind, contract) = citrea_token_data(ticker);

        let note = Note {
            kind,
            contract,
            address: self_address,
            psi: hash_merge([pk, pk]),
            //psi: Element::secure_random(rand::thread_rng()),
            value: Element::from(amount),
        };

        InputNote::new(note.clone(), pk)
    }

    pub fn import_note(&mut self, note: &Note) -> Result<(), WalletError> {
        let mut i = 0;
        for pk in self.keys.clone() {
            let self_address = hash_merge([pk, Element::ZERO]);
            if note.address == self_address {
                let amount = self.get_note_amount(&note)?;

                let ticker = citrea_ticker_from_contract(note.contract);

                debug!(ticker = ticker, amount = amount, "importing note");

                let b = self.push_to_avail(&ticker, InputNote::new(note.clone(), pk))?;

                debug!(balance = b, "updated wallet balance");

                self.keys.remove(i);
                return Ok(());
            }
            i += 1
        }
        Err(WalletError::KeyNotFound(format!("Cant import {note:?}")))
    }

    pub fn get_address(&mut self, amount: u64, ticker: &str) -> CipheraAddress {
        let pk = self.gen_pk();
        let psi = self.gen_pk();
        let address = hash_merge([pk, Element::ZERO]);
        let (kind, contract) = citrea_token_data(ticker);

        self.keys.push(pk);
        let note = Note {
            kind,
            contract,
            address,
            psi,
            value: Element::new(amount),
        };
        self.pending
            .insert(ticker.to_string(), vec![InputNote::new(note.clone(), pk)]);

        (&note).into()
    }

    pub fn sync(&mut self, txns: &Vec<TxnWithInfo>) -> Result<(), WalletError> {
        for tx in txns {
            let id = tx.hash;
            let block = tx.block_height;
            for c in tx.proof.public_inputs.output_commitments {
                if c != Element::ZERO {
                    // not a padding note
                    let mut new_notes = vec![];

                    for (_, asset_notes) in &mut self.pending {
                        let mut idx = vec![];
                        for (i, p) in asset_notes.iter().enumerate() {
                            if c == p.note.commitment() {
                                info!("found commitment - {c:x} in {block}:{id}");
                                idx.push(i);
                                new_notes.push(p.clone());
                            }
                        }
                        idx.sort_unstable_by(|a, b| b.cmp(a));
                        for j in idx {
                            asset_notes.remove(j);
                        }
                    }

                    for n in new_notes {
                        let ticker = citrea_ticker_from_contract(n.note.contract);
                        let b = self.push_to_avail(&ticker, n.clone())?;
                        debug!(balance = b, "added note");
                    }
                }
            }
        }
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
    use crate::address::decode_address;
    use element::Element;
    use zk_primitives::InputNote;

    // Helper function to create a test wallet with known balance
    fn create_test_wallet(balance: u64, num_notes: usize) -> Wallet {
        let mut wallet = Wallet::random(5115, Some("test_wallet".to_string()));

        // Create input notes with specified amounts
        for i in 0..num_notes {
            let note = Note {
                kind: Element::new(2),
                contract: Element::ZERO,
                address: Element::from(i as u64),
                psi: Element::ZERO,
                value: Element::from(balance / num_notes as u64),
            };

            if let Some(asset_notes) = wallet.avail.get_mut("WCBTC") {
                asset_notes.push(InputNote::new(note, Element::from(i as u64)));
            // Note was removed
            } else {
                wallet.avail.insert(
                    "WCBTC".to_string(),
                    vec![InputNote::new(note, Element::from(i as u64))],
                );
            };
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
    // find_note Tests
    // =====================================================================

    #[test]
    fn test_find_note_success_single_note() {
        let mut wallet = create_test_wallet(1000, 1);

        let result = wallet.find_note(1000, "WCBTC");
        assert!(result.is_ok());
        let _ = wallet.pull_from_avail("WCBTC", result.unwrap()).unwrap();
        assert_eq!(wallet.balance, 0); // Balance updated
        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 0); // Note was removed
        } else {
            panic!();
        };
    }

    #[test]
    fn test_find_note_success_multiple_notes() {
        let mut wallet = create_test_wallet(1200, 3);

        let result = wallet.find_note(400, "WCBTC");
        assert!(result.is_ok());
        let _ = wallet.pull_from_avail("WCBTC", result.unwrap()).unwrap();
        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 2); // One note removed
        } else {
            panic!();
        };
        assert_eq!(wallet.balance, 800);
    }

    #[test]
    fn test_find_note_selects_best_fit() {
        // Test that find_note selects the note closest to requested amount
        let mut wallet = Wallet::random(5115, Some("test".to_string()));

        // Add notes with values: 100, 500, 1000
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![
                create_input_note(100),
                create_input_note(500),
                create_input_note(1000),
            ],
        );
        wallet.balance = 1600;

        // Request 450 - should select 500 (delta=50) over 1000 (delta=550)
        let result = wallet.find_note(450, "WCBTC");
        assert!(result.is_ok());
        let _ = wallet.pull_from_avail("WCBTC", result.unwrap()).unwrap();

        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 2);
        } else {
            panic!();
        };
    }

    #[test]
    fn test_find_note_empty_wallet() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));

        let result = wallet.find_note(100, "WCBTC");
        assert!(result.is_err());
        match result {
            Err(WalletError::LowBalance(_)) => (),
            _ => panic!("Expected LowBalance error"),
        }
    }

    #[test]
    fn test_find_note_exact_match() {
        let mut wallet = create_test_wallet(1000, 1);

        let result = wallet.find_note(1000, "WCBTC");
        assert!(result.is_ok());
        let _ = wallet.pull_from_avail("WCBTC", result.unwrap()).unwrap();

        assert_eq!(wallet.balance, 0);

        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 0);
        } else {
            panic!();
        };
    }

    #[test]
    fn test_find_note_with_none_amount() {
        // Test behavior when None is passed as amount
        let mut wallet = create_test_wallet(1000, 2);

        let result = wallet.find_note(1, "WCBTC");
        assert!(result.is_ok());

        let _ = wallet.pull_from_avail("WCBTC", result.unwrap()).unwrap();

        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 1); // One note removed
        } else {
            panic!();
        };
    }

    #[test]
    fn test_find_note_large_request_small_note() {
        let mut wallet = create_test_wallet(100, 1);

        let result = wallet.find_note(1000, "WCBTC");
        assert!(result.is_ok());

        let _ = wallet.pull_from_avail("WCBTC", result.unwrap()).unwrap();
        assert_eq!(wallet.balance, 0);
    }

    // =====================================================================
    // spend_to Tests
    // =====================================================================

    #[test]
    fn test_spend_to_exact_amount() {
        let mut wallet = create_test_wallet(1000, 1);
        let address = create_note_and_encode_address(1000);
        let note = Note::from(&decode_address(address));

        let result = wallet.spend_to(&note);
        assert!(result.is_ok());

        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 0); // Note consumed
        } else {
            panic!();
        };
    }

    #[test]
    fn test_spend_to_with_change() {
        let mut wallet = create_test_wallet(1000, 1);
        let address = create_note_and_encode_address(100);
        let note = Note::from(&decode_address(address));

        let result = wallet.spend_to(&note);
        assert!(result.is_ok());

        // Balance should be updated with change
        assert!(wallet.balance == 900);
        // Change Note should be added immidiately
        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 1);
        } else {
            panic!();
        };
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
        assert!(result.is_ok());

        // Balance should be updated with change
        assert!(wallet.balance == 200);
        // Change Note should be added immidiately
        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 1);
        } else {
            panic!();
        };
    }

    #[test]
    fn test_spend_to_and_pick_only_two() {
        let mut wallet = create_test_wallet(1200, 3);

        let address = create_note_and_encode_address(700);
        let result = wallet.spend_to(&address);
        assert!(result.is_ok());

        // Balance should be updated with change
        assert!(wallet.balance == 500);
        // Change Note should be added immidiately
        if let Some(asset_notes) = wallet.avail.get("WCBTC") {
            assert_eq!(asset_notes.len(), 2); // One note removed
        } else {
            panic!();
        };
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
        assert!(result.is_ok());
        assert!(wallet.balance <= initial_balance);
    }

    // =====================================================================
    // Edge Cases and Integration Tests
    // =====================================================================

    #[test]
    fn test_consecutive_find_notes() {
        let mut wallet = create_test_wallet(2000, 2);

        let result1 = wallet.find_note(1000, "WCBTC");
        assert!(result1.is_ok());
        let _ = wallet.pull_from_avail("WCBTC", result1.unwrap()).unwrap();

        let result2 = wallet.find_note(1000, "WCBTC");
        assert!(result2.is_ok());
        let _ = wallet.pull_from_avail("WCBTC", result2.unwrap()).unwrap();

        let result3 = wallet.find_note(500, "WCBTC");
        assert!(result3.is_err()); // Should fail - no notes left
    }

    // =====================================================================
    // Bug-fix regression tests (see wallet_note_selection_analysis.md)
    // =====================================================================

    fn note_value(n: &InputNote) -> u64 {
        *n.note.value.to_u64_array().first().unwrap()
    }

    // Notes [1000, 500, 100], request 150.
    // |100-150|=50 < |500-150|=350 < |1000-150|=850 → expect 100.
    #[test]
    fn test_best_fit_selects_last_not_first() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![
                create_input_note(1000),
                create_input_note(500),
                create_input_note(100),
            ],
        );
        wallet.balance = 1600;

        let result = wallet.find_note(150, "WCBTC").unwrap();
        assert_eq!(
            note_value(&result),
            100,
            "expected closest note (100) but got wrong note"
        );
    }

    // Notes [1000, 450, 100], request 400.
    // |450-400|=50 < |100-400|=300 < |1000-400|=600 → expect 450 (index 1).
    // Buggy code returns 1000 (index 0).
    #[test]
    fn test_best_fit_selects_middle_not_first() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![
                create_input_note(1000),
                create_input_note(450),
                create_input_note(100),
            ],
        );
        wallet.balance = 1550;

        let result = wallet.find_note(400, "WCBTC").unwrap();
        assert_eq!(
            note_value(&result),
            450,
            "expected closest note (450) but got wrong note"
        );
    }

    // Notes [800, 200], request 250.
    // |200-250|=-50 < |800-250|=550 → expect 200.
    #[test]
    fn test_best_fit_two_notes_picks_second() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![create_input_note(800), create_input_note(200)],
        );
        wallet.balance = 1000;

        let result = wallet.find_note(250, "WCBTC").unwrap();
        assert_eq!(
            note_value(&result),
            200,
            "expected closest note (200) but got wrong note"
        );
    }

    // Notes [999, 500, 500], request 500.
    // Exact match at index 1 (delta=0) beats index 0 (delta=499).
    // Buggy code returns 999 (index 0).
    #[test]
    fn test_best_fit_exact_match_not_at_index_0() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![
                create_input_note(999),
                create_input_note(500),
                create_input_note(500),
            ],
        );
        wallet.balance = 1999;

        let result = wallet.find_note(500, "WCBTC").unwrap();
        assert_eq!(
            note_value(&result),
            500,
            "expected exact-match note (500), not 999"
        );
    }

    // Regression: existing best-fit test strengthened to assert the returned value.
    #[test]
    fn test_find_note_selects_best_fit_value() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![
                create_input_note(100),
                create_input_note(500),
                create_input_note(1000),
            ],
        );
        wallet.balance = 1600;

        // |500-450|=50 beats |100-450|=350 and |1000-450|=550
        let note = wallet.find_note(450, "WCBTC").unwrap();
        assert_eq!(
            note_value(&note),
            500,
            "find_note must select the closest note, not index 0"
        );

        let _ = wallet.pull_from_avail("WCBTC", note).unwrap();
        assert_eq!(wallet.avail["WCBTC"].len(), 2);
    }

    // Wallet: notes [400, 300] (total 700), spend_to 600.
    // Best fit for 600 → 400 (delta=200), then find_note(200) → 300.
    // Correct change: 400+300-600 = 100  →  wallet.balance = 100.
    // Buggy change:   400+400-600 = 200  →  wallet.balance = 200.
    #[test]
    fn test_two_note_change_uses_second_note_value() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![create_input_note(400), create_input_note(300)],
        );
        wallet.balance = 700;

        let address = create_note_and_encode_address(600);
        wallet.spend_to(&address).unwrap();
        assert_eq!(
            wallet.balance, 100,
            "change should be 100 (400+300-600), not 200 (400+400-600)"
        );
    }

    // Wallet: notes [500, 250] (total 750), spend_to 700.
    // Correct change: 500+250-700 = 50   →  wallet.balance = 50.
    // Buggy change:   500+500-700 = 300  →  wallet.balance = 300.
    #[test]
    fn test_two_note_change_large_value_gap() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));

        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![create_input_note(500), create_input_note(250)],
        );
        wallet.balance = 750;

        let address = create_note_and_encode_address(700);
        wallet.spend_to(&address).unwrap();
        assert_eq!(
            wallet.balance, 50,
            "change should be 50 (500+250-700), not 300 (500+500-700)"
        );
    }

    // Wallet: notes [400, 200] (total 600), spend_to 600.
    // change_amount = 0 → padding note, wallet.balance = 0.
    // Buggy code reads 400 for both notes → (400+400-600)=200, or may underflow.
    #[test]
    fn test_two_note_exact_sum_no_change() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            "WCBTC".to_string(),
            vec![create_input_note(400), create_input_note(200)],
        );
        wallet.balance = 600;

        let address = create_note_and_encode_address(600);
        wallet.spend_to(&address).unwrap();
        assert_eq!(
            wallet.balance, 0,
            "exact two-note spend should leave zero balance"
        );
    }
}
