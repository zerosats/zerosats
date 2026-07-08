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
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zk_primitives::{Note, TimeLock, get_address_for_private_key};

/// A supplied preimage does not hash to the descriptor's committed payment
/// hash — i.e. the redeemer mistyped `--preimage`.
///
/// Distinct from a lost claim key so the caller can tell a
/// recoverable-in-seconds typo from genuinely stuck funds. The claim address
/// [`htlc_claim_address`] is a one-way hash, so without this check both cases
/// fail identically in the key search with `WalletError::NoKey`.
#[derive(Clone, Debug, Error)]
#[error(
    "preimage does not match the escrow payment hash \
     (got SHA-256(preimage)=0x{got}, expected 0x{expected}); check --preimage"
)]
pub struct PreimageMismatch {
    pub expected: String,
    pub got: String,
}

/// Secret-free HTLC note descriptor that can be sent to the redeemer.
///
/// The flat note fields are the committed escrow output. `timelock` records
/// the locker's refund anchor and window; `escrow-redeem` surfaces it so the
/// redeemer sees when the locker could reclaim the funds. The refund side
/// spends from its own separate witness file, not this descriptor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EscrowNoteDescriptor {
    #[serde(flatten)]
    pub note: Note,
    pub timelock: TimeLock,
    /// SHA-256 payment hash the claim branch is bound to, committed by the
    /// locker at lock time. Carried here so the redeemer can verify a supplied
    /// preimage *before* the key search (see [`Self::check_preimage`]).
    /// `Option` + `serde(default)` keeps descriptors written by older CLI
    /// versions loadable: they decode to `None` and preimage verification is
    /// skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_hash: Option<[u8; 32]>,
}

impl EscrowNoteDescriptor {
    /// Verify `SHA-256(preimage)` matches the committed [`Self::payment_hash`].
    ///
    /// Returns a distinct [`PreimageMismatch`] on a wrong preimage so the
    /// redeemer can distinguish a mistyped `--preimage` (recoverable in
    /// seconds) from a lost claim key (funds stuck until the refund timelock).
    /// Legacy descriptors that carry no payment hash pass unconditionally —
    /// there is nothing to check against.
    pub fn check_preimage(&self, preimage: [u8; 32]) -> Result<(), PreimageMismatch> {
        let Some(expected) = self.payment_hash else {
            return Ok(());
        };
        let got: [u8; 32] = Sha256::digest(preimage).into();
        if got == expected {
            Ok(())
        } else {
            Err(PreimageMismatch {
                expected: hex::encode(expected),
                got: hex::encode(got),
            })
        }
    }
}

/// `type` tag written into the `*-htlc-note.json` descriptor that
/// `escrow-redeem` consumes.
pub const REDEEM_DESCRIPTOR_TYPE: &str = "ciphera-htlc-redeem-descriptor";

/// `type` tag written into the `*-htlc-refund.json` witness that
/// `escrow-refund` consumes.
pub const REFUND_WITNESS_TYPE: &str = "ciphera-htlc-refund-witness";

/// Error reading a tagged escrow file (see [`from_tagged_json`]).
#[derive(Debug, Error)]
pub enum EscrowFileError {
    #[error("failed to parse escrow file as JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// The file carries a `type` tag, but for the *other* escrow command.
    /// This is the common two-party mistake (redeem given the refund file, or
    /// vice versa); name both files so the fix is obvious.
    #[error(
        "wrong escrow file: expected a {expected} but this file is a {found}; \
         escrow-redeem takes the *-htlc-note.json descriptor and escrow-refund \
         takes the *-htlc-refund.json witness"
    )]
    WrongType { expected: String, found: String },
}

/// Serialize `value` to pretty JSON with a top-level `"type": file_type` tag.
///
/// The tag lets [`from_tagged_json`] reject a file passed to the wrong escrow
/// command with a clear error instead of an opaque serde "missing field". The
/// tag is a sibling of the payload fields, so the payload type ([`Note`],
/// [`EscrowNoteDescriptor`], `EscrowInputNote`) needs no CLI-specific field.
pub fn to_tagged_json<T: Serialize>(
    file_type: &str,
    value: &T,
) -> Result<String, serde_json::Error> {
    let mut json = serde_json::to_value(value)?;
    if let serde_json::Value::Object(map) = &mut json {
        map.insert(
            "type".to_string(),
            serde_json::Value::String(file_type.to_string()),
        );
    }
    serde_json::to_string_pretty(&json)
}

/// Deserialize a tagged escrow file, requiring its `"type"` tag to equal
/// `expected_type`.
///
/// A tag for a different command yields a clear [`EscrowFileError::WrongType`].
/// A file with *no* tag (written by a CLI predating tagging) is accepted and
/// deserialized as before — lenient for backward compatibility, since an
/// untagged file cannot be misattributed to the wrong command.
pub fn from_tagged_json<T: DeserializeOwned>(
    json: &str,
    expected_type: &str,
) -> Result<T, EscrowFileError> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    if let Some(found) = value.get("type").and_then(serde_json::Value::as_str) {
        if found != expected_type {
            return Err(EscrowFileError::WrongType {
                expected: expected_type.to_string(),
                found: found.to_string(),
            });
        }
    }
    Ok(serde_json::from_value(value)?)
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

    fn descriptor_with_payment_hash(payment_hash: Option<[u8; 32]>) -> EscrowNoteDescriptor {
        EscrowNoteDescriptor {
            note: Note {
                utxo_kind: Element::new(2),
                note_kind: Element::new(2),
                address: Element::new(1),
                psi: Element::ZERO,
                value: Element::new(1),
            },
            timelock: TimeLock {
                zero_block: [0u8; 32],
                n_blocks: Element::new(2),
            },
            payment_hash,
        }
    }

    #[test]
    fn check_preimage_accepts_correct_preimage() {
        let preimage = [7u8; 32];
        let payment_hash: [u8; 32] = Sha256::digest(preimage).into();
        let descriptor = descriptor_with_payment_hash(Some(payment_hash));

        assert!(descriptor.check_preimage(preimage).is_ok());
    }

    #[test]
    fn check_preimage_rejects_wrong_preimage() {
        let preimage = [7u8; 32];
        let payment_hash: [u8; 32] = Sha256::digest(preimage).into();
        let descriptor = descriptor_with_payment_hash(Some(payment_hash));

        // A mistyped preimage must surface a distinct, actionable error rather
        // than the generic NoKey the key search would produce.
        let err = descriptor
            .check_preimage([9u8; 32])
            .expect_err("a preimage that hashes differently must be rejected");
        assert!(
            err.to_string().contains("preimage"),
            "error should name the preimage; got: {err}"
        );
    }

    #[test]
    fn check_preimage_skips_legacy_descriptor_without_hash() {
        // Descriptors written by older CLI versions decode payment_hash as
        // None; there is nothing to verify against, so any preimage passes.
        let descriptor = descriptor_with_payment_hash(None);
        assert!(descriptor.check_preimage([9u8; 32]).is_ok());
    }

    #[test]
    fn tagged_descriptor_roundtrips_and_rejects_wrong_command() {
        let descriptor = descriptor_with_payment_hash(Some([3u8; 32]));

        let json = to_tagged_json(REDEEM_DESCRIPTOR_TYPE, &descriptor).unwrap();
        assert!(
            json.contains(REDEEM_DESCRIPTOR_TYPE),
            "serialized file must carry its type tag"
        );

        // Reading it back as the matching command succeeds and preserves data.
        let back: EscrowNoteDescriptor = from_tagged_json(&json, REDEEM_DESCRIPTOR_TYPE).unwrap();
        assert_eq!(back.payment_hash, descriptor.payment_hash);

        // Passing the redeem descriptor to escrow-refund yields a clear,
        // command-naming error rather than an opaque serde "missing field".
        let err = from_tagged_json::<EscrowNoteDescriptor>(&json, REFUND_WITNESS_TYPE)
            .expect_err("a redeem descriptor must be rejected when a refund witness is expected");
        assert!(matches!(err, EscrowFileError::WrongType { .. }));
        assert!(err.to_string().contains(REFUND_WITNESS_TYPE));
    }

    #[test]
    fn untagged_file_is_accepted_for_backward_compat() {
        // A descriptor written by a CLI predating tagging has no "type" key; it
        // must still deserialize, since an untagged file cannot be misattributed
        // to the wrong command.
        let descriptor = descriptor_with_payment_hash(Some([3u8; 32]));
        let untagged = serde_json::to_string(&descriptor).unwrap();
        assert!(!untagged.contains("\"type\""));

        let back: EscrowNoteDescriptor = from_tagged_json(&untagged, REDEEM_DESCRIPTOR_TYPE).unwrap();
        assert_eq!(back.payment_hash, descriptor.payment_hash);
    }

    #[test]
    fn descriptor_without_payment_hash_field_decodes_as_none() {
        // Backward compat: JSON produced by a pre-payment_hash CLI has no
        // "payment_hash" key. It must still deserialize, with payment_hash =
        // None, and then skip preimage verification.
        let descriptor = descriptor_with_payment_hash(Some([1u8; 32]));
        let mut value = serde_json::to_value(&descriptor).unwrap();
        value
            .as_object_mut()
            .expect("descriptor serializes to a JSON object")
            .remove("payment_hash");

        let decoded: EscrowNoteDescriptor = serde_json::from_value(value).unwrap();
        assert!(decoded.payment_hash.is_none());
        assert!(decoded.check_preimage([9u8; 32]).is_ok());
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
