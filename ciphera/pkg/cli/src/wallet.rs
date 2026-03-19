use element::Element;
use hash::hash_merge;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::fs;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
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

    #[error("Wallet has no storage path configured")]
    MissingStoragePath,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip)]
    storage_path: Option<PathBuf>,
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
            chain_id: Some(chain_id),
            storage_path: None,
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
            chain_id: Some(chain_id),
            storage_path: None,
        }
    }

    pub fn gen_pk(&self) -> Element {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Element::from_be_bytes(bytes)
    }

    fn with_storage_path(mut self, storage_path: PathBuf) -> Self {
        self.storage_path = Some(storage_path);
        self
    }

    pub fn wallet_path_in<P: AsRef<Path>>(base_dir: P, name: &str) -> PathBuf {
        base_dir.as_ref().join(format!("{name}.json"))
    }

    /// Creates wallet with random secret and saves JSON file
    pub fn create_in<P: AsRef<Path>>(
        base_dir: P,
        chain_id: u64,
        name: &str,
    ) -> Result<Self, WalletError> {
        let wallet_path = Self::wallet_path_in(base_dir, name);

        if wallet_path.is_file() {
            Err(WalletError::WalletExists(wallet_path.display().to_string()))
        } else {
            let wallet =
                Self::random(chain_id, Some(name.to_string())).with_storage_path(wallet_path);
            wallet.save()?;
            Ok(wallet)
        }
    }

    pub fn create(chain_id: u64, name: &str) -> Result<Self, WalletError> {
        Self::create_in(std::env::current_dir()?, chain_id, name)
    }

    /// Load wallet from JSON file
    pub fn load_from<P: AsRef<Path>>(base_dir: P, name: &str) -> Result<Self, WalletError> {
        let wallet_path = Self::wallet_path_in(base_dir, name);

        if wallet_path.is_file() {
            let json_str = fs::read_to_string(&wallet_path)?;
            Ok(serde_json::from_str::<Self>(&json_str)?.with_storage_path(wallet_path))
        } else {
            Err(WalletError::FileNotFound(wallet_path.display().to_string()))
        }
    }

    pub fn load(name: &str) -> Result<Self, WalletError> {
        Self::load_from(std::env::current_dir()?, name)
    }

    /// Save wallet to JSON file (uses configured path or provided path)
    pub fn save(&self) -> Result<(), WalletError> {
        let path = self
            .storage_path
            .as_ref()
            .ok_or(WalletError::MissingStoragePath)?;
        self.save_to(path)
    }

    /// Save wallet to specific JSON file
    pub fn save_to<P: AsRef<Path>>(&self, path: P) -> Result<(), WalletError> {
        let json_str = serde_json::to_string_pretty(&self)?;
        fs::write(path, json_str)?;
        Ok(())
    }

    fn stage<R>(
        &self,
        apply: impl FnOnce(&mut Self) -> Result<R, WalletError>,
    ) -> Result<(Self, R), WalletError> {
        let mut staged = self.clone();
        let value = apply(&mut staged)?;
        Ok((staged, value))
    }

    fn stage_value<R>(&self, apply: impl FnOnce(&mut Self) -> R) -> (Self, R) {
        let mut staged = self.clone();
        let value = apply(&mut staged);
        (staged, value)
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
            let pos = notes.iter().position(|n| n.note == note.note)?;
            let removed_note = notes.remove(pos);
            println!("{:?}", removed_note);
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
        let asset_notes = self
            .avail
            .get(ticker)
            .filter(|n| !n.is_empty())
            .cloned()
            .ok_or_else(|| {
                WalletError::LowBalance(format!(
                    "Wallet {} has 0 balance",
                    self.name.as_deref().unwrap_or("Noname")
                ))
            })?;

        let note_amounts = asset_notes
            .iter()
            .map(|note| self.get_note_amount(&note.note))
            .collect::<Result<Vec<_>, _>>()?;

        let mut best_selection: Option<(Vec<InputNote>, u64)> = None;

        let mut consider_candidate = |notes: Vec<InputNote>, total: u64| {
            if total < amount {
                return;
            }

            let better = match &best_selection {
                None => true,
                Some((best_notes, best_total)) => {
                    let excess = total - amount;
                    let best_excess = best_total - amount;

                    excess < best_excess
                        || (excess == best_excess && notes.len() < best_notes.len())
                }
            };

            if better {
                best_selection = Some((notes, total));
            }
        };

        for (i, note1) in asset_notes.iter().enumerate() {
            consider_candidate(vec![note1.clone()], note_amounts[i]);

            for (j, note2) in asset_notes.iter().enumerate().skip(i + 1) {
                let Some(total) = note_amounts[i].checked_add(note_amounts[j]) else {
                    continue;
                };

                consider_candidate(vec![note1.clone(), note2.clone()], total);
            }
        }

        let Some((selected_notes, total_input)) = best_selection else {
            return Err(WalletError::LowBalance(
                "Insufficient balance even with two notes, consolidate".to_string(),
            ));
        };

        for note in &selected_notes {
            self.pull_from_avail(ticker, note.clone())?;
        }

        let change = if total_input == amount {
            Note::padding_note()
        } else {
            let change_amount = total_input - amount;
            let change_note = self.make_change_note(&selected_notes[0].note, change_amount);
            self.push_to_avail(ticker, change_note.clone())?;
            change_note.note
        };

        let inputs = match selected_notes.as_slice() {
            [note1] => [note1.clone(), InputNote::padding_note()],
            [note1, note2] => [note1.clone(), note2.clone()],
            _ => unreachable!("wallet input selection only supports one or two notes"),
        };

        Ok((inputs, change))
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
            return Err(WalletError::CantReadNoteValue);
        };
        Ok(amount.to_owned())
    }

    fn spend_to(&mut self, note: &Note) -> Result<Utxo, WalletError> {
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

    pub fn prepare_spend_to(&self, note: &Note) -> Result<(Self, Utxo), WalletError> {
        self.stage(|wallet| wallet.spend_to(note))
    }

    fn receive(&mut self, input_note: &InputNote) -> Result<Utxo, WalletError> {
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

    pub fn prepare_receive(&self, input_note: &InputNote) -> Result<(Self, Utxo), WalletError> {
        self.stage(|wallet| wallet.receive(input_note))
    }

    fn mint(&mut self, amount: u64, ticker: &str) -> Result<Utxo, WalletError> {
        let received_note: InputNote = self.receive_note(amount, ticker);

        let b = self.push_to_avail(&ticker, received_note.clone())?;
        debug!(balance = b, "updated wallet balance");

        Ok(Utxo::new_mint([
            received_note.note.clone(),
            Note::padding_note(),
        ]))
    }

    pub fn prepare_mint(&self, amount: u64, ticker: &str) -> Result<(Self, Utxo), WalletError> {
        self.stage(|wallet| wallet.mint(amount, ticker))
    }

    fn burn(
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

    pub fn prepare_burn(
        &self,
        burner_note: &InputNote,
        evm_address: &Element,
    ) -> Result<(Self, Utxo), WalletError> {
        self.stage(|wallet| wallet.burn(burner_note, evm_address))
    }

    fn receive_note(&mut self, amount: u64, ticker: &str) -> InputNote {
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

    pub fn prepare_receive_note(&self, amount: u64, ticker: &str) -> (Self, InputNote) {
        self.stage_value(|wallet| wallet.receive_note(amount, ticker))
    }

    fn import_note(&mut self, note: &Note) -> Result<(), WalletError> {
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

    pub fn prepare_import_note(&self, note: &Note) -> Result<(Self, ()), WalletError> {
        self.stage(|wallet| wallet.import_note(note))
    }

    fn get_address(&mut self, amount: u64, ticker: &str) -> CipheraAddress {
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

    pub fn prepare_get_address(&self, amount: u64, ticker: &str) -> (Self, CipheraAddress) {
        self.stage_value(|wallet| wallet.get_address(amount, ticker))
    }

    fn sync(&mut self, txns: &[TxnWithInfo]) -> Result<(), WalletError> {
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

    pub fn prepare_sync(&self, txns: &[TxnWithInfo]) -> Result<(Self, ()), WalletError> {
        self.stage(|wallet| wallet.sync(txns))
    }
}

#[cfg(test)]
mod wallet_tests {
    use super::*;
    use crate::address::decode_address;
    use element::Element;
    use tempdir::TempDir;
    use zk_primitives::InputNote;

    // Helper function to create a test wallet with known balance
    fn setup_wallet(notes: Vec<u64>, ticker: &str) -> Wallet {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.avail.insert(
            ticker.to_string(),
            notes.into_iter().map(create_input_note).collect::<Vec<_>>(),
        );
        wallet.balance = wallet.avail[ticker]
            .iter()
            .map(|n| *n.note.value.to_u64_array().first().unwrap())
            .sum();
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

    #[test]
    fn test_load_legacy_wallet_without_chain_id() {
        let wallet_dir = TempDir::new("legacy-wallet-load").unwrap();
        let wallet_path = Wallet::wallet_path_in(wallet_dir.path(), "legacy");

        let wallet = Wallet::random(5115, Some("legacy".to_string()));
        let mut wallet_json = serde_json::to_value(&wallet).unwrap();
        wallet_json.as_object_mut().unwrap().remove("chain_id");
        std::fs::write(
            &wallet_path,
            serde_json::to_string_pretty(&wallet_json).unwrap(),
        )
        .unwrap();

        let loaded_wallet = Wallet::load_from(wallet_dir.path(), "legacy").unwrap();
        assert_eq!(loaded_wallet.chain_id, None);
    }

    #[test]
    fn test_save_legacy_wallet_persists_chain_id_once_bound() {
        let wallet_dir = TempDir::new("legacy-wallet-save").unwrap();
        let wallet_path = Wallet::wallet_path_in(wallet_dir.path(), "legacy");

        let wallet = Wallet::random(5115, Some("legacy".to_string()));
        let mut wallet_json = serde_json::to_value(&wallet).unwrap();
        wallet_json.as_object_mut().unwrap().remove("chain_id");
        std::fs::write(
            &wallet_path,
            serde_json::to_string_pretty(&wallet_json).unwrap(),
        )
        .unwrap();

        let mut loaded_wallet = Wallet::load_from(wallet_dir.path(), "legacy").unwrap();
        loaded_wallet.chain_id = Some(5115);
        loaded_wallet.save().unwrap();

        let saved_wallet_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&wallet_path).unwrap()).unwrap();
        assert_eq!(
            saved_wallet_json.get("chain_id"),
            Some(&serde_json::json!(5115))
        );
    }

    // =====================================================================
    // find_note Tests
    // =====================================================================

    #[test]
    fn test_find_note_success_single_note() {
        let mut wallet = setup_wallet(vec![1000], "WCBTC");
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
        let mut wallet = setup_wallet(vec![400, 400, 400], "WCBTC");
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
        let mut wallet = setup_wallet(vec![1000], "WCBTC");
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
        let mut wallet = setup_wallet(vec![1000, 1000], "WCBTC");
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
        let mut wallet = setup_wallet(vec![100], "WCBTC");
        let result = wallet.find_note(1000, "WCBTC");
        assert!(result.is_ok());

        let _ = wallet.pull_from_avail("WCBTC", result.unwrap()).unwrap();
        assert_eq!(wallet.balance, 0);
    }

    // =====================================================================
    // select_input_notes Tests
    // =====================================================================

    // Path A1: Single exact note
    #[test]
    fn test_select_single_exact_match() {
        let mut wallet = setup_wallet(vec![1000], "WCBTC");
        let (inputs, change) = wallet.select_input_notes("WCBTC", 1000).unwrap();

        assert_ne!(inputs[0].note, InputNote::padding_note().note);
        assert_eq!(inputs[1].note, InputNote::padding_note().note);
        assert_eq!(change, Note::padding_note());
        assert_eq!(wallet.balance, 0);
    }

    // Path A2: Single note with change
    #[test]
    fn test_select_single_with_change() {
        let mut wallet = setup_wallet(vec![1000], "WCBTC");
        let (inputs, change) = wallet.select_input_notes("WCBTC", 600).unwrap();

        assert_ne!(inputs[0].note, InputNote::padding_note().note);
        assert_eq!(inputs[1].note, InputNote::padding_note().note);
        assert_ne!(change, Note::padding_note());

        let change_amount = *change.value.to_u64_array().first().unwrap();
        assert_eq!(change_amount, 400);
        assert_eq!(wallet.balance, 400);
    }

    // Path B1: Two notes exact
    #[test]
    fn test_select_two_exact() {
        let mut wallet = setup_wallet(vec![400, 600], "WCBTC");
        let (inputs, change) = wallet.select_input_notes("WCBTC", 1000).unwrap();

        assert_ne!(inputs[0].note, InputNote::padding_note().note);
        assert_ne!(inputs[1].note, InputNote::padding_note().note);
        assert_eq!(change, Note::padding_note());
        assert_eq!(wallet.balance, 0);
    }

    // Path B2: Two notes with change (THE BUG FIX TEST)
    #[test]
    fn test_select_two_with_change() {
        let mut wallet = setup_wallet(vec![400, 300], "WCBTC");
        let (inputs, change) = wallet.select_input_notes("WCBTC", 600).unwrap();

        assert_ne!(inputs[0].note, InputNote::padding_note().note);
        assert_ne!(inputs[1].note, InputNote::padding_note().note);
        assert_ne!(change, Note::padding_note());

        // THE BUG WAS HERE: change should be 100, not 200
        let change_amount = *change.value.to_u64_array().first().unwrap();
        assert_eq!(change_amount, 100, "change must use both note amounts");
        assert_eq!(wallet.balance, 100);
    }

    // Path B3: Two notes insufficient
    #[test]
    fn test_select_two_insufficient() {
        let mut wallet = setup_wallet(vec![100, 100], "WCBTC");
        let result = wallet.select_input_notes("WCBTC", 500);

        assert!(result.is_err());
        assert!(matches!(result, Err(WalletError::LowBalance(_))));
    }

    // Edge case: Large gap between available notes
    #[test]
    fn test_select_two_large_gap() {
        let mut wallet = setup_wallet(vec![500, 250], "WCBTC");
        let (inputs, change) = wallet.select_input_notes("WCBTC", 700).unwrap();

        assert_ne!(inputs[0].note, InputNote::padding_note().note);
        assert_ne!(inputs[1].note, InputNote::padding_note().note);

        let change_amount = *change.value.to_u64_array().first().unwrap();
        assert_eq!(change_amount, 50, "change = 500 + 250 - 700");
    }

    // =====================================================================
    // spend_to Tests
    // =====================================================================

    #[test]
    fn test_spend_to_exact_amount() {
        let mut wallet = setup_wallet(vec![1000], "WCBTC");
        let address = create_note_and_encode_address(1000);
        let note = Note::from(&decode_address(&address));

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
        let mut wallet = setup_wallet(vec![1000], "WCBTC");
        let address = create_note_and_encode_address(100);
        let note = Note::from(&decode_address(&address));

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
        let mut wallet = setup_wallet(vec![100], "WCBTC");
        let address = create_note_and_encode_address(1000);
        let note = Note::from(&decode_address(&address));

        let result = wallet.spend_to(&note);
        assert!(result.is_err());

        match result {
            Err(WalletError::LowBalance(_)) => (),
            _ => panic!("Expected LowBalance error"),
        }
    }

    #[test]
    fn test_spend_to_multiple_notes_required() {
        let mut wallet = setup_wallet(vec![600, 600], "WCBTC");
        let address = create_note_and_encode_address(1000);
        let note = Note::from(&decode_address(&address));
        let result = wallet.spend_to(&note);
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
        let mut wallet = setup_wallet(vec![400, 400, 400], "WCBTC");
        let address = create_note_and_encode_address(700);
        let note = Note::from(&decode_address(&address));
        let result = wallet.spend_to(&note);
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
        let mut wallet = setup_wallet(vec![], "WCBTC");
        let address = create_note_and_encode_address(1000);
        let note = Note::from(&decode_address(&address));
        let result = wallet.spend_to(&note);

        // Should fail due to low balance
        assert!(result.is_err());
    }

    #[test]
    fn test_spend_to_updates_balance_correctly() {
        let mut wallet = setup_wallet(vec![1000, 1000], "WCBTC");
        let initial_balance = wallet.balance;
        let address = create_note_and_encode_address(1000);
        let note = Note::from(&decode_address(&address));

        let result = wallet.spend_to(&note);
        assert!(result.is_ok());
        assert!(wallet.balance <= initial_balance);
    }

    // =====================================================================
    // Edge Cases and Integration Tests
    // =====================================================================

    #[test]
    fn test_consecutive_find_notes() {
        let mut wallet = setup_wallet(vec![1000, 1000], "WCBTC");
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

    #[test]
    fn test_select_input_notes_prefers_valid_single_note_over_closer_small_note() {
        let mut wallet = setup_wallet(vec![42_700_000_000_000_000, 3_000_000_000_000_000], "WCBTC");

        let (inputs, change) = wallet
            .select_input_notes("WCBTC", 14_000_000_000_000_000)
            .unwrap();

        assert_eq!(
            note_value(&inputs[0]),
            42_700_000_000_000_000,
            "selection must prefer the covering note instead of reusing the smaller note twice"
        );
        assert_eq!(inputs[1].note, InputNote::padding_note().note);
        assert_eq!(
            *change.value.to_u64_array().first().unwrap(),
            28_700_000_000_000_000
        );
        assert_eq!(wallet.balance, 31_700_000_000_000_000);
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
        let note = Note::from(&decode_address(&address));
        wallet.spend_to(&note).unwrap();
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
        let note = Note::from(&decode_address(&address));
        wallet.spend_to(&note).unwrap();
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
        let note = Note::from(&decode_address(&address));
        wallet.spend_to(&note).unwrap();
        assert_eq!(
            wallet.balance, 0,
            "exact two-note spend should leave zero balance"
        );
    }

    // =====================================================================
    // Helpers for mint / burn / receive / import_note / sync tests
    // =====================================================================

    use crate::rpc::TxnWithInfo;
    use primitives::{block_height::BlockHeight, hash::CryptoHash};
    use zk_primitives::{UtxoKind, UtxoProof, UtxoProofBytes, UtxoPublicInput};

    /// InputNote with a real WCBTC contract so `citrea_ticker_from_contract` resolves.
    fn create_wcbtc_input_note_with_contract(amount: u64) -> InputNote {
        let (kind, contract) = citrea_token_data("WCBTC");
        let pk = Element::from(99999u64);
        let address = hash_merge([pk, Element::ZERO]);
        InputNote::new(
            Note {
                kind,
                contract,
                address,
                psi: hash_merge([pk, pk]),
                value: Element::from(amount),
            },
            pk,
        )
    }

    /// Fake TxnWithInfo whose first output_commitment equals `commitment`.
    fn make_txn_with_commitment(commitment: Element) -> TxnWithInfo {
        TxnWithInfo {
            proof: UtxoProof {
                proof: UtxoProofBytes::default(),
                public_inputs: UtxoPublicInput {
                    input_commitments: [Element::ZERO, Element::ZERO],
                    output_commitments: [commitment, Element::ZERO],
                    messages: [Element::ZERO; 5],
                },
            },
            hash: CryptoHash::genesis(),
            index_in_block: 0,
            block_height: BlockHeight::default(),
            time: 0,
        }
    }

    /// Insert a WCBTC note into `wallet.pending` and return the Note
    /// so callers can compute its commitment for sync tests.
    fn add_pending_note(wallet: &mut Wallet, amount: u64) -> Note {
        let pk = Element::from(12345u64);
        let (kind, contract) = citrea_token_data("WCBTC");
        let address = hash_merge([pk, Element::ZERO]);
        let note = Note {
            kind,
            contract,
            address,
            psi: hash_merge([pk, pk]),
            value: Element::from(amount),
        };
        wallet
            .pending
            .entry("WCBTC".to_string())
            .or_default()
            .push(InputNote::new(note.clone(), pk));
        note
    }

    /// Push a key into `wallet.keys` and return the matching Note so
    /// `import_note` can find and claim it.
    fn make_importable_note(wallet: &mut Wallet, amount: u64) -> Note {
        let pk = Element::from(12345u64);
        wallet.keys.push(pk);
        let (kind, contract) = citrea_token_data("WCBTC");
        Note {
            kind,
            contract,
            address: hash_merge([pk, Element::ZERO]),
            psi: hash_merge([pk, pk]),
            value: Element::from(amount),
        }
    }

    // =====================================================================
    // mint() tests
    // =====================================================================

    #[test]
    fn test_mint_adds_note_to_avail() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        assert_eq!(wallet.avail["WCBTC"].len(), 1);
    }

    #[test]
    fn test_mint_increases_balance() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        assert_eq!(wallet.balance, 1000);
    }

    #[test]
    fn test_mint_returns_mint_utxo_kind() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let utxo = wallet.mint(1000, "WCBTC").unwrap();
        assert_eq!(utxo.kind, UtxoKind::Mint);
    }

    #[test]
    fn test_mint_output_note_has_correct_value() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let utxo = wallet.mint(500, "WCBTC").unwrap();
        let value = *utxo.output_notes[0].value.to_u64_array().first().unwrap();
        assert_eq!(value, 500);
    }

    #[test]
    fn test_mint_stores_key() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        assert_eq!(wallet.keys.len(), 1);
    }

    #[test]
    fn test_mint_multiple_notes_accumulate_balance() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        wallet.mint(500, "WCBTC").unwrap();
        assert_eq!(wallet.balance, 1500);
        assert_eq!(wallet.avail["WCBTC"].len(), 2);
    }

    #[test]
    fn test_mint_usdc_uses_correct_ticker() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(100, "USDC").unwrap();
        assert_eq!(wallet.avail["USDC"].len(), 1);
        assert!(!wallet.avail.contains_key("WCBTC"));
    }

    // =====================================================================
    // burn() tests
    // =====================================================================

    #[test]
    fn test_burn_removes_note_from_avail() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        wallet.burn(&burner_note, &Element::from(42u64)).unwrap();

        assert_eq!(wallet.avail["WCBTC"].len(), 0);
    }

    #[test]
    fn test_burn_decreases_balance() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        wallet.burn(&burner_note, &Element::from(42u64)).unwrap();

        assert_eq!(wallet.balance, 0);
    }

    #[test]
    fn test_burn_returns_burn_utxo_kind() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        let utxo = wallet.burn(&burner_note, &Element::from(42u64)).unwrap();

        assert_eq!(utxo.kind, UtxoKind::Burn);
    }

    #[test]
    fn test_burn_note_not_in_avail_returns_error() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let (kind, contract) = citrea_token_data("WCBTC");
        let pk = Element::from(12345u64);
        let input_note = InputNote::new(
            Note {
                kind,
                contract,
                address: hash_merge([pk, Element::ZERO]),
                psi: hash_merge([pk, pk]),
                value: Element::from(1000u64),
            },
            pk,
        );

        let result = wallet.burn(&input_note, &Element::from(42u64));

        assert!(matches!(result, Err(WalletError::CantPullNote)));
    }

    #[test]
    fn test_burn_partial_avail_removes_only_burned_note() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        wallet.mint(500, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        wallet.burn(&burner_note, &Element::from(42u64)).unwrap();

        assert_eq!(wallet.avail["WCBTC"].len(), 1);
        assert_eq!(wallet.balance, 500);
    }

    // =====================================================================
    // receive() tests
    // =====================================================================

    #[test]
    fn test_receive_adds_note_to_avail() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let gifted_note = create_wcbtc_input_note_with_contract(1000);

        wallet.receive(&gifted_note).unwrap();

        assert_eq!(wallet.avail["WCBTC"].len(), 1);
    }

    #[test]
    fn test_receive_increases_balance() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let gifted_note = create_wcbtc_input_note_with_contract(500);

        wallet.receive(&gifted_note).unwrap();

        assert_eq!(wallet.balance, 500);
    }

    #[test]
    fn test_receive_returns_send_utxo_kind() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let gifted_note = create_wcbtc_input_note_with_contract(1000);

        let utxo = wallet.receive(&gifted_note).unwrap();

        assert_eq!(utxo.kind, UtxoKind::Send);
    }

    #[test]
    fn test_receive_creates_fresh_note_not_gifted_note() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let gifted_note = create_wcbtc_input_note_with_contract(1000);

        wallet.receive(&gifted_note).unwrap();

        // The note in avail must be a freshly-owned note, not the gifted one.
        let avail_note = &wallet.avail["WCBTC"][0];
        assert_ne!(avail_note.secret_key, gifted_note.secret_key);
    }

    // =====================================================================
    // import_note() tests
    // =====================================================================

    #[test]
    fn test_import_note_adds_to_avail() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let note = make_importable_note(&mut wallet, 1000);

        wallet.import_note(&note).unwrap();

        assert_eq!(wallet.avail["WCBTC"].len(), 1);
    }

    #[test]
    fn test_import_note_increases_balance() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let note = make_importable_note(&mut wallet, 750);

        wallet.import_note(&note).unwrap();

        assert_eq!(wallet.balance, 750);
    }

    #[test]
    fn test_import_note_removes_used_key() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let note = make_importable_note(&mut wallet, 1000);
        assert_eq!(wallet.keys.len(), 1);

        wallet.import_note(&note).unwrap();

        assert_eq!(wallet.keys.len(), 0);
    }

    #[test]
    fn test_import_note_unknown_address_returns_error() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let (kind, contract) = citrea_token_data("WCBTC");
        // Address does not correspond to any key in wallet.keys.
        let note = Note {
            kind,
            contract,
            address: Element::from(99999u64),
            psi: Element::ZERO,
            value: Element::from(1000u64),
        };

        let result = wallet.import_note(&note);

        assert!(matches!(result, Err(WalletError::KeyNotFound(_))));
    }

    #[test]
    fn test_import_note_does_not_remove_other_keys() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        // Insert an unrelated key before the importable one.
        let unrelated_key = Element::from(55555u64);
        wallet.keys.push(unrelated_key);
        let note = make_importable_note(&mut wallet, 1000);

        wallet.import_note(&note).unwrap();

        // Only the matched key (12345) is removed; unrelated_key stays.
        assert_eq!(wallet.keys.len(), 1);
        assert_eq!(wallet.keys[0], unrelated_key);
    }

    // =====================================================================
    // get_address() tests
    // =====================================================================

    #[test]
    fn test_get_address_stores_key() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        assert_eq!(wallet.keys.len(), 0);

        wallet.get_address(1000, "WCBTC");

        assert_eq!(wallet.keys.len(), 1);
    }

    #[test]
    fn test_get_address_adds_to_pending() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));

        wallet.get_address(1000, "WCBTC");

        assert!(wallet.pending.contains_key("WCBTC"));
        assert_eq!(wallet.pending["WCBTC"].len(), 1);
    }

    #[test]
    fn test_get_address_pending_note_has_correct_amount() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));

        wallet.get_address(750, "WCBTC");

        let pending_note = &wallet.pending["WCBTC"][0];
        let value = *pending_note.note.value.to_u64_array().first().unwrap();
        assert_eq!(value, 750);
    }

    #[test]
    fn test_get_address_returns_correct_value() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));

        let addr = wallet.get_address(1000, "WCBTC");

        let value = *addr.value.to_u64_array().first().unwrap();
        assert_eq!(value, 1000);
    }

    // =====================================================================
    // sync() tests
    // =====================================================================

    #[test]
    fn test_sync_moves_pending_to_avail() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let note = add_pending_note(&mut wallet, 1000);
        let txn = make_txn_with_commitment(note.commitment());

        wallet.sync(&vec![txn]).unwrap();

        assert!(wallet.pending["WCBTC"].is_empty());
        assert_eq!(wallet.avail["WCBTC"].len(), 1);
    }

    #[test]
    fn test_sync_increases_balance() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let note = add_pending_note(&mut wallet, 1000);
        let txn = make_txn_with_commitment(note.commitment());

        wallet.sync(&vec![txn]).unwrap();

        assert_eq!(wallet.balance, 1000);
    }

    #[test]
    fn test_sync_ignores_zero_commitment() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        add_pending_note(&mut wallet, 1000);
        // Zero commitment is a padding note and must be skipped.
        let txn = make_txn_with_commitment(Element::ZERO);

        wallet.sync(&vec![txn]).unwrap();

        assert_eq!(wallet.pending["WCBTC"].len(), 1);
        assert_eq!(wallet.balance, 0);
    }

    #[test]
    fn test_sync_nonmatching_commitment_leaves_pending() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        add_pending_note(&mut wallet, 1000);
        // A commitment that belongs to no pending note.
        let txn = make_txn_with_commitment(Element::from(999u64));

        wallet.sync(&vec![txn]).unwrap();

        assert_eq!(wallet.pending["WCBTC"].len(), 1);
        assert_eq!(wallet.balance, 0);
    }

    #[test]
    fn test_sync_empty_txns_changes_nothing() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        add_pending_note(&mut wallet, 1000);

        wallet.sync(&vec![]).unwrap();

        assert_eq!(wallet.pending["WCBTC"].len(), 1);
        assert_eq!(wallet.balance, 0);
    }

    #[test]
    fn test_sync_multiple_pending_only_matching_confirmed() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let note1 = add_pending_note(&mut wallet, 1000);
        add_pending_note(&mut wallet, 500);
        // Confirm only note1.
        let txn = make_txn_with_commitment(note1.commitment());

        wallet.sync(&vec![txn]).unwrap();

        assert_eq!(wallet.avail["WCBTC"].len(), 1);
        assert_eq!(wallet.pending["WCBTC"].len(), 1);
        assert_eq!(wallet.balance, 1000);
    }

    #[test]
    fn test_prepare_mint_leaves_original_wallet_unchanged() {
        let wallet = Wallet::random(5115, Some("test".to_string()));

        let (prepared_wallet, _) = wallet.prepare_mint(1000, "WCBTC").unwrap();

        assert_eq!(wallet.balance, 0);
        assert!(!wallet.avail.contains_key("WCBTC"));
        assert_eq!(prepared_wallet.balance, 1000);
        assert_eq!(prepared_wallet.avail["WCBTC"].len(), 1);
    }

    #[test]
    fn test_prepare_spend_to_leaves_original_wallet_unchanged() {
        let wallet = setup_wallet(vec![1000], "WCBTC");
        let note = Note::from(&decode_address(&create_note_and_encode_address(400)));

        let (prepared_wallet, _) = wallet.prepare_spend_to(&note).unwrap();

        assert_eq!(wallet.balance, 1000);
        assert_eq!(wallet.avail["WCBTC"].len(), 1);
        assert_eq!(prepared_wallet.balance, 600);
        assert_eq!(prepared_wallet.avail["WCBTC"].len(), 1);
    }

    #[test]
    fn test_prepare_get_address_leaves_original_wallet_unchanged() {
        let wallet = Wallet::random(5115, Some("test".to_string()));

        let (prepared_wallet, _) = wallet.prepare_get_address(1000, "WCBTC");

        assert!(wallet.keys.is_empty());
        assert!(wallet.pending.is_empty());
        assert_eq!(prepared_wallet.keys.len(), 1);
        assert_eq!(prepared_wallet.pending["WCBTC"].len(), 1);
    }
}
