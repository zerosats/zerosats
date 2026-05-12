use crate::{NoteURLPayload, decode_activity_url_payload, note::Note};
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
