use crate::clients::{
    CipheraClient, ElementStatus, LightningClient, LightningPaymentStatus, SubmitError,
};
use crate::db::{DbPool, quotes, service_notes};
use crate::domain::{Quote, QuoteStatus, ServiceNote};
use crate::settlement::proof::{EscrowClaimProver, service_owned_note};
use chrono::Utc;
use element::Element;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct SettlementContext {
    pub db: DbPool,
    pub ciphera: Arc<dyn CipheraClient>,
    pub lightning: Arc<dyn LightningClient>,
    pub prover: Arc<dyn EscrowClaimProver>,
    /// EVM address claimed-value will be settled to (downstream burn,
    /// not produced here -- the current Escrow circuit only supports
    /// `Send`, so this address is just persisted for the future
    /// burn-from-service-note stage).
    pub service_evm_address: Element,
    /// Secret key whose `Poseidon([sk, 0])` was bound into the HTLC's
    /// `note.address` at lock time -- required by the
    /// `EscrowClaimProver` to satisfy the claim-branch witness.
    pub service_secret_key: Element,
    pub tick: Duration,
}

pub async fn run_supervisor(ctx: SettlementContext) {
    let mut ticker = time::interval(ctx.tick);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        if let Err(e) = tick_once(&ctx).await {
            error!(error = ?e, "settlement supervisor tick failed");
        }
    }
}

async fn tick_once(ctx: &SettlementContext) -> eyre::Result<()> {
    let quotes_list = quotes::list_non_terminal(&ctx.db).await?;
    for q in quotes_list {
        if let Err(e) = step(ctx, &q).await {
            warn!(payment_hash = %hex::encode(q.payment_hash), error = ?e, "step failed; will retry");
            let _ = quotes::record_error(&ctx.db, q.payment_hash, &e.to_string()).await;
        }
    }
    Ok(())
}

async fn step(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    match q.status {
        QuoteStatus::EscrowRequested => step_detect_escrow(ctx, q).await,
        QuoteStatus::EscrowDetected => step_pay_invoice(ctx, q).await,
        QuoteStatus::LightningPaying => step_poll_payment(ctx, q).await,
        QuoteStatus::LightningPaid => step_submit_claim(ctx, q).await,
        QuoteStatus::ClaimSubmitted => step_confirm_claim(ctx, q).await,
        QuoteStatus::ClaimConfirmed | QuoteStatus::Refundable | QuoteStatus::Cancelled => Ok(()),
    }
}

async fn step_detect_escrow(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    if Utc::now() > q.expires_at {
        info!(payment_hash = %hex::encode(q.payment_hash), "quote expired before escrow; cancelling");
        quotes::update_status(&ctx.db, q.payment_hash, QuoteStatus::Cancelled).await?;
        return Ok(());
    }

    match ctx.ciphera.element_status(q.note_commitment).await? {
        ElementStatus::Unspent { height, .. } => {
            info!(payment_hash = %hex::encode(q.payment_hash), height, "escrow detected on ciphera");
            quotes::update_status(&ctx.db, q.payment_hash, QuoteStatus::EscrowDetected).await?;
        }
        ElementStatus::NotFound => {}
    }
    Ok(())
}

async fn step_pay_invoice(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    info!(payment_hash = %hex::encode(q.payment_hash), "paying bolt11 invoice");
    // phoenixd-rs's pay_bolt11_invoice blocks until settled. Mark
    // LightningPaying up front so a crash mid-call doesn't lose the row's
    // state.
    quotes::record_payment_started(&ctx.db, q.payment_hash).await?;
    match ctx.lightning.pay_invoice(&q.bolt11).await {
        Ok(result) => {
            if let Some(preimage) = result.preimage {
                quotes::record_payment_succeeded(&ctx.db, q.payment_hash, preimage).await?;
            } else {
                // Synchronous phoenixd return without preimage shouldn't
                // happen, but defend against it by polling next tick.
                quotes::record_payment_started(&ctx.db, q.payment_hash).await?;
            }
        }
        Err(e) => {
            warn!(payment_hash = %hex::encode(q.payment_hash), error = ?e, "pay_invoice errored; will poll status next tick");
            quotes::record_payment_started(&ctx.db, q.payment_hash).await?;
            quotes::record_error(&ctx.db, q.payment_hash, &e.to_string()).await?;
        }
    }
    Ok(())
}

async fn step_poll_payment(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let hash_hex = hex::encode(q.payment_hash);
    match ctx.lightning.payment_status(&hash_hex).await? {
        LightningPaymentStatus::Pending => Ok(()),
        LightningPaymentStatus::Succeeded { preimage } => {
            info!(payment_hash = %hex::encode(q.payment_hash), "lightning payment succeeded");
            quotes::record_payment_succeeded(&ctx.db, q.payment_hash, preimage).await?;
            Ok(())
        }
        LightningPaymentStatus::Failed => {
            warn!(payment_hash = %hex::encode(q.payment_hash), "lightning payment failed; quote is refundable");
            quotes::update_status(&ctx.db, q.payment_hash, QuoteStatus::Refundable).await?;
            Ok(())
        }
    }
}

async fn step_submit_claim(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let preimage = q
        .preimage
        .ok_or_else(|| eyre::eyre!("LightningPaid without preimage"))?;

    // Stage shape post-Escrow split:
    //   1. Build + locally verify the HTLC claim proof (Escrow circuit).
    //   2. Submit it to the node's `/v0/transaction` endpoint as a
    //      `LeafProof::Escrow`. The node now accepts the tagged escrow
    //      variant and routes it through the `agg_escrow` aggregator
    //      bucket, so this is a real submission, not a stub.
    info!(payment_hash = %hex::encode(q.payment_hash), "generating Escrow claim proof");
    let proof = ctx
        .prover
        .build_and_prove(q, preimage, ctx.service_secret_key)
        .await?;

    info!(payment_hash = %hex::encode(q.payment_hash), "submitting Escrow claim proof to ciphera");
    match ctx.ciphera.submit_escrow_transaction(proof).await {
        Ok(resp) => {
            quotes::record_claim_submitted(&ctx.db, q.payment_hash, resp.txn_hash).await?;
            Ok(())
        }
        Err(e) => {
            // Idempotency: `output-commitments-exists` means this claim's
            // output note is already in the tree. Each claim's output is
            // keyed by the unique per-quote `note_secret`, so the only
            // thing that could have minted it is this quote's own earlier
            // submission -- i.e. the claim already landed. Treat it as
            // confirmed instead of retrying forever.
            if let Some(SubmitError::OutputCommitmentsExist(_)) =
                e.downcast_ref::<SubmitError>()
            {
                info!(
                    payment_hash = %hex::encode(q.payment_hash),
                    "claim output already on chain; marking confirmed"
                );
                quotes::update_status(&ctx.db, q.payment_hash, QuoteStatus::ClaimConfirmed)
                    .await?;
                return Ok(());
            }
            // Surface the *actual* node error rather than assuming a
            // missing-feature gap. `tick_once` records it on the quote
            // and retries next round; the locally-generated proof is
            // deterministic, so a retry after a transient node error
            // re-proves but stays correct.
            Err(e.wrap_err("submit_escrow_transaction to node failed"))
        }
    }
}

async fn step_confirm_claim(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let txn_hash = match q.claim_address {
        Some(h) => h,
        None => return Ok(()),
    };
    if ctx.ciphera.transaction_height(txn_hash).await?.is_some() {
        info!(payment_hash = %hex::encode(q.payment_hash), "claim proof confirmed on ciphera");

        // The claim minted a service-owned output note (keyed by
        // `note_secret`). Now that it is on chain and spendable, record it
        // in the service-note ledger so the onramp flow can spend it as
        // withdrawal liquidity. Idempotent on commitment.
        record_service_note(ctx, q).await?;

        // Previously stamped `LightningPaid` here, which would have
        // looped the state machine back into the claim-submit step
        // forever. `ClaimConfirmed` is the canonical terminal.
        quotes::update_status(&ctx.db, q.payment_hash, QuoteStatus::ClaimConfirmed).await?;
    }
    Ok(())
}

/// Add the claim's service-owned output note to the ledger so onramps can
/// spend it. Mirrors the note built in `EscrowClaimProver::build_and_prove`.
async fn record_service_note(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let note = service_owned_note(q.note_secret, q.note_kind, q.amount);
    let limbs = q.amount.to_u64_array();
    if limbs[1] != 0 || limbs[2] != 0 || limbs[3] != 0 {
        // Beyond u64 wei the ledger can't range-select it; skip rather
        // than store a truncated value.
        warn!(payment_hash = %hex::encode(q.payment_hash), "claim amount exceeds u64; not added to service ledger");
        return Ok(());
    }
    let svc = ServiceNote {
        commitment: note.commitment(),
        note_secret: q.note_secret,
        note_kind: q.note_kind,
        value: limbs[0],
        spent: false,
        source_payment_hash: Some(q.payment_hash),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    service_notes::insert(&ctx.db, &svc).await?;
    Ok(())
}
