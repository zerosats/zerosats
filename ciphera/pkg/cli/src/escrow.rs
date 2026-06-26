//! HTLC escrow flow for the CLI.
//!
//! The Noir `escrow` leaf circuit accepts an `EscrowInputNote` with
//! `spend_type == 3` for the HTLC branch (see
//! `noir/common/src/lib.nr::check_spend_conditions`). Two paths:
//!
//! * **Claim** (`preimage != [0; 32]`): the redeemer must know the
//!   32-byte preimage *and* hold the secret key whose `key_hash` was
//!   embedded into the note's address. The circuit checks
//!   `note.address == Poseidon([key_hash, low, high])` where
//!   `(low, high)` is the field-pair of `SHA256(preimage)`.
//! * **Refund** (`preimage == [0; 32]`): the locker presents a PoW
//!   chain extending `lock.zero_block` by at least `lock.n_blocks`,
//!   together with the secret key whose `key_hash` was embedded into
//!   `note.psi == Poseidon([key_hash, lock.commitment()])`.
//!
//! `EscrowInputNote` already carries `spend_type`, `secret_key`,
//! `preimage` and `time_proof`, so it can be serialised as-is to JSON
//! and shipped between the lock / redeem / refund CLI invocations.
//!
//! Address derivations mirror the test path in
//! `pkg/barretenberg/src/circuits/tests.rs::signature32sha_address`,
//! which is the canonical Rust reference the live circuit was last
//! validated against.

use element::Element;
use hash::hash_merge;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zk_primitives::{Note, TimeLock, get_address_for_private_key};

/// Secret-free HTLC note descriptor that can be sent to the redeemer.
///
/// The flat note fields are the committed escrow output. `timelock`
/// preserves the refund anchor so either side can reconstruct the HTLC
/// spend data later without carrying the refund secret or the preimage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EscrowNoteDescriptor {
    #[serde(flatten)]
    pub note: Note,
    pub timelock: TimeLock,
}

/// HTLC `note.address` for the claim branch -- binds the redeemer's
/// secret key into the SHA-256 commitment of the preimage.
#[must_use]
pub fn htlc_claim_address(redeemer_secret_key: Element, preimage: [u8; 32]) -> Element {
    let payment_hash: [u8; 32] = Sha256::digest(preimage).into();
    let redeemer_address = get_address_for_private_key(redeemer_secret_key);
    htlc_claim_address_from_hash(redeemer_address, payment_hash)
}

/// HTLC `note.address` for the claim branch when only the payment hash
/// is known. This is the two-party lock-time form: the locker binds the
/// claim branch to the redeemer's Ciphera address without learning the
/// preimage.
#[must_use]
pub fn htlc_claim_address_from_hash(redeemer_address: Element, payment_hash: [u8; 32]) -> Element {
    let elem = Element::from_be_bytes(payment_hash);
    let (high, low) = elem.decompose_be();
    hash_merge([redeemer_address, high, low])
}

/// HTLC `note.psi` for the refund branch -- binds the locker's secret
/// key into the timelock commitment. The locker spends with
/// `spend_type == 3`, `preimage == [0; 32]`, a fresh
/// [`TimeProof`] extending `lock.zero_block`.
#[must_use]
pub fn htlc_refund_psi(locker_secret_key: Element, lock: &TimeLock) -> Element {
    let key_hash = get_address_for_private_key(locker_secret_key);
    hash_merge([key_hash, lock.commitment()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_address_from_hash_matches_preimage_form() {
        let redeemer_secret_key = Element::from(123u64);
        let redeemer_address = get_address_for_private_key(redeemer_secret_key);
        let preimage = [7u8; 32];
        let payment_hash: [u8; 32] = Sha256::digest(preimage).into();

        assert_eq!(
            htlc_claim_address_from_hash(redeemer_address, payment_hash),
            htlc_claim_address(redeemer_secret_key, preimage)
        );
    }
}

// ---------------------------------------------------------------------------
// Test-only PoW fixtures.
//
// Production builds the timelock anchor and refund PoW witness from *real*
// Bitcoin blocks via `crate::mempool` (mempool.space). These fixed headers
// (block 946920 + 946921/946922) only exist so unit tests can exercise the
// lock/refund round-trip without a Bitcoin client -- using them in
// production makes the timelock a no-op (the two extending blocks already
// exist), so they are gated behind `#[cfg(test)]`. Mirrors
// `pow_two_block_proof()` in `pkg/barretenberg/src/circuits/tests.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
use zk_primitives::TimeProof;

#[cfg(test)]
#[allow(dead_code)]
#[must_use]
pub fn pow_two_block_proof() -> TimeProof {
    TimeProof {
        lock: pow_two_block_lock(),
        headers: [header_946921(), header_946922()],
    }
}

/// Matching [`TimeLock`] for [`pow_two_block_proof`].
#[cfg(test)]
#[allow(dead_code)]
#[must_use]
pub fn pow_two_block_lock() -> TimeLock {
    TimeLock {
        zero_block: anchor_zero_block(),
        n_blocks: Element::new(2),
    }
}

#[cfg(test)]
fn anchor_zero_block() -> [u8; 32] {
    [
        0xf8, 0xa1, 0x7c, 0xed, 0x1d, 0xac, 0x17, 0xba, 0x27, 0xba, 0x9d, 0xee, 0x7f, 0x63, 0x95,
        0x9b, 0xa7, 0x54, 0x18, 0xb6, 0x7c, 0xe7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ]
}

#[cfg(test)]
fn header_946921() -> [u8; 80] {
    [
        0x00, 0x40, 0x0b, 0x20, 0xf8, 0xa1, 0x7c, 0xed, 0x1d, 0xac, 0x17, 0xba, 0x27, 0xba, 0x9d,
        0xee, 0x7f, 0x63, 0x95, 0x9b, 0xa7, 0x54, 0x18, 0xb6, 0x7c, 0xe7, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xee, 0xa2, 0x39, 0xdc, 0xe3, 0x77, 0x3c, 0x5f, 0x61,
        0x79, 0xd2, 0xd1, 0x49, 0xb2, 0x5f, 0x1b, 0x17, 0xf6, 0x49, 0x33, 0x86, 0x95, 0x5c, 0xf5,
        0x3f, 0xc7, 0x04, 0x5a, 0x39, 0xb8, 0xc6, 0x00, 0x0c, 0xc8, 0xef, 0x69, 0x69, 0x13, 0x02,
        0x17, 0xe3, 0x10, 0xa9, 0x35,
    ]
}

#[cfg(test)]
fn header_946922() -> [u8; 80] {
    [
        0x00, 0x00, 0x07, 0x20, 0xcf, 0x51, 0x90, 0x4c, 0xcc, 0x0c, 0xf4, 0x7b, 0x6a, 0xab, 0xf0,
        0xcc, 0xfe, 0x55, 0x5c, 0x19, 0x77, 0x7c, 0xf6, 0x62, 0x06, 0x01, 0x02, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x74, 0x3f, 0xc7, 0xf1, 0xaf, 0xc9, 0x8f, 0x0e, 0x2f,
        0x4e, 0x20, 0xc4, 0x0c, 0xb2, 0x11, 0x35, 0x07, 0x8a, 0x30, 0x5c, 0x01, 0xb9, 0x05, 0xe7,
        0xc5, 0x26, 0xac, 0x10, 0xb7, 0xb4, 0x25, 0xc9, 0xf2, 0xc9, 0xef, 0x69, 0x69, 0x13, 0x02,
        0x17, 0x65, 0xdb, 0x8d, 0x21,
    ]
}
