//! Onramp settlement worker.
//!
//! The service generates the deposit preimage itself, so it can both lock
//! the note to it and reclaim it. Liquidity comes from the service's own
//! UTXO balance -- notes collected from claimed offramp HTLCs (see
//! [`crate::db::service_notes`]) -- spent via a Utxo Send. This closes the
//! loop: offramp deposits fund onramp withdrawals, no minting required.
//!
//! Stages:
//!   * `InvoicePending` -> commit a preimage-locked note on chain right
//!     after the invoice is issued, funded by spending a service note.
//!   * `Committed` -> poll phoenixd. On payment, flip to `Settled` (the
//!     status route now reveals the preimage so the user can redeem). Past
//!     the TTL with no payment, spend the note back to the service balance
//!     (`Refunding`).
//!   * `Refunding` -> once the refund Send confirms, return the value to
//!     the service ledger (`Refunded`).

use crate::clients::{CipheraClient, LightningClient};
use crate::db::{DbPool, onramps, service_notes};
use crate::domain::{Onramp, OnrampStatus, ServiceNote};
use crate::settlement::proof::service_owned_note;
use barretenberg::Prove;
use chrono::Utc;
use element::Element;
use hash::hash_merge;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{error, info};
use zk_primitives::{InputNote, Note, Utxo, get_address_for_private_key};

#[derive(Clone)]
pub struct OnrampContext {
    pub db: DbPool,
    pub ciphera: Arc<dyn CipheraClient>,
    pub lightning: Arc<dyn LightningClient>,
    pub tick: Duration,
}

pub async fn run_onramp_supervisor(ctx: OnrampContext) {
    let mut ticker = time::interval(ctx.tick);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        if let Err(e) = tick_once(&ctx).await {
            error!(error = ?e, "onramp supervisor tick failed");
        }
    }
}

async fn tick_once(ctx: &OnrampContext) -> eyre::Result<()> {
    for o in onramps::list_non_terminal(&ctx.db).await? {
        if let Err(e) = step(ctx, &o).await {
            tracing::warn!(payment_hash = %hex::encode(o.payment_hash), error = ?e, "onramp step failed; will retry");
            let _ = onramps::record_error(&ctx.db, o.payment_hash, &e.to_string()).await;
        }
    }
    Ok(())
}

async fn step(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    match o.status {
        OnrampStatus::InvoicePending => step_commit(ctx, o).await,
        OnrampStatus::Committed => step_await(ctx, o).await,
        OnrampStatus::Refunding => step_refund_confirm(ctx, o).await,
        OnrampStatus::Settled | OnrampStatus::Refunded | OnrampStatus::Failed => Ok(()),
    }
}

/// Commit the preimage-locked note on chain immediately, funded from a
/// service note. The preimage stays secret -- the note is only spendable
/// by whoever learns it (after paying) or, on timeout, by the service.
async fn step_commit(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    let unlock_key = preimage_field(o.preimage);
    let onramp_note = preimage_locked_note(unlock_key, o.note_kind, o.amount);

    let need = amount_to_u64(o.amount)?;
    let svc = service_notes::select_available(&ctx.db, o.note_kind, need)
        .await?
        .ok_or_else(|| {
            eyre::eyre!("insufficient service balance: no unspent note covers {need} wei")
        })?;

    let input = InputNote::new(
        service_owned_note(svc.note_secret, svc.note_kind, Element::new(svc.value)),
        svc.note_secret,
    );

    // Change returns to the service ledger under a fresh key.
    let change: Option<ServiceNote> = if svc.value > need {
        let change_secret = random_element();
        let change_value = svc.value - need;
        let change_note = service_owned_note(change_secret, svc.note_kind, Element::new(change_value));
        Some(ServiceNote {
            commitment: change_note.commitment(),
            note_secret: change_secret,
            note_kind: svc.note_kind,
            value: change_value,
            spent: false,
            source_payment_hash: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    } else {
        None
    };
    let change_output = change
        .as_ref()
        .map(|c| service_owned_note(c.note_secret, c.note_kind, Element::new(c.value)))
        .unwrap_or_else(Note::padding_note);

    let utxo = Utxo::new_send(
        [input, InputNote::padding_note()],
        [onramp_note.clone(), change_output],
    );

    // Claim the input first so a crash after submit doesn't double spend.
    if service_notes::mark_spent(&ctx.db, svc.commitment).await? == 0 {
        eyre::bail!("service note already spent by a concurrent onramp; retry");
    }

    info!(payment_hash = %hex::encode(o.payment_hash), "submitting onramp funding Send");
    let proof = match tokio::task::spawn_blocking(move || utxo.prove().map_err(|e| format!("{e:?}")))
        .await
    {
        Ok(Ok(proof)) => proof,
        Ok(Err(e)) => {
            let _ = service_notes::mark_unspent(&ctx.db, svc.commitment).await;
            return Err(eyre::eyre!("onramp utxo.prove failed: {e}"));
        }
        Err(e) => {
            let _ = service_notes::mark_unspent(&ctx.db, svc.commitment).await;
            return Err(eyre::eyre!("prove task panicked: {e}"));
        }
    };

    let resp = match ctx.ciphera.submit_transaction(proof).await {
        Ok(resp) => resp,
        Err(e) => {
            let _ = service_notes::mark_unspent(&ctx.db, svc.commitment).await;
            return Err(e);
        }
    };

    if let Some(change) = change {
        service_notes::insert(&ctx.db, &change).await?;
    }
    onramps::record_committed(&ctx.db, o.payment_hash, onramp_note.commitment(), resp.txn_hash)
        .await?;
    info!(payment_hash = %hex::encode(o.payment_hash), txn_hash = %resp.txn_hash, "onramp note committed");
    Ok(())
}

/// Committed note awaiting payment. Reveal the preimage on payment; refund
/// it on timeout.
async fn step_await(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    let hash_hex = hex::encode(o.payment_hash);
    if ctx.lightning.incoming_preimage(&hash_hex).await?.is_some() {
        info!(payment_hash = %hash_hex, "onramp invoice paid; preimage now revealed");
        onramps::update_status(&ctx.db, o.payment_hash, OnrampStatus::Settled).await?;
        return Ok(());
    }
    if Utc::now() > o.expires_at {
        info!(payment_hash = %hash_hex, "onramp invoice timed out; refunding committed note");
        step_refund(ctx, o).await?;
    }
    Ok(())
}

/// Spend the committed note back to a fresh service-owned note. The service
/// knows the preimage, so it holds the note's spend key.
async fn step_refund(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    let unlock_key = preimage_field(o.preimage);
    let committed_note = preimage_locked_note(unlock_key, o.note_kind, o.amount);
    let input = InputNote::new(committed_note, unlock_key);

    // Deterministic refund key (derived from the preimage) so the confirm
    // step can recompute the output note and re-add it to the ledger
    // without extra storage.
    let refund_key = refund_secret(o.preimage);
    let refund_note = service_owned_note(refund_key, o.note_kind, o.amount);

    let utxo = Utxo::new_send(
        [input, InputNote::padding_note()],
        [refund_note, Note::padding_note()],
    );

    let proof = prove(utxo).await?;
    let resp = ctx.ciphera.submit_transaction(proof).await?;
    onramps::record_refunding(&ctx.db, o.payment_hash, resp.txn_hash).await?;
    info!(payment_hash = %hex::encode(o.payment_hash), txn_hash = %resp.txn_hash, "onramp refund submitted");
    Ok(())
}

async fn step_refund_confirm(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    let txn_hash = match o.refund_txn_hash {
        Some(h) => h,
        None => return Ok(()),
    };
    if ctx.ciphera.transaction_height(txn_hash).await?.is_some() {
        // Refund landed: return the reclaimed value to the service ledger.
        let refund_key = refund_secret(o.preimage);
        let refund_note = service_owned_note(refund_key, o.note_kind, o.amount);
        let svc = ServiceNote {
            commitment: refund_note.commitment(),
            note_secret: refund_key,
            note_kind: o.note_kind,
            value: amount_to_u64(o.amount)?,
            spent: false,
            source_payment_hash: Some(o.payment_hash),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        service_notes::insert(&ctx.db, &svc).await?;
        onramps::update_status(&ctx.db, o.payment_hash, OnrampStatus::Refunded).await?;
        info!(payment_hash = %hex::encode(o.payment_hash), "onramp refund confirmed; liquidity returned");
    }
    Ok(())
}

async fn prove(utxo: Utxo) -> eyre::Result<zk_primitives::UtxoProof> {
    tokio::task::spawn_blocking(move || utxo.prove().map_err(|e| format!("{e:?}")))
        .await
        .map_err(|e| eyre::eyre!("prove task panicked: {e}"))?
        .map_err(|e| eyre::eyre!("onramp utxo.prove failed: {e}"))
}

/// A UTXO note unlockable by the preimage's low-half field element: a
/// standard Poseidon-key note whose key is `preimage_field(preimage)`.
pub fn preimage_locked_note(unlock_key: Element, note_kind: Element, amount: Element) -> Note {
    Note {
        utxo_kind: Element::new(2),
        note_kind,
        address: get_address_for_private_key(unlock_key),
        psi: hash_merge([unlock_key, unlock_key]),
        value: amount,
    }
}

/// Reduce a 32-byte preimage to a single BN254 field element by taking its
/// **low 16 bytes** (least-significant half). A full 32-byte (256-bit)
/// value can exceed the ~254-bit BN254 modulus and would reduce
/// ambiguously; 128 bits always fits. The redeemer derives the same key the
/// same way. `Element::from_be_bytes` reads big-endian, so the low half is
/// the trailing 16 bytes.
pub fn preimage_field(preimage: [u8; 32]) -> Element {
    let mut buf = [0u8; 32];
    buf[16..].copy_from_slice(&preimage[16..]);
    Element::from_be_bytes(buf)
}

/// Deterministic key for the refund output note, derived from the preimage
/// so it is recomputable across the submit/confirm steps. Distinct from the
/// note's own key derivations (`[k,0]` address, `[k,k]` psi).
fn refund_secret(preimage: [u8; 32]) -> Element {
    hash_merge([preimage_field(preimage), Element::new(1)])
}

fn amount_to_u64(amount: Element) -> eyre::Result<u64> {
    let limbs = amount.to_u64_array();
    if limbs[1] != 0 || limbs[2] != 0 || limbs[3] != 0 {
        eyre::bail!("note value exceeds u64 range: {amount}");
    }
    Ok(limbs[0])
}

fn random_element() -> Element {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Element::from_be_bytes(bytes)
}
