//! Settlement-side prover.
//!
//! Updated for the `Escrow` circuit split: the offramp HTLC's claim branch
//! lives in `noir/escrow` (`spend_type == 3`), not in `noir/utxo`.
//! `LocalEscrowClaimProver::build_and_prove` therefore returns an
//! [`EscrowProof`]; the proof spends the on-chain HTLC note and outputs a
//! regular service-owned note. Burning that note to the service's EVM
//! address is a separate Utxo proof (out of scope here) until the node's
//! `/v0/transaction` endpoint learns to accept escrow proofs and the
//! prover learns to bundle `Escrow + Utxo` into one `AggAgg` slot pair.
//!
//! Address binding (see `noir/common/src/lib.nr::check_spend_conditions`,
//! `spend_type == 3`):
//!   * claim branch: `note.address == Poseidon([service_key_hash, low, high])`
//!     where `(low, high) = field_pair(SHA256(preimage))` and
//!     `service_key_hash = Poseidon([service_secret_key, 0])`.
//!   * refund branch: `note.psi == Poseidon([user_key_hash, lock.commitment()])`
//!     where `user_key_hash = Poseidon([user_secret_key, 0])`.
//!
//! The user constructs the HTLC by binding the claim side to the
//! service's published public key (`get_address_for_private_key(service_secret_key)`)
//! and the refund side to their own. The service then redeems by
//! presenting `(preimage, service_secret_key)`.

use crate::domain::Quote;
use async_trait::async_trait;
use barretenberg::Prove;
use element::Element;
use hash::hash_merge;
use sha2::{Digest, Sha256};
use zk_primitives::{
    Escrow, EscrowInputNote, EscrowProof, Note, TimeLock, TimeProof, UtxoKind,
    get_address_for_private_key,
};

/// Builds and proves the HTLC claim that redeems a quote's escrow note.
///
/// In production the prover talks to `barretenberg` (`bb_rs` backend); in
/// tests the trait can be mocked with a stub returning a canned proof.
#[async_trait]
pub trait EscrowClaimProver: Send + Sync {
    /// `service_secret_key` is the offramp service's own secret whose
    /// hash was baked into the HTLC note's address at quote time. The
    /// returned proof spends the HTLC note via the
    /// `EscrowInputNote` HTLC hash-claim branch and routes the value
    /// into a single service-owned output note that downstream code
    /// can later burn / forward.
    async fn build_and_prove(
        &self,
        quote: &Quote,
        preimage: [u8; 32],
        service_secret_key: Element,
    ) -> eyre::Result<EscrowProof>;
}

#[derive(Default)]
pub struct LocalEscrowClaimProver;

#[async_trait]
impl EscrowClaimProver for LocalEscrowClaimProver {
    async fn build_and_prove(
        &self,
        quote: &Quote,
        preimage: [u8; 32],
        service_secret_key: Element,
    ) -> eyre::Result<EscrowProof> {
        let lock = TimeLock {
            zero_block: quote.zero_block,
            n_blocks: quote.n_blocks,
        };

        // Reconstruct the HTLC escrow note exactly as it lives in the
        // rollup tree. The user (locker) bound the claim branch to the
        // service's pubkey and the refund branch to their own.
        let escrow_note = htlc_note_for_service_claim(
            service_secret_key,
            quote.user_address,
            &lock,
            quote.payment_hash,
            quote.note_kind,
            quote.amount,
        );

        // Sanity check: the noir circuit will reject mismatched commitments.
        if escrow_note.commitment() != quote.note_commitment {
            eyre::bail!(
                "HTLC commitment mismatch: reconstructed {} != on-chain {}. \
                 Check `service_secret_key` and `quote.user_address` -- one or both \
                 disagrees with what was actually used at lock time.",
                escrow_note.commitment(),
                quote.note_commitment,
            );
        }

        // Claim branch: spend_type=3, preimage non-zero, time_proof
        // unused (the circuit's hash branch ignores it). Build the
        // `EscrowInputNote` explicitly rather than via the
        // `new_htlc_claim` helper because the helper assumes
        // `secret_key = 0`, which would only be correct if the HTLC
        // were anyone-can-claim. The offramp's HTLC binds the address
        // to a real service key.
        let input = EscrowInputNote {
            note: escrow_note,
            spend_type: 3,
            secret_key: service_secret_key,
            preimage,
            time_proof: TimeProof::default(),
        };

        // Output: a single service-owned note carrying the claimed
        // value, padded with `Note::padding_note()` in slot 1. Because
        // the escrow circuit's `is_multiple_kinds` guard requires
        // consistent `note_kind` across inputs/outputs, the output
        // mirrors `quote.note_kind`.
        let service_note = service_owned_note(service_secret_key, quote.note_kind, quote.amount);

        let escrow = Escrow {
            kind: UtxoKind::Send,
            input_notes: [input, EscrowInputNote::padding_note()],
            output_notes: [service_note, Note::padding_note()],
            burn_address: None,
        };

        // Heavy proving call. Runs synchronously on a blocking thread so
        // the async settlement worker does not block. The raw barretenberg
        // error is `Box<dyn StdError>` which is !Send, so stringify it
        // before crossing the thread boundary.
        let proof = tokio::task::spawn_blocking(move || {
            escrow.prove().map_err(|e| format!("{e:?}"))
        })
        .await
        .map_err(|e| eyre::eyre!("prove task panicked: {e}"))?
        .map_err(|e| eyre::eyre!("escrow.prove failed: {e}"))?;

        Ok(proof)
    }
}

/// Reconstruct the offramp HTLC note. Address is bound to the service's
/// secret key (so only the service can claim with the preimage), psi is
/// bound to the user's address + timelock (so only they can refund
/// after PoW expiry). Mirrors what the user-side wallet produced at
/// lock time.
///
/// Exposed `pub` so the HTTP handler can hash an HTLC commitment for
/// quote storage using the exact same binding the prover will later
/// witness against.
pub fn htlc_note_for_service_claim(
    service_secret_key: Element,
    user_address: Element,
    lock: &TimeLock,
    payment_hash: [u8; 32],
    note_kind: Element,
    amount: Element,
) -> Note {
    let service_key_hash = get_address_for_private_key(service_secret_key);
    let elem = Element::from_be_bytes(payment_hash);
    let (high, low) = elem.decompose_be();
    let address = hash_merge([service_key_hash, high, low]);
    let psi = hash_merge([user_address, lock.commitment()]);

    Note {
        utxo_kind: Element::new(2),
        note_kind,
        address,
        psi,
        value: amount,
    }
}

/// Derive the SHA-256 hash bound into an HTLC address. Exposed for use
/// at quote-construction time (we hash the preimage that came from the
/// bolt11 invoice the user is paying through us).
#[must_use]
pub fn payment_hash_bytes(preimage: [u8; 32]) -> [u8; 32] {
    Sha256::digest(preimage).into()
}

/// Build the service-owned output note that receives the claimed value.
/// Uses the standard Poseidon-key derivation:
///   * `address = Poseidon([service_secret_key, 0])`
///   * `psi     = Poseidon([service_secret_key, service_secret_key])`
/// so the resulting note is spendable by anything that owns
/// `service_secret_key` via the `utxo` circuit's `spend_type == 0`
/// branch (i.e. a follow-up burn).
fn service_owned_note(service_secret_key: Element, note_kind: Element, amount: Element) -> Note {
    Note {
        utxo_kind: Element::new(2),
        note_kind,
        address: get_address_for_private_key(service_secret_key),
        psi: hash_merge([service_secret_key, service_secret_key]),
        value: amount,
    }
}
