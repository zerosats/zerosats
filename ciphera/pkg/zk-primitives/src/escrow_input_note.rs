use crate::{get_address_for_private_key, note::Note};
use element::Element;
use serde::{Deserialize, Serialize, Serializer, Deserializer};

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
///
/// Serializes only the `lock` (anchor + required work); `headers` are
/// proving-time witnesses and are not persisted.
#[derive(Clone, Debug)]
pub struct TimeProof {
    /// The anchor and required number of subsequent blocks.
    pub lock: TimeLock,
    /// Block headers chaining from `lock.zero_block`.
    /// Not serialized (custom impl) -- filled in at refund time from real Bitcoin headers.
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

impl Serialize for TimeProof {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.lock.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TimeProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let lock = TimeLock::deserialize(deserializer)?;
        Ok(TimeProof {
            lock,
            headers: [[0u8; 80]; 2],
        })
    }
}

/// EscrowInputNote is a Note that belongs to the current user, i.e. they have the
/// spending sercret key and can therefore use it as an input, "spending" the note. Extra
/// constraints need to be applied to input notes to ensure they are valid.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EscrowInputNote {
    /// The note to spend
    pub note: Note,
    /// Selects which spend path the circuit enforces. Mirrors the Noir
    /// `EscrowInputNote.spend_type` field:
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
    /// kind-8 refund path). The lock is serialized (anchor persists to JSON),
    /// but headers are not (they're filled in at refund time from real blocks).
    #[serde(default)]
    pub time_proof: TimeProof,
}

impl EscrowInputNote {
    /// Create a new input note for the standard Poseidon-key ownership path
    /// (`spend_type == 0`, kinds 1..=4 with an inactive timelock).
    #[must_use]
    pub fn new(note: Note, secret_key: Element) -> Self {
        Self {
            note,
            secret_key,
            ..Self::default()
        }
    }

    /// Input note for the signature32 spend path (`spend_type == 1`, note
    /// kind 5). The 32-byte preimage proves ownership; `secret_key` and
    /// `time_proof` are not consulted by the circuit on this path.
    #[must_use]
    pub fn new_signature32(note: Note, preimage: [u8; 32]) -> Self {
        Self {
            note,
            spend_type: 1,
            secret_key: Element::ZERO,
            preimage,
            time_proof: TimeProof::default(),
        }
    }

    /// Input note for the signature32sha spend path (`spend_type == 2`,
    /// note kind 6). The SHA-256 preimage proves ownership.
    #[must_use]
    pub fn new_signature32sha(note: Note, preimage: [u8; 32]) -> Self {
        Self {
            note,
            spend_type: 2,
            secret_key: Element::ZERO,
            preimage,
            time_proof: TimeProof::default(),
        }
    }

    /// Input note for the Poseidon-key path under an active timelock
    /// (`spend_type == 0`, kinds 1..=4 with a non-zero `lock.zero_block`).
    /// The PoW chain in `time_proof` must extend `lock.zero_block` by
    /// `lock.n_blocks` blocks; `note.psi` must equal
    /// `Poseidon(get_address_for_private_key(secret_key), lock.commitment())`.
    #[must_use]
    pub fn new_timelocked(note: Note, secret_key: Element, time_proof: TimeProof) -> Self {
        Self {
            note,
            spend_type: 0,
            secret_key,
            preimage: [0u8; 32],
            time_proof,
        }
    }

    /// Input note for the HTLC hash-claim branch (`spend_type == 3`,
    /// note kind 8, non-zero `preimage`). The SHA-256 preimage hashes to
    /// `note.address`.
    #[must_use]
    pub fn new_htlc_claim(note: Note, preimage: [u8; 32]) -> Self {
        Self {
            note,
            spend_type: 3,
            secret_key: Element::ZERO,
            preimage,
            time_proof: TimeProof::default(),
        }
    }

    /// Input note for the HTLC refund branch (`spend_type == 3`,
    /// note kind 8, zero `preimage`). The PoW chain in `time_proof` must
    /// extend `lock.zero_block`; `note.psi` must equal
    /// `Poseidon(get_address_for_private_key(secret_key), lock.commitment())`.
    #[must_use]
    pub fn new_htlc_refund(note: Note, secret_key: Element, time_proof: TimeProof) -> Self {
        Self {
            note,
            spend_type: 3,
            secret_key,
            preimage: [0u8; 32],
            time_proof,
        }
    }

    /// Create a new padding note
    #[must_use]
    pub fn padding_note() -> Self {
        Self {
            note: Note::padding_note(),
            spend_type: 0,
            secret_key: Element::ZERO,
            preimage: [0u8; 32],
            time_proof: TimeProof::default(),
        }
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
}