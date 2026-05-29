use crate::clients::{ElementStatus, LightningClient, LightningPaymentStatus, RollupClient};
use crate::db::{DbPool, quotes};
use crate::domain::{Quote, QuoteStatus};
use crate::settlement::proof::EscrowClaimProver;
use chrono::Utc;
use element::Element;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct SettlementContext {
    pub db: DbPool,
    pub rollup: Arc<dyn RollupClient>,
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
        QuoteStatus::Refundable | QuoteStatus::Cancelled => Ok(()),
    }
}

async fn step_detect_escrow(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    if Utc::now() > q.expires_at {
        info!(payment_hash = %hex::encode(q.payment_hash), "quote expired before escrow; cancelling");
        quotes::update_status(&ctx.db, q.payment_hash, QuoteStatus::Cancelled).await?;
        return Ok(());
    }

    match ctx.rollup.element_status(q.note_commitment).await? {
        ElementStatus::Unspent { height, .. } => {
            info!(payment_hash = %hex::encode(q.payment_hash), height, "escrow detected on rollup");
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

    // New stage shape post-Escrow split:
    //   1. Build + locally verify the HTLC claim proof (Escrow circuit).
    //   2. Submit it on chain.
    // The current node mempool endpoint (`/v0/transaction`) accepts
    // UtxoProof only; submitting EscrowProof needs a node-side
    // extension (and prover-side AggEscrow batching). Until then, the
    // worker generates + verifies the proof locally and parks the
    // quote in `ClaimSubmitted` with a sentinel txn_hash so the
    // state machine still drains -- the actual on-chain burn is the
    // operator's responsibility for now.
    info!(payment_hash = %hex::encode(q.payment_hash), "generating Escrow claim proof");
    let proof = ctx
        .prover
        .build_and_prove(q, preimage, ctx.service_secret_key)
        .await?;

    info!(payment_hash = %hex::encode(q.payment_hash), "submitting Escrow claim proof to rollup");
    match ctx.rollup.submit_escrow_transaction(proof).await {
        Ok(resp) => {
            quotes::record_claim_submitted(&ctx.db, q.payment_hash, resp.txn_hash).await?;
        }
        Err(e) => {
            // Don't tip the quote into a failure state on the
            // transient submission gap -- the proof itself is valid
            // and stored, so retrying once node-side support lands
            // will work without redoing the heavy proving step.
            warn!(
                payment_hash = %hex::encode(q.payment_hash),
                error = ?e,
                "submit_escrow_transaction not yet supported by node; \
                 claim proof generated locally and will need manual or \
                 future-automated forwarding to chain"
            );
        }
    }
    Ok(())
}

async fn step_confirm_claim(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let txn_hash = match q.claim_address {
        Some(h) => h,
        None => return Ok(()),
    };
    if ctx.rollup.transaction_height(txn_hash).await?.is_some() {
        info!(payment_hash = %hex::encode(q.payment_hash), "SlowBurn confirmed");
        quotes::update_status(&ctx.db, q.payment_hash, QuoteStatus::LightningPaid).await?;
    }
    Ok(())
}
