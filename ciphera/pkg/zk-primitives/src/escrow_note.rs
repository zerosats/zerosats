use crate::{
    Note, Signature32, Signature32Sha, TimeLock, get_address_for_private_key, timelock_address,
};
use element::Element;

/// Constructors for the non-Poseidon-key spend paths that the `escrow`
/// leaf circuit exposes. Each method builds a [`Note`] whose `address`
/// (and where relevant `psi`) is bound to the witness the corresponding
/// Noir branch expects -- a 32-byte preimage, a SHA-256 preimage, a
/// timelocked refund commitment, or an HTLC hashlock + refund commitment.
/// The base UTXO `kind` (`Element::new(2)`) is shared across all
/// constructors; the per-path selector lives in `note_kind`.
pub trait EscrowSignatures {
    /// Build a kind-5 (signature32) note bound to a 32-byte preimage:
    /// `address = Poseidon(field_pair(preimage))`.
    fn new_signature32(
        preimage: [u8; 32],
        note_kind: Element,
        value: Element,
        psi: Element,
    ) -> Self;

    /// Build a kind-6 (signature32sha) note bound to a SHA-256 preimage:
    /// `address = Poseidon(field_pair(SHA256(preimage)))`.
    fn new_signature32sha(
        preimage: [u8; 32],
        note_kind: Element,
        value: Element,
        psi: Element,
    ) -> Self;

    /// Build a timelock-gated note (Noir `spend_type == 0` with active
    /// `TimeLock`). `address` carries the owner key hash (inactive-fallback
    /// branch); `psi` binds the active-timelock branch to
    /// `Poseidon(key_hash, lock.commitment())`.
    fn new_timelock(
        secret_key: Element,
        lock: &TimeLock,
        note_kind: Element,
        value: Element,
    ) -> Self;

    /// Build a kind-8 (HTLC) note for a holder who already knows the
    /// preimage. `address` carries the SHA-256 hashlock (claim branch);
    /// `psi` carries the timelocked refund commitment.
    fn new_htlc(
        secret_key: Element,
        lock: &TimeLock,
        preimage: [u8; 32],
        note_kind: Element,
        value: Element,
    ) -> Self;

    /// Build a kind-8 (HTLC) note from a payment hash + recipient address,
    /// for the offramp-gateway flow where the spender does not yet hold
    /// the preimage. Equivalent to [`EscrowNote::new_htlc`] for any party
    /// that later learns the preimage.
    fn new_htlc_for_recipient(
        recipient_address: Element,
        lock: &TimeLock,
        payment_hash: [u8; 32],
        note_kind: Element,
        value: Element,
    ) -> Self;
}
impl EscrowSignatures for Note {

    /// Note for the utxo_kind-5 (signature32) spend path. The address is bound
    /// to a 32-byte preimage: `Poseidon(field_pair(preimage))`.
    fn new_signature32(
        preimage: [u8; 32],
        note_kind: Element,
        value: Element,
        psi: Element,
    ) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind,
            address: Signature32::new(preimage, Element::ZERO).address(),
            psi,
            value,
        }
    }

    /// Note for the utxo_kind-6 (signature32sha) spend path. The address is bound
    /// to a SHA-256 preimage: `Poseidon(field_pair(SHA256(preimage)))`.
    fn new_signature32sha(
        preimage: [u8; 32],
        note_kind: Element,
        value: Element,
        psi: Element,
    ) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind,
            address: Signature32Sha::new(preimage, Element::ZERO).address(),
            psi,
            value,
        }
    }

    /// Note for the timelock spend path (Noir `spend_type == 0` with an
    /// active TimeLock). `address` carries the plain owner key hash --
    /// used by the inactive-fallback branch. `psi` is derived from
    /// `(key_hash, lock.commitment())` and is the field actually
    /// asserted by the active-timelock branch in the circuit.
    fn new_timelock(
        secret_key: Element,
        lock: &TimeLock,
        note_kind: Element,
        value: Element,
    ) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind,
            address: get_address_for_private_key(secret_key),
            psi: timelock_address(secret_key, lock),
            value,
        }
    }

    /// Note for the utxo_kind-8 (HTLC) spend path. The address is the timelock
    /// refund commitment and `psi` is the SHA-256 hashlock — matching the
    /// circuit's two-branch check (`note.address` for refund, `note.psi`
    /// for the hash path).
    fn new_htlc(
        secret_key: Element,
        lock: &TimeLock,
        preimage: [u8; 32],
        note_kind: Element,
        value: Element,
    ) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind,
            address: {
                let payment_hash = Signature32Sha::new(preimage, Element::ZERO).sha_hash();
                let element = Element::from_be_bytes(payment_hash);
                let (high, low) = element.decompose_be();
                let key_hash = get_address_for_private_key(secret_key);
                hash::hash_merge([key_hash, high, low])
            },
            psi: timelock_address(secret_key, lock),
            value,
        }
    }

    /// HTLC note (kind 8) for the offramp-gateway flow where the spender
    /// (the gateway) does not yet know the preimage when the note is built:
    /// the address is derived directly from a Lightning `payment_hash`,
    /// and the refund branch is bound to `recipient_address` -- the user's
    /// `get_address_for_private_key(sk)`. The gateway learns the preimage
    /// by paying the invoice; the user keeps the refund path if the gateway
    /// fails to claim before the timelock window opens.
    fn new_htlc_for_recipient(
        recipient_address: Element,
        lock: &TimeLock,
        payment_hash: [u8; 32],
        note_kind: Element,
        value: Element,
    ) -> Self {
        Self {
            utxo_kind: Element::new(2),
            note_kind,
            address: {
                let element = Element::from_be_bytes(payment_hash);
                let (high, low) = element.decompose_be();
                hash::hash_merge([recipient_address, high, low])
            },
            psi: hash::hash_merge([recipient_address, lock.commitment()]),
            value,
        }
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
        // utxo_kind is the fixed 0x2 prefix used by get_note_commitment;
        // the per-note spend-path selector lives in note_kind (here 99).
        assert_eq!(note.utxo_kind, Element::new(2));
        assert_eq!(note.note_kind, Element::new(99));
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
    fn timelock_note_binds_commitment_to_psi() {
        // Noir's active-timelock branch (spend_type == 0) asserts
        // `psi == Poseidon(key_hash, lock.commitment())`, not `address`.
        // The inactive-fallback branch asserts `address == key_hash`.
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
        );

        let key_hash = get_address_for_private_key(secret_key);
        let expected_psi = hash::hash_merge([key_hash, lock.commitment()]);

        assert_eq!(note.address, key_hash);
        assert_eq!(note.psi, expected_psi);
    }

    #[test]
    fn htlc_combines_timelock_refund_and_sha_hashlock() {
        // HTLC (spend_type == 3) checks:
        //   - hash branch (nonzero preimage): note.address must equal
        //     Poseidon(field_pair(SHA256(preimage)));
        //   - refund branch (zero preimage):  note.psi must equal
        //     Poseidon(key_hash, lock.commitment()).
        // So address carries the hashlock and psi carries the refund
        // commitment -- opposite to the pre-fix layout.
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

        let key_hash = get_address_for_private_key(secret_key);
        let expected_refund = hash::hash_merge([key_hash, lock.commitment()]);

        let element = Element::from_be_bytes(preimage_sha());
        let (high, low) = element.decompose_be();
        let expected_hashlock = hash::hash_merge([key_hash, high, low]);

        assert_eq!(note.address, expected_hashlock);
        assert_eq!(note.psi, expected_refund);
    }

    #[test]
    fn new_htlc_for_recipient_matches_new_htlc() {
        // The offramp gateway constructs the HTLC note from the user's
        // address (derived from their sk) and the bolt11 payment_hash
        // (SHA-256 of the preimage). The resulting note must be identical
        // to `new_htlc` built by someone who happens to know the preimage.
        let secret_key = Element::new(101);
        let lock = TimeLock {
            zero_block: [7u8; 32],
            n_blocks: Element::new(2),
        };
        let note_kind = Element::new(8);
        let value = Element::new(50);

        let preimage_bytes = preimage();
        let payment_hash = preimage_sha();
        let user_address = get_address_for_private_key(secret_key);

        let from_preimage = Note::new_htlc(secret_key, &lock, preimage_bytes, note_kind, value);
        let from_hash = Note::new_htlc_for_recipient(
            user_address,
            &lock,
            payment_hash,
            note_kind,
            value,
        );

        assert_eq!(from_preimage.address, from_hash.address);
        assert_eq!(from_preimage.psi, from_hash.psi);
        assert_eq!(from_preimage.value, from_hash.value);
        assert_eq!(from_preimage.note_kind, from_hash.note_kind);
        assert_eq!(from_preimage.utxo_kind, from_hash.utxo_kind);
    }

    #[test]
    fn padding_check_gates_on_note_kind() {
        // note_kind=0 → commitment is zero regardless of value (matches Noir utxo_kind==0 branch).
        let zero_utxo_kind = Note {
            utxo_kind: Element::new(2),
            note_kind: Element::ZERO,
            address: Element::new(123),
            psi: Element::new(456),
            value: Element::new(10),
        };
        assert_eq!(zero_utxo_kind.commitment(), Element::ZERO);

        // note_kind!=0 with value=0 → still commits to a non-zero hash, matching
        // Noir which only zero-commits when note.utxo_kind == 0.
        let zero_value = Note {
            utxo_kind: Element::new(2),
            note_kind: Element::new(5),
            address: Element::ZERO,
            psi: Element::ZERO,
            value: Element::ZERO,
        };
        assert_ne!(zero_value.commitment(), Element::ZERO);
    }
}