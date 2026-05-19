use crate::{
    Signature32, Signature32Sha, TimeLock, bridged_polygon_usdc_note_kind,
    get_address_for_private_key, hash_private_key_for_psi, timelock_address,
};
use element::Element;
use noirc_abi::input_parser::InputValue;
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(feature = "ts-rs")]
use ts_rs::TS;

/// A note is used in zk circuits to represent some kind of token (e.g. USDC) on
/// the Ciphera Network.
///
/// This is used to create notes in the zk-rollup
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The kind of note
    pub kind: Element,
    /// The contract of note
    pub contract: Element,
    /// The address of the note
    pub address: Element,
    /// The psi adds additional entropy to the note, to ensure uniqueness
    pub psi: Element,
    /// The value of the note (dependent on the token)
    pub value: Element,
}

impl Note {
    /// Create a new note
    #[must_use]
    pub fn new(address: Element, value: Element) -> Self {
        Self {
            kind: Element::new(2),
            contract: bridged_polygon_usdc_note_kind(),
            address,
            psi: Element::secure_random(thread_rng()),
            value,
        }
    }

    /// Create a new note with custom PSI
    #[must_use]
    pub fn new_with_psi(address: Element, value: Element, psi: Element) -> Self {
        Self {
            kind: Element::new(2),
            contract: bridged_polygon_usdc_note_kind(),
            address,
            psi,
            value,
        }
    }

    /// New note from ephemeral private key (only use private key once)
    #[must_use]
    pub fn new_from_ephemeral_private_key(private_key: Element, value: Element) -> Self {
        let address = get_address_for_private_key(private_key);
        let psi = hash_private_key_for_psi(private_key);
        Self {
            kind: Element::new(2),
            contract: bridged_polygon_usdc_note_kind(),
            address,
            psi,
            value,
        }
    }

    /// Note for the kind-5 (signature32) spend path. The address is bound
    /// to a 32-byte preimage: `Poseidon(field_pair(preimage))`.
    #[must_use]
    pub fn new_signature32(
        preimage: [u8; 32],
        contract: Element,
        value: Element,
        psi: Element,
    ) -> Self {
        Self {
            kind: Element::new(5),
            contract,
            address: Signature32::new(preimage, Element::ZERO).address(),
            psi,
            value,
        }
    }

    /// Note for the kind-6 (signature32sha) spend path. The address is bound
    /// to a SHA-256 preimage: `Poseidon(field_pair(SHA256(preimage)))`.
    #[must_use]
    pub fn new_signature32sha(
        preimage: [u8; 32],
        contract: Element,
        value: Element,
        psi: Element,
    ) -> Self {
        Self {
            kind: Element::new(6),
            contract,
            address: Signature32Sha::new(preimage, Element::ZERO).address(),
            psi,
            value,
        }
    }

    /// Note for the kind-7 (timelock) spend path. The address binds the
    /// owner's secret key to a PoW timelock:
    /// `Poseidon(get_secret_hash(secret_key), TimeLock::commitment)`.
    #[must_use]
    pub fn new_timelock(
        secret_key: Element,
        lock: &TimeLock,
        contract: Element,
        value: Element,
        psi: Element,
    ) -> Self {
        Self {
            kind: Element::new(7),
            contract,
            address: timelock_address(secret_key, lock),
            psi,
            value,
        }
    }

    /// Note for the kind-8 (HTLC) spend path. The address is the timelock
    /// refund commitment and `psi` is the SHA-256 hashlock — matching the
    /// circuit's two-branch check (`note.address` for refund, `note.psi`
    /// for the hash path).
    #[must_use]
    pub fn new_htlc(
        secret_key: Element,
        lock: &TimeLock,
        preimage: [u8; 32],
        contract: Element,
        value: Element,
    ) -> Self {
        Self {
            kind: Element::new(8),
            contract,
            address: timelock_address(secret_key, lock),
            psi: Signature32Sha::new(preimage, Element::ZERO).address(),
            value,
        }
    }

    /// Deterministic padding note, because circuits have a fixed note input size,
    /// and so we pad extra notes with zeros
    #[must_use]
    pub fn padding_note() -> Self {
        Note {
            kind: Element::new(2),
            contract: Element::ZERO,
            address: Element::ZERO,
            psi: Element::ZERO,
            value: Element::ZERO,
        }
    }

    /// Check if the note is a padding note
    #[must_use]
    pub fn is_padding_note(&self) -> bool {
        self.contract == Element::ZERO && self.value == Element::ZERO
    }

    /// Commitment of the note, this is stored in the merkle tree and proves the note exists.
    ///
    /// Matches the Noir `get_note_commitment` helper: padding (Noir `note.kind == 0`,
    /// which maps to Rust `self.contract == 0`) commits to zero, every other note
    /// commits to `Poseidon([2, contract, value, address, psi, 0, 0], 7)`.
    // TODO: should we leave some space in here?
    #[must_use]
    pub fn commitment(&self) -> Element {
        if self.contract == Element::ZERO {
            Element::ZERO
        } else {
            hash::hash_merge([
                self.kind,
                self.contract,
                self.value,
                self.address,
                self.psi,
                Element::ZERO,
                Element::ZERO,
            ])
        }
    }
}

impl Default for Note {
    fn default() -> Self {
        Self::padding_note()
    }
}

impl From<&Note> for InputValue {
    fn from(note: &Note) -> Self {
        let mut struct_ = BTreeMap::new();

        struct_.insert(
            "address".to_owned(),
            InputValue::Field(note.address.to_base()),
        );
        struct_.insert(
            "kind".to_owned(),
            InputValue::Field(note.contract.to_base()),
        );
        struct_.insert("psi".to_owned(), InputValue::Field(note.psi.to_base()));
        struct_.insert("value".to_owned(), InputValue::Field(note.value.to_base()));

        InputValue::Struct(struct_)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimeLock;

    fn preimage() -> [u8; 32] {
        [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ]
    }

    // Mirrors `shared_preimage_sha()` in noir/utxo/src/main.nr.
    fn preimage_sha() -> [u8; 32] {
        [
            174, 33, 108, 46, 245, 36, 122, 55, 130, 193, 53, 239, 162, 121, 163, 228, 205, 198,
            16, 148, 39, 15, 93, 43, 229, 140, 98, 4, 183, 166, 18, 201,
        ]
    }

    fn raw_field_pair_address(bytes: [u8; 32]) -> Element {
        let element = Element::from_be_bytes(bytes);
        let (high, low) = element.decompose_be();
        hash::hash_merge([high, low])
    }

    #[test]
    fn signature32_address_matches_field_pair() {
        let note = Note::new_signature32(preimage(), Element::new(99), Element::new(50), Element::ZERO);
        assert_eq!(note.address, raw_field_pair_address(preimage()));
        assert_eq!(note.kind, Element::new(5));
        assert_eq!(note.contract, Element::new(99));
    }

    #[test]
    fn signature32sha_address_matches_sha_field_pair() {
        let note = Note::new_signature32sha(
            preimage(),
            Element::new(6),
            Element::new(40),
            Element::ZERO,
        );
        // SHA-256(preimage) is the fixture in noir/signature32sha::test_main.
        assert_eq!(note.address, raw_field_pair_address(preimage_sha()));
    }

    #[test]
    fn timelock_address_matches_key_plus_lock() {
        let secret_key = Element::new(101);
        let lock = TimeLock {
            zero_block: [7u8; 32],
            n_blocks: Element::new(2),
        };
        let note = Note::new_timelock(
            secret_key,
            &lock,
            Element::new(7),
            Element::new(50),
            Element::ZERO,
        );

        let expected_address = hash::hash_merge([
            get_address_for_private_key(secret_key),
            lock.commitment(),
        ]);
        assert_eq!(note.address, expected_address);
    }

    #[test]
    fn htlc_combines_timelock_refund_and_sha_hashlock() {
        let secret_key = Element::new(101);
        let lock = TimeLock {
            zero_block: [7u8; 32],
            n_blocks: Element::new(2),
        };
        let note = Note::new_htlc(
            secret_key,
            &lock,
            preimage(),
            Element::new(8),
            Element::new(40),
        );

        // Refund branch: address = timelock_address(sk, lock).
        let expected_address = hash::hash_merge([
            get_address_for_private_key(secret_key),
            lock.commitment(),
        ]);
        assert_eq!(note.address, expected_address);
        // Hash branch: psi = Poseidon(field_pair(SHA256(preimage))).
        assert_eq!(note.psi, raw_field_pair_address(preimage_sha()));
    }

    #[test]
    fn padding_check_gates_on_contract() {
        // contract=0 → commitment is zero regardless of value (matches Noir kind==0 branch).
        let zero_kind = Note {
            kind: Element::new(2),
            contract: Element::ZERO,
            address: Element::new(123),
            psi: Element::new(456),
            value: Element::new(10),
        };
        assert_eq!(zero_kind.commitment(), Element::ZERO);

        // contract!=0 with value=0 → still commits to a non-zero hash, matching
        // Noir which only zero-commits when note.kind == 0.
        let zero_value = Note {
            kind: Element::new(2),
            contract: Element::new(5),
            address: Element::ZERO,
            psi: Element::ZERO,
            value: Element::ZERO,
        };
        assert_ne!(zero_value.commitment(), Element::ZERO);
    }
}
