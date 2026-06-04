//! Onramp settlement worker.
//!
//! Drives an onramp from `InvoicePending` to a confirmed, preimage-locked
//! note on chain. Liquidity comes from the service's own UTXO balance --
//! notes collected from claimed offramp HTLCs (see
//! [`crate::db::service_notes`]) -- spent via a Utxo Send. This closes the
//! loop: offramp deposits fund onramp withdrawals, no minting required.
//!
//! Stages:
//!   * `InvoicePending` -> poll phoenixd for payment; on settle, store the
//!     preimage (`Paid`). Past the TTL with no payment, `Expired`.
//!   * `Paid` -> spend a covering service note into a fresh note locked to
//!     the preimage's low-half field element, submit the Send (`NoteSubmitted`).
//!   * `NoteSubmitted` -> confirm on chain (`NoteConfirmed`).

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
use tracing::{error, info, warn};
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
            warn!(payment_hash = %hex::encode(o.payment_hash), error = ?e, "onramp step failed; will retry");
            let _ = onramps::record_error(&ctx.db, o.payment_hash, &e.to_string()).await;
        }
    }
    Ok(())
}

async fn step(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    match o.status {
        OnrampStatus::InvoicePending => step_await_payment(ctx, o).await,
        OnrampStatus::Paid => step_create_note(ctx, o).await,
        OnrampStatus::NoteSubmitted => step_confirm_note(ctx, o).await,
        OnrampStatus::NoteConfirmed | OnrampStatus::Expired | OnrampStatus::Failed => Ok(()),
    }
}

async fn step_await_payment(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    let hash_hex = hex::encode(o.payment_hash);
    if let Some(preimage) = ctx.lightning.incoming_preimage(&hash_hex).await? {
        info!(payment_hash = %hash_hex, "onramp invoice paid; preimage obtained");
        onramps::record_preimage(&ctx.db, o.payment_hash, preimage).await?;
    } else if Utc::now() > o.expires_at {
        info!(payment_hash = %hash_hex, "onramp invoice expired unpaid; cancelling");
        onramps::update_status(&ctx.db, o.payment_hash, OnrampStatus::Expired).await?;
    }
    Ok(())
}

async fn step_create_note(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    let preimage = o
        .preimage
        .ok_or_else(|| eyre::eyre!("Paid onramp without preimage"))?;

    // Output the user redeems: a normal UTXO note whose spend key is the
    // preimage's low-half field element. The user, having paid the
    // invoice, knows the preimage and can derive the same key.
    let unlock_key = preimage_field(preimage);
    let onramp_note = preimage_locked_note(unlock_key, o.note_kind, o.amount);

    // Fund it from a service-owned note that covers the amount.
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

    // Change (if any) returns to the service ledger under a fresh key so
    // its commitment is unique.
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

    // Claim the input note first so a crash after submit doesn't double
    // spend it on the next tick. If it was already consumed (race), bail.
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
    onramps::record_note_submitted(&ctx.db, o.payment_hash, onramp_note.commitment(), resp.txn_hash)
        .await?;
    info!(
        payment_hash = %hex::encode(o.payment_hash),
        txn_hash = %resp.txn_hash,
        "onramp note submitted"
    );
    Ok(())
}

async fn step_confirm_note(ctx: &OnrampContext, o: &Onramp) -> eyre::Result<()> {
    let txn_hash = match o.txn_hash {
        Some(h) => h,
        None => return Ok(()),
    };
    if ctx.ciphera.transaction_height(txn_hash).await?.is_some() {
        info!(payment_hash = %hex::encode(o.payment_hash), "onramp note confirmed on chain");
        onramps::update_status(&ctx.db, o.payment_hash, OnrampStatus::NoteConfirmed).await?;
    }
    Ok(())
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

/// Reduce a 32-byte preimage to a single BN254 field element by taking
/// its **low 16 bytes** (least-significant half). A full 32-byte (256-bit)
/// value can exceed the ~254-bit BN254 modulus and would reduce
/// ambiguously; 128 bits always fits unambiguously. The redeemer derives
/// the same key the same way. `Element::from_be_bytes` reads big-endian,
/// so the low half is the trailing 16 bytes.
pub fn preimage_field(preimage: [u8; 32]) -> Element {
    let mut buf = [0u8; 32];
    buf[16..].copy_from_slice(&preimage[16..]);
    Element::from_be_bytes(buf)
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
