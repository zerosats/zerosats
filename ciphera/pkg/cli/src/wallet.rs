use element::Element;
use hash::hash_merge;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::fs;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use zk_primitives::{
    CitreaNetwork, Escrow, EscrowInputNote, InputNote, Note, TimeLock, TimeProof, Utxo, UtxoKind,
};

use crate::escrow::{htlc_claim_address, htlc_refund_psi};

use crate::CipheraAddress;
use crate::address::{
    CLI_NETWORK, citrea_ticker_from_contract, citrea_token_data, network_for_chain,
    normalize_citrea_ticker,
};
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
    pub pending: HashMap<String, Vec<InputNote>>,
    pub avail: HashMap<String, Vec<InputNote>>,
    /// Append-only keystore of every receive secret key ever handed out by
    /// [`get_address`](Self::get_address). `pending` only holds the *most
    /// recent* address per ticker (each `get_address` replaces the list) and
    /// is drained as notes are spent, so an HTLC escrow locked to an earlier
    /// receive address would otherwise lose its claim key. This list is never
    /// truncated, guaranteeing [`candidate_secret_keys`](Self::candidate_secret_keys)
    /// can always recover it. `#[serde(default)]` keeps wallets saved by older
    /// CLI versions loadable.
    #[serde(default)]
    pub receive_keys: Vec<Element>,
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
            pending: HashMap::new(),
            avail: HashMap::new(),
            receive_keys: Vec::new(),
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
            pending: HashMap::new(),
            avail: HashMap::new(),
            receive_keys: Vec::new(),
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

    /// Citrea network this wallet operates on, derived from its bound
    /// `chain_id`. Used when *constructing* notes so the WCBTC `note_kind`
    /// matches the wallet's chain instead of a hardcoded default, and as the
    /// single source of truth for the CLI's per-command network (only wallet
    /// creation takes an explicit `--chain`). Legacy wallets that predate
    /// `chain_id` fall back to [`CLI_NETWORK`].
    pub fn network(&self) -> CitreaNetwork {
        self.chain_id.map_or(CLI_NETWORK, network_for_chain)
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
            let mut wallet = serde_json::from_str::<Self>(&json_str)?;
            wallet.normalize_asset_maps();
            Ok(wallet.with_storage_path(wallet_path))
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

    fn canonical_asset_key(ticker: &str) -> String {
        normalize_citrea_ticker(ticker)
            .map(ToString::to_string)
            .unwrap_or_else(|| ticker.trim().to_string())
    }

    fn normalize_note_map(map: &mut HashMap<String, Vec<InputNote>>) {
        let old = std::mem::take(map);
        for (ticker, notes) in old {
            map.entry(Self::canonical_asset_key(&ticker))
                .or_default()
                .extend(notes);
        }
    }

    fn normalize_asset_maps(&mut self) {
        Self::normalize_note_map(&mut self.pending);
        Self::normalize_note_map(&mut self.avail);
    }

    fn push_to_avail(&mut self, ticker: &str, note: InputNote) -> Result<u64, WalletError> {
        let ticker = Self::canonical_asset_key(ticker);
        self.avail.entry(ticker).or_default().push(note.clone());
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
        self.normalize_asset_maps();
        let ticker = Self::canonical_asset_key(ticker);
        let opt_balance = self.avail.get_mut(&ticker).and_then(|notes| {
            let pos = notes.iter().position(|n| n.note == note.note)?;
            let removed_note = notes.remove(pos);
            println!("{removed_note:?}");
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
                utxo_kind: origin_note.utxo_kind,
                note_kind: origin_note.note_kind,
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
        self.normalize_asset_maps();
        let ticker = Self::canonical_asset_key(ticker);
        let asset_notes = self
            .avail
            .get(&ticker)
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
            self.pull_from_avail(&ticker, note.clone())?;
        }

        let change = if total_input == amount {
            Note::padding_note()
        } else {
            let change_amount = total_input - amount;
            let change_note = self.make_change_note(&selected_notes[0].note, change_amount);
            self.push_to_avail(&ticker, change_note.clone())?;
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
        self.normalize_asset_maps();
        let ticker = Self::canonical_asset_key(ticker);
        let asset_notes = self
            .avail
            .get_mut(&ticker)
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
        let ticker = citrea_ticker_from_contract(note.note_kind);
        let amount = self.get_note_amount(note)?;

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
        let ticker = citrea_ticker_from_contract(input_note.note.note_kind);
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
        let ticker = Self::canonical_asset_key(ticker);
        let received_note: InputNote = self.receive_note(amount, &ticker);

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
        natively_substitute: bool,
    ) -> Result<Utxo, WalletError> {
        let ticker = citrea_ticker_from_contract(burner_note.note.note_kind);

        let b = self.pull_from_avail(&ticker, burner_note.to_owned())?;
        debug!(balance = b, "pulled first input note");

        if natively_substitute {
            Ok(Utxo::new_burn(
                [burner_note.to_owned(), InputNote::padding_note()],
                evm_address.to_owned(),
            ))
        } else {
            Ok(Utxo::new_burn_no_sub(
                [burner_note.to_owned(), InputNote::padding_note()],
                evm_address.to_owned(),
            ))
        }
    }

    pub fn prepare_burn(
        &self,
        burner_note: &InputNote,
        evm_address: &Element,
        natively_substitute: bool,
    ) -> Result<(Self, Utxo), WalletError> {
        self.stage(|wallet| wallet.burn(burner_note, evm_address, natively_substitute))
    }

    fn receive_note(&mut self, amount: u64, ticker: &str) -> InputNote {
        let pk = self.gen_pk();
        let self_address = hash_merge([pk, Element::ZERO]);

        let (utxo_kind, note_kind) = citrea_token_data(self.network(), ticker);

        let note = Note {
            utxo_kind,
            note_kind,
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

    // -------- HTLC escrow flow --------
    //
    // The three methods below mirror the three sides of a SHA-256 HTLC:
    // `lock` spends a normal note from `self`'s available balance and
    // emits a hashlock+timelock-encumbered output; `redeem` spends that
    // output by revealing the preimage; `refund` spends the same output
    // after the PoW timelock has elapsed. Lock uses the `Utxo` circuit
    // (inputs are normal Poseidon-key notes); redeem and refund use the
    // `Escrow` circuit with `spend_type == 3` (HTLC) inputs. All three
    // keep `note_kind` constant across inputs/outputs so the
    // `is_multiple_kinds` check inside each leaf circuit is satisfied.

    fn escrow_lock(
        &mut self,
        amount: u64,
        ticker: &str,
        htlc_secret_key: Element,
        preimage: [u8; 32],
        lock: &TimeLock,
    ) -> Result<(Utxo, EscrowInputNote), WalletError> {
        let (inputs, change) = self.select_input_notes(ticker, amount)?;
        let (utxo_kind, note_kind) = citrea_token_data(self.network(), ticker);

        let htlc_note = Note {
            utxo_kind,
            note_kind,
            address: htlc_claim_address(htlc_secret_key, preimage),
            // Bind the refund branch to the real timelock (anchored to the
            // current Bitcoin tip); the refund spend must later present a
            // PoW chain extending `lock.zero_block` by `lock.n_blocks`.
            psi: htlc_refund_psi(htlc_secret_key, lock),
            value: Element::from(amount),
        };

        // EscrowInputNote default sets spend_type=0; pin it to the HTLC
        // branch (3) up-front and stash the witness data so the redeem
        // / refund CLI commands can spend it without re-deriving
        // anything. The lock (anchor + required work) is persisted via
        // TimeProof serialization, but headers are filled in at refund time
        // from real Bitcoin blocks. The persisted JSON is what gets passed
        // between alice (locker) and bob (redeemer) in a real two-party flow.
        let escrow_input_note = EscrowInputNote {
            note: htlc_note.clone(),
            spend_type: 3,
            secret_key: htlc_secret_key,
            preimage,
            time_proof: TimeProof {
                lock: lock.clone(),
                ..TimeProof::default()
            },
        };

        Ok((
            Utxo::new_send(inputs, [htlc_note, change]),
            escrow_input_note,
        ))
    }

    pub fn prepare_escrow_lock(
        &self,
        amount: u64,
        ticker: &str,
        htlc_secret_key: Element,
        preimage: [u8; 32],
        lock: &TimeLock,
    ) -> Result<(Self, Utxo, EscrowInputNote), WalletError> {
        let mut staged = self.clone();
        let (utxo, escrow_note) =
            staged.escrow_lock(amount, ticker, htlc_secret_key, preimage, lock)?;
        Ok((staged, utxo, escrow_note))
    }

    /// Commit funds into a pre-built HTLC escrow note. Unlike
    /// `escrow_lock`, the output note's commitment fields (address, psi,
    /// value, note_kind, utxo_kind) are dictated by an external party
    /// -- the ln-service `/v0/offramp` response -- which is the redeemer
    /// and bound the claim branch to its own key and the bolt11 payment
    /// hash. This wallet is only the locker: it spends inputs to
    /// materialise the note, keeps no preimage, and persists no
    /// `EscrowInputNote`. The ticker is derived from the note's
    /// `note_kind` so input selection stays on the same asset.
    fn escrow_lock_to_note(&mut self, htlc_note: Note) -> Result<Utxo, WalletError> {
        let ticker = citrea_ticker_from_contract(htlc_note.note_kind);
        let amount = self.get_note_amount(&htlc_note)?;
        let (inputs, change) = self.select_input_notes(&ticker, amount)?;
        Ok(Utxo::new_send(inputs, [htlc_note, change]))
    }

    pub fn prepare_escrow_lock_to_note(
        &self,
        htlc_note: Note,
    ) -> Result<(Self, Utxo), WalletError> {
        let mut staged = self.clone();
        let utxo = staged.escrow_lock_to_note(htlc_note)?;
        Ok((staged, utxo))
    }

    fn escrow_redeem(
        &mut self,
        htlc_input_note: &EscrowInputNote,
    ) -> Result<(Escrow, InputNote), WalletError> {
        let ticker = citrea_ticker_from_contract(htlc_input_note.note.note_kind);
        let amount = self.get_note_amount(&htlc_input_note.note)?;

        // Force the claim branch even if the caller passed a stale
        // EscrowInputNote default.
        let redeem_input = EscrowInputNote {
            note: htlc_input_note.note.clone(),
            spend_type: 3,
            secret_key: htlc_input_note.secret_key,
            preimage: htlc_input_note.preimage,
            ..EscrowInputNote::default()
        };

        let received: InputNote = self.receive_note(amount, &ticker);
        let b = self.push_to_avail(&ticker, received.clone())?;
        debug!(balance = b, "updated wallet balance after redeem");

        let escrow = Escrow {
            kind: UtxoKind::Send,
            input_notes: [redeem_input, EscrowInputNote::padding_note()],
            output_notes: [received.note.clone(), Note::padding_note()],
            burn_address: None,
        };

        Ok((escrow, received))
    }

    fn candidate_secret_keys(&self) -> Vec<Element> {
        let mut keys = vec![self.pk];
        // Durable receive keys first: these survive `get_address` overwrites
        // and note spends, so a stale redeemer address can still be claimed.
        keys.extend(self.receive_keys.iter().copied());
        keys.extend(
            self.pending
                .values()
                .chain(self.avail.values())
                .flat_map(|notes| notes.iter().map(|note| note.secret_key)),
        );
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter().filter(|key| !key.is_zero()).collect()
    }

    fn htlc_claim_secret_key(
        &self,
        htlc_note: &Note,
        preimage: [u8; 32],
    ) -> Result<Element, WalletError> {
        self.candidate_secret_keys()
            .into_iter()
            .find(|secret_key| htlc_claim_address(*secret_key, preimage) == htlc_note.address)
            .ok_or(WalletError::NoKey)
    }

    pub fn prepare_escrow_redeem(
        &self,
        htlc_input_note: &EscrowInputNote,
    ) -> Result<(Self, Escrow, InputNote), WalletError> {
        let mut staged = self.clone();
        let (escrow, received) = staged.escrow_redeem(htlc_input_note)?;
        Ok((staged, escrow, received))
    }

    pub fn prepare_escrow_redeem_note(
        &self,
        htlc_note: &Note,
        preimage: [u8; 32],
    ) -> Result<(Self, Escrow, InputNote), WalletError> {
        let secret_key = self.htlc_claim_secret_key(htlc_note, preimage)?;
        let htlc_input_note = EscrowInputNote {
            note: htlc_note.clone(),
            spend_type: 3,
            secret_key,
            preimage,
            ..EscrowInputNote::default()
        };
        self.prepare_escrow_redeem(&htlc_input_note)
    }

    /// Assemble the HTLC **refund** witness.
    ///
    /// `spend_type = 3` with an all-zero preimage selects the timelocked
    /// refund branch (a non-zero preimage would instead select the hash/claim
    /// branch). `time_proof` carries the lock anchor the refund must extend —
    /// just the lock when persisting the witness at lock time, plus the real
    /// PoW headers once the refund is actually spent.
    ///
    /// Single home for the "spend_type 3 + zero preimage = refund" protocol
    /// convention, shared by the escrow-lock / ln-withdraw persist paths and
    /// the refund-spend path in [`escrow_refund`](Self::escrow_refund) so the
    /// three can't drift.
    #[must_use]
    pub fn refund_witness(
        note: Note,
        refund_secret_key: Element,
        time_proof: TimeProof,
    ) -> EscrowInputNote {
        EscrowInputNote {
            note,
            spend_type: 3,
            secret_key: refund_secret_key,
            preimage: [0u8; 32],
            time_proof,
        }
    }

    fn escrow_refund(
        &mut self,
        htlc_input_note: &EscrowInputNote,
        time_proof: TimeProof,
    ) -> Result<(Escrow, InputNote), WalletError> {
        let ticker = citrea_ticker_from_contract(htlc_input_note.note.note_kind);
        let amount = self.get_note_amount(&htlc_input_note.note)?;

        // Refund branch: the PoW witness extending the lock anchor is attached
        // (`time_proof`, built from real Bitcoin headers by the caller).
        // `secret_key` must match the one whose `key_hash` was baked into `psi`
        // at lock time, and `time_proof.lock` must be the lock that `psi`
        // committed to.
        let refund_input = Self::refund_witness(
            htlc_input_note.note.clone(),
            htlc_input_note.secret_key,
            time_proof,
        );

        let received: InputNote = self.receive_note(amount, &ticker);
        let b = self.push_to_avail(&ticker, received.clone())?;
        debug!(balance = b, "updated wallet balance after refund");

        let escrow = Escrow {
            kind: UtxoKind::Send,
            input_notes: [refund_input, EscrowInputNote::padding_note()],
            output_notes: [received.note.clone(), Note::padding_note()],
            burn_address: None,
        };

        Ok((escrow, received))
    }

    pub fn prepare_escrow_refund(
        &self,
        htlc_input_note: &EscrowInputNote,
        time_proof: TimeProof,
    ) -> Result<(Self, Escrow, InputNote), WalletError> {
        let mut staged = self.clone();
        let (escrow, received) = staged.escrow_refund(htlc_input_note, time_proof)?;
        Ok((staged, escrow, received))
    }

    fn import_note(&mut self, note: &Note) -> Result<(), WalletError> {
        self.normalize_asset_maps();
        for ticker in self.pending.keys().cloned().collect::<Vec<_>>() {
            let asset_notes = self.pending.get(&ticker).unwrap();
            if let Some(pos) = asset_notes
                .iter()
                .position(|n| n.note.address == note.address)
            {
                let pending_note = self.pending.get_mut(&ticker).unwrap().remove(pos);
                let amount = self.get_note_amount(note)?;

                debug!(ticker = ticker, amount = amount, "importing note");

                let b = self.push_to_avail(
                    &ticker,
                    InputNote::new(note.clone(), pending_note.secret_key),
                )?;

                debug!(balance = b, "updated wallet balance");

                return Ok(());
            }
        }
        Err(WalletError::KeyNotFound(format!("Cant import {note:?}")))
    }

    pub fn prepare_import_note(&self, note: &Note) -> Result<(Self, ()), WalletError> {
        self.stage(|wallet| wallet.import_note(note))
    }

    /// Directly add a known `InputNote` (with its embedded secret key) to the
    /// available notes. Use this when you already hold the `InputNote` value
    /// (e.g. a self-send burner note) rather than searching `pending` by address.
    fn add_to_avail(&mut self, input_note: InputNote) -> Result<(), WalletError> {
        let ticker = citrea_ticker_from_contract(input_note.note.note_kind);
        self.push_to_avail(&ticker, input_note).map(|_| ())
    }

    pub fn prepare_add_to_avail(&self, input_note: InputNote) -> Result<(Self, ()), WalletError> {
        self.stage(|wallet| wallet.add_to_avail(input_note))
    }

    fn get_address(&mut self, amount: u64, ticker: &str) -> CipheraAddress {
        let ticker = Self::canonical_asset_key(ticker);
        let pk = self.gen_pk();
        let psi = self.gen_pk();
        let address = hash_merge([pk, Element::ZERO]);
        let (utxo_kind, note_kind) = citrea_token_data(self.network(), &ticker);

        let note = Note {
            utxo_kind,
            note_kind,
            address,
            psi,
            value: Element::new(amount),
        };
        // Persist the receive key durably *before* overwriting `pending`.
        // Without this, calling `address` again for the same ticker (or
        // spending the note) drops the key an HTLC escrow may be locked to,
        // leaving redeemable funds unreachable until the locker's refund.
        self.remember_receive_key(pk);
        self.pending
            .insert(ticker, vec![InputNote::new(note.clone(), pk)]);

        (&note).into()
    }

    /// Record a receive secret key in the append-only [`receive_keys`](Self::receive_keys)
    /// keystore, ignoring zero keys and duplicates.
    fn remember_receive_key(&mut self, secret_key: Element) {
        if !secret_key.is_zero() && !self.receive_keys.contains(&secret_key) {
            self.receive_keys.push(secret_key);
        }
    }

    pub fn prepare_get_address(&self, amount: u64, ticker: &str) -> (Self, CipheraAddress) {
        self.stage_value(|wallet| wallet.get_address(amount, ticker))
    }

    fn sync(&mut self, txns: &[TxnWithInfo]) -> Result<(), WalletError> {
        self.normalize_asset_maps();
        for tx in txns {
            let id = tx.hash;
            let block = tx.block_height;
            for c in tx.proof.public_inputs.output_commitments {
                if c != Element::ZERO {
                    // not a padding note
                    let mut new_notes = vec![];

                    for asset_notes in self.pending.values_mut() {
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
                        let ticker = citrea_ticker_from_contract(n.note.note_kind);
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
    use crate::address::{CITREA_USD_TICKER, decode_address};
    use crate::escrow::{pow_two_block_lock, pow_two_block_proof};
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
        let (utxo_kind, note_kind) = citrea_token_data(CitreaNetwork::Testnet, "WCBTC");

        let note = Note {
            utxo_kind,
            note_kind,
            address: hash_merge([Element::new(101), Element::ZERO]),
            psi: Element::ZERO,
            value: Element::new(amount),
        };

        let a: CipheraAddress = (&note).into();

        a.encode_address()
    }

    fn create_input_note(amount: u64) -> InputNote {
        let note = Note {
            utxo_kind: Element::new(2),
            note_kind: Element::ZERO,
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

    #[test]
    fn test_load_normalizes_legacy_usdc_asset_key() {
        let wallet_dir = TempDir::new("legacy-wallet-usdc").unwrap();
        let wallet_path = Wallet::wallet_path_in(wallet_dir.path(), "legacy-usdc");

        let mut wallet = Wallet::random(5115, Some("legacy-usdc".to_string()));
        let (utxo_kind, note_kind) = citrea_token_data(CitreaNetwork::Testnet, "USDC");
        let pk = Element::from(44u64);
        wallet.avail.insert(
            "USDC".to_string(),
            vec![InputNote::new(
                Note {
                    utxo_kind,
                    note_kind,
                    address: hash_merge([pk, Element::ZERO]),
                    psi: hash_merge([pk, pk]),
                    value: Element::from(10u64),
                },
                pk,
            )],
        );
        wallet.save_to(&wallet_path).unwrap();

        let loaded_wallet = Wallet::load_from(wallet_dir.path(), "legacy-usdc").unwrap();
        assert!(loaded_wallet.avail.contains_key(CITREA_USD_TICKER));
        assert!(!loaded_wallet.avail.contains_key("USDC"));
    }

    #[test]
    fn test_load_legacy_wallet_with_keys_field() {
        // Old wallet JSON files may contain a `keys` field. serde must silently
        // ignore it so existing wallets can be loaded after the refactoring.
        let wallet_dir = TempDir::new("legacy-wallet-keys").unwrap();
        let wallet_path = Wallet::wallet_path_in(wallet_dir.path(), "legacy-keys");

        let wallet = Wallet::random(5115, Some("legacy-keys".to_string()));
        let mut wallet_json = serde_json::to_value(&wallet).unwrap();
        wallet_json
            .as_object_mut()
            .unwrap()
            .insert("keys".to_string(), serde_json::json!([]));
        std::fs::write(
            &wallet_path,
            serde_json::to_string_pretty(&wallet_json).unwrap(),
        )
        .unwrap();

        let loaded_wallet = Wallet::load_from(wallet_dir.path(), "legacy-keys").unwrap();
        assert_eq!(loaded_wallet.chain_id, Some(5115));
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
        let (utxo_kind, note_kind) = citrea_token_data(CitreaNetwork::Testnet, "WCBTC");
        let pk = Element::from(99999u64);
        let address = hash_merge([pk, Element::ZERO]);
        InputNote::new(
            Note {
                utxo_kind,
                note_kind,
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
        let (utxo_kind, note_kind) = citrea_token_data(CitreaNetwork::Testnet, "WCBTC");
        let address = hash_merge([pk, Element::ZERO]);
        let note = Note {
            utxo_kind,
            note_kind,
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

    /// Add a WCBTC note to `wallet.pending` (mimicking `get_address`) so that
    /// `import_note` can find and claim it by matching the address.
    fn make_importable_note(wallet: &mut Wallet, amount: u64) -> Note {
        let pk = Element::from(12345u64);
        let (utxo_kind, note_kind) = citrea_token_data(CitreaNetwork::Testnet, "WCBTC");
        let note = Note {
            utxo_kind,
            note_kind,
            address: hash_merge([pk, Element::ZERO]),
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
    fn test_mint_note_has_embedded_key() {
        // After mint the note in avail must carry a non-zero secret key so it
        // can be spent later without a separate `keys` list.
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        let avail_note = &wallet.avail["WCBTC"][0];
        assert_ne!(avail_note.secret_key, Element::ZERO);
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
    fn test_mint_cusd_uses_correct_ticker() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(100, "CUSD").unwrap();
        assert_eq!(wallet.avail[CITREA_USD_TICKER].len(), 1);
        assert!(!wallet.avail.contains_key("WCBTC"));
    }

    #[test]
    fn test_mint_legacy_usdc_alias_uses_cusd_ticker() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(100, "USDC").unwrap();
        assert_eq!(wallet.avail[CITREA_USD_TICKER].len(), 1);
        assert!(!wallet.avail.contains_key("USDC"));
    }

    #[test]
    fn test_cusd_escrow_lock_uses_cusd_note_kind() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1_000, "CUSD").unwrap();
        let lock = pow_two_block_lock();
        let preimage = [7u8; 32];
        let htlc_secret_key = Element::from(123u64);

        let (utxo, htlc_input_note) = wallet
            .escrow_lock(400, "CUSD", htlc_secret_key, preimage, &lock)
            .unwrap();
        let (_, expected_note_kind) = citrea_token_data(CitreaNetwork::Testnet, "CUSD");

        assert_eq!(htlc_input_note.note.note_kind, expected_note_kind);
        assert_eq!(htlc_input_note.spend_type, 3);
        assert_eq!(utxo.output_notes[0].note_kind, expected_note_kind);
        assert_eq!(wallet.avail[CITREA_USD_TICKER].len(), 1);
    }

    #[test]
    fn test_cusd_escrow_redeem_adds_cusd_note() {
        let wallet = Wallet::random(5115, Some("test".to_string()));
        let (_, expected_note_kind) = citrea_token_data(CitreaNetwork::Testnet, "CUSD");
        let htlc_input_note = EscrowInputNote {
            note: Note {
                utxo_kind: Element::new(2),
                note_kind: expected_note_kind,
                address: Element::from(1u64),
                psi: Element::from(2u64),
                value: Element::from(400u64),
            },
            spend_type: 3,
            secret_key: Element::from(123u64),
            preimage: [7u8; 32],
            ..EscrowInputNote::default()
        };

        let (prepared_wallet, escrow, received) =
            wallet.prepare_escrow_redeem(&htlc_input_note).unwrap();

        assert_eq!(received.note.note_kind, expected_note_kind);
        assert_eq!(escrow.output_notes[0].note_kind, expected_note_kind);
        assert_eq!(prepared_wallet.avail[CITREA_USD_TICKER].len(), 1);
    }

    #[test]
    fn test_escrow_redeem_note_finds_redeemer_key_from_wallet() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let (_, expected_note_kind) = citrea_token_data(CitreaNetwork::Testnet, "CUSD");
        let redeemer_secret_key = Element::from(123u64);
        let preimage = [9u8; 32];

        wallet.pending.insert(
            CITREA_USD_TICKER.to_string(),
            vec![InputNote::new(
                Note {
                    utxo_kind: Element::new(2),
                    note_kind: expected_note_kind,
                    address: hash_merge([redeemer_secret_key, Element::ZERO]),
                    psi: Element::ZERO,
                    value: Element::from(400u64),
                },
                redeemer_secret_key,
            )],
        );

        let htlc_note = Note {
            utxo_kind: Element::new(2),
            note_kind: expected_note_kind,
            address: htlc_claim_address(redeemer_secret_key, preimage),
            psi: Element::from(2u64),
            value: Element::from(400u64),
        };

        let (prepared_wallet, escrow, received) = wallet
            .prepare_escrow_redeem_note(&htlc_note, preimage)
            .unwrap();

        assert_eq!(escrow.input_notes[0].secret_key, redeemer_secret_key);
        assert_eq!(escrow.input_notes[0].preimage, preimage);
        assert_eq!(received.note.note_kind, expected_note_kind);
        assert_eq!(prepared_wallet.avail[CITREA_USD_TICKER].len(), 1);
    }

    #[test]
    fn test_escrow_redeem_survives_second_get_address() {
        // Regression: generating a second receive address for the same
        // ticker overwrites `pending[ticker]`, dropping the first key from
        // the note map. Redeem must still recover it from the durable
        // `receive_keys` keystore instead of failing with `NoKey`.
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let (_, expected_note_kind) = citrea_token_data(CitreaNetwork::Testnet, "CUSD");
        let preimage = [9u8; 32];

        // Redeemer hands out a receive address; capture the key it binds.
        let _first = wallet.get_address(400, CITREA_USD_TICKER);
        let redeemer_secret_key = wallet.receive_keys[0];

        // A second `address` call for the same ticker replaces the pending
        // list, evicting the first key from `pending` entirely.
        let _second = wallet.get_address(500, CITREA_USD_TICKER);
        assert_eq!(wallet.pending[CITREA_USD_TICKER].len(), 1);
        assert_ne!(
            wallet.pending[CITREA_USD_TICKER][0].secret_key,
            redeemer_secret_key,
            "second get_address should have evicted the first key from pending",
        );
        assert!(
            wallet.receive_keys.contains(&redeemer_secret_key),
            "durable keystore must retain the evicted key",
        );

        // The escrow was locked to the *first* receive address.
        let htlc_note = Note {
            utxo_kind: Element::new(2),
            note_kind: expected_note_kind,
            address: htlc_claim_address(redeemer_secret_key, preimage),
            psi: Element::from(2u64),
            value: Element::from(400u64),
        };

        let (_prepared_wallet, escrow, _received) = wallet
            .prepare_escrow_redeem_note(&htlc_note, preimage)
            .expect("redeem must recover the key from the durable keystore");

        assert_eq!(escrow.input_notes[0].secret_key, redeemer_secret_key);
        assert_eq!(escrow.input_notes[0].preimage, preimage);
    }

    #[test]
    fn test_cusd_escrow_refund_adds_cusd_note() {
        let wallet = Wallet::random(5115, Some("test".to_string()));
        let (_, expected_note_kind) = citrea_token_data(CitreaNetwork::Testnet, "CUSD");
        let htlc_input_note = EscrowInputNote {
            note: Note {
                utxo_kind: Element::new(2),
                note_kind: expected_note_kind,
                address: Element::from(1u64),
                psi: Element::from(2u64),
                value: Element::from(400u64),
            },
            spend_type: 3,
            secret_key: Element::from(123u64),
            preimage: [0u8; 32],
            ..EscrowInputNote::default()
        };

        let (prepared_wallet, escrow, received) = wallet
            .prepare_escrow_refund(&htlc_input_note, pow_two_block_proof())
            .unwrap();

        assert_eq!(received.note.note_kind, expected_note_kind);
        assert_eq!(escrow.output_notes[0].note_kind, expected_note_kind);
        assert_eq!(prepared_wallet.avail[CITREA_USD_TICKER].len(), 1);
    }

    // =====================================================================
    // burn() tests
    // =====================================================================

    #[test]
    fn test_burn_removes_note_from_avail() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        wallet
            .burn(&burner_note, &Element::from(42u64), false)
            .unwrap();

        assert_eq!(wallet.avail["WCBTC"].len(), 0);
    }

    #[test]
    fn test_burn_decreases_balance() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        wallet
            .burn(&burner_note, &Element::from(42u64), false)
            .unwrap();

        assert_eq!(wallet.balance, 0);
    }

    #[test]
    fn test_burn_returns_burn_utxo_kind() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        let utxo = wallet
            .burn(&burner_note, &Element::from(42u64), true)
            .unwrap();

        assert_eq!(utxo.kind, UtxoKind::Burn);
    }

    #[test]
    fn test_burn_note_not_in_avail_returns_error() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let (utxo_kind, note_kind) = citrea_token_data(CitreaNetwork::Testnet, "WCBTC");
        let pk = Element::from(12345u64);
        let input_note = InputNote::new(
            Note {
                utxo_kind,
                note_kind,
                address: hash_merge([pk, Element::ZERO]),
                psi: hash_merge([pk, pk]),
                value: Element::from(1000u64),
            },
            pk,
        );

        let result = wallet.burn(&input_note, &Element::from(42u64), false);

        assert!(matches!(result, Err(WalletError::CantPullNote)));
    }

    #[test]
    fn test_burn_partial_avail_removes_only_burned_note() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        wallet.mint(1000, "WCBTC").unwrap();
        wallet.mint(500, "WCBTC").unwrap();
        let burner_note = wallet.avail["WCBTC"][0].clone();

        wallet
            .burn(&burner_note, &Element::from(42u64), false)
            .unwrap();

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
    fn test_import_note_removes_from_pending() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let note = make_importable_note(&mut wallet, 1000);
        assert_eq!(wallet.pending["WCBTC"].len(), 1);

        wallet.import_note(&note).unwrap();

        assert_eq!(wallet.pending["WCBTC"].len(), 0);
    }

    #[test]
    fn test_import_note_unknown_address_returns_error() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        let (utxo_kind, note_kind) = citrea_token_data(CitreaNetwork::Testnet, "WCBTC");
        // Address does not correspond to any pending note in the wallet.
        let note = Note {
            utxo_kind,
            note_kind,
            address: Element::from(99999u64),
            psi: Element::ZERO,
            value: Element::from(1000u64),
        };

        let result = wallet.import_note(&note);

        assert!(matches!(result, Err(WalletError::KeyNotFound(_))));
    }

    #[test]
    fn test_import_note_does_not_remove_other_pending_notes() {
        let mut wallet = Wallet::random(5115, Some("test".to_string()));
        // Add an unrelated pending note for Citrea USD, using the legacy alias.
        let pk_usd = Element::from(55555u64);
        let (kind_usd, contract_usd) = citrea_token_data(CitreaNetwork::Testnet, "USDC");
        let unrelated_note = InputNote::new(
            Note {
                utxo_kind: kind_usd,
                note_kind: contract_usd,
                address: hash_merge([pk_usd, Element::ZERO]),
                psi: hash_merge([pk_usd, pk_usd]),
                value: Element::from(500u64),
            },
            pk_usd,
        );
        wallet
            .pending
            .insert("USDC".to_string(), vec![unrelated_note]);

        // Add the importable WCBTC note.
        let note = make_importable_note(&mut wallet, 1000);

        wallet.import_note(&note).unwrap();

        // The Citrea USD pending note must not be touched.
        assert_eq!(wallet.pending[CITREA_USD_TICKER].len(), 1);
    }

    // =====================================================================
    // get_address() tests
    // =====================================================================

    #[test]
    fn test_get_address_pending_note_has_embedded_key() {
        // The secret key must be embedded inside the pending InputNote rather
        // than stored in a separate `keys` list.
        let mut wallet = Wallet::random(5115, Some("test".to_string()));

        wallet.get_address(1000, "WCBTC");

        let pending_note = &wallet.pending["WCBTC"][0];
        assert_ne!(pending_note.secret_key, Element::ZERO);
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

        wallet.sync(&[]).unwrap();

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

        assert!(wallet.pending.is_empty());
        assert_eq!(prepared_wallet.pending["WCBTC"].len(), 1);
        // The pending note in the prepared wallet must carry the secret key.
        assert_ne!(
            prepared_wallet.pending["WCBTC"][0].secret_key,
            Element::ZERO
        );
    }

    // =====================================================================
    // prepare_add_to_avail() tests
    // =====================================================================

    #[test]
    fn test_prepare_add_to_avail_adds_note() {
        let wallet = Wallet::random(5115, Some("test".to_string()));
        let note = create_wcbtc_input_note_with_contract(500);

        let (prepared_wallet, _) = wallet.prepare_add_to_avail(note).unwrap();

        assert_eq!(prepared_wallet.avail["WCBTC"].len(), 1);
        assert_eq!(prepared_wallet.balance, 500);
    }

    #[test]
    fn test_prepare_add_to_avail_leaves_original_unchanged() {
        let wallet = Wallet::random(5115, Some("test".to_string()));
        let note = create_wcbtc_input_note_with_contract(500);

        let (prepared_wallet, _) = wallet.prepare_add_to_avail(note).unwrap();

        assert_eq!(wallet.balance, 0);
        assert!(!wallet.avail.contains_key("WCBTC"));
        assert_eq!(prepared_wallet.balance, 500);
    }

    #[test]
    fn test_prepare_add_to_avail_preserves_secret_key() {
        let wallet = Wallet::random(5115, Some("test".to_string()));
        let note = create_wcbtc_input_note_with_contract(500);
        let expected_key = note.secret_key;

        let (prepared_wallet, _) = wallet.prepare_add_to_avail(note).unwrap();

        assert_eq!(prepared_wallet.avail["WCBTC"][0].secret_key, expected_key);
    }
}
