use crate::{
    NoteURLPayload, decode_activity_url_payload, get_address_for_private_key, note::Note,
};
use element::Element;
use serde::{Deserialize, Serialize};

/// Anchor + required-work specification for a timelocked spend path.
///
/// Mirrors the Noir `TimeLock` struct used by note kinds 7 and 8.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TimeLock {
    /// Bitcoin block hash used as the anchor (little-endian, as stored in headers).
    pub zero_block: [u8; 32],
    /// Number of additional PoW blocks required on top of the anchor.
    pub n_blocks: Element,
}

impl TimeLock {
    /// Poseidon commitment to the lock's `(zero_block, n_blocks)`. Matches
    /// the Noir `get_timelock_commitment` helper used by note kinds 7 and 8:
    /// `Poseidon([Poseidon(field_pair(zero_block)), n_blocks])`.
    #[must_use]
    pub fn commitment(&self) -> Element {
        let element = Element::from_be_bytes(self.zero_block);
        let (high, low) = element.decompose_be();
        let zero_block_hash = hash::hash_merge([high, low]);
        hash::hash_merge([zero_block_hash, self.n_blocks])
    }
}

/// Address for the timelocked spend path (kind 7) and the HTLC refund
/// path (kind 8): `Poseidon(get_secret_hash(secret_key), TimeLock::commitment)`.
#[must_use]
pub fn timelock_address(secret_key: Element, lock: &TimeLock) -> Element {
    let key_hash = get_address_for_private_key(secret_key);
    hash::hash_merge([key_hash, lock.commitment()])
}

/// PoW chain witness backing a timelocked spend.
///
/// Mirrors the Noir `TimeProof` struct used by note kinds 7 (timelock) and
/// 8 (HTLC refund path). The headers must chain from `lock.zero_block`.
#[derive(Clone, Debug)]
pub struct TimeProof {
    /// The anchor and required number of subsequent blocks.
    pub lock: TimeLock,
    /// Block headers chaining from `lock.zero_block`.
    pub headers: [[u8; 80]; 2],
}

impl Default for TimeProof {
    fn default() -> Self {
        Self {
            lock: TimeLock::default(),
            headers: [[0u8; 80]; 2],
        }
    }
}

/// InputNote is a Note that belongs to the current user, i.e. they have the
/// spending sercret key and can therefore use it as an input, "spending" the note. Extra
/// constraints need to be applied to input notes to ensure they are valid.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InputNote {
    /// The note to spend
    pub note: Note,
    /// Selects which spend path the circuit enforces. Mirrors the Noir
    /// `InputNote.spend_type` field:
    /// `0` = Poseidon-key ownership (with optional timelock, kinds 1..4 and 7),
    /// `1` = signature32 preimage (kind 5),
    /// `2` = signature32sha preimage (kind 6),
    /// `3` = HTLC (kind 8 -- preimage spends the hash path, all-zero preimage
    ///       falls back to the timelocked refund path).
    #[serde(default)]
    pub spend_type: u8,
    /// Secret key for the address, required to spend a note via the Poseidon
    /// ownership path (note kinds 1..4 and the kind-7/8 refund path).
    pub secret_key: Element,
    /// Preimage witness for the kinds that prove ownership by revealing a
    /// 32-byte preimage (kind 5, kind 6, and the kind-8 hash path). Zero
    /// for note kinds that don't use it.
    #[serde(default)]
    pub preimage: [u8; 32],
    /// Bitcoin PoW witness for the timelocked spend paths (kind 7, and the
    /// kind-8 refund path). Not serialized -- it's a proving-time witness,
    /// not part of the persisted note.
    #[serde(skip)]
    pub time_proof: TimeProof,
}

impl InputNote {
    /// Create a new input note for the standard Poseidon-key ownership path.
    /// `preimage` and `time_proof` default to zero -- callers that need them
    /// (kinds 5/6/7/8) should construct `InputNote` directly.
    #[must_use]
    pub fn new(note: Note, secret_key: Element) -> Self {
        Self {
            note,
            secret_key,
            ..Self::default()
        }
    }

    /// Create a new padding note
    #[must_use]
    pub fn padding_note() -> Self {
        Self::default()
    }

    /// Generates a new note with given value, for an ephemeral private key, the private key
    /// must only be used once
    #[must_use]
    pub fn new_from_ephemeral_private_key(private_key: Element, value: Element) -> Self {
        Self::new(
            Note::new_from_ephemeral_private_key(private_key, value),
            private_key,
        )
    }

    /// Generates an InputNote from a link string e.g. /s#A0F3...
    #[must_use]
    pub fn new_from_link(link: &str) -> Self {
        InputNote::from(&decode_activity_url_payload(link))
    }

    /// Generates a Ciphera link from the Note + Private Key
    #[must_use]
    pub fn generate_link(&self) -> String {
        let payload: NoteURLPayload = self.into();
        payload.encode_activity_url_payload()
    }
}
