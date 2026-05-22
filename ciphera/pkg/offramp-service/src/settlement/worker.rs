use crate::clients::{ElementStatus, LightningClient, LightningPaymentStatus, RollupClient};
use crate::db::{DbPool, quotes};
use crate::domain::{Quote, QuoteStatus};
use crate::settlement::proof::BurnProver;
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
    pub prover: Arc<dyn BurnProver>,
    pub service_evm_address: Element,
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
            warn!(quote_id = %q.quote_id, error = ?e, "step failed; will retry");
            let _ = quotes::record_error(&ctx.db, q.quote_id, &e.to_string()).await;
        }
    }
    Ok(())
}

async fn step(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    match q.status {
        QuoteStatus::Pending => step_detect_escrow(ctx, q).await,
        QuoteStatus::EscrowDetected => step_pay_invoice(ctx, q).await,
        QuoteStatus::LightningPaying => step_poll_payment(ctx, q).await,
        QuoteStatus::LightningPaid => step_submit_burn(ctx, q).await,
        QuoteStatus::SlowBurnSubmitted => step_confirm_burn(ctx, q).await,
        QuoteStatus::SlowBurnConfirmed
        | QuoteStatus::Refundable
        | QuoteStatus::Cancelled => Ok(()),
    }
}

async fn step_detect_escrow(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    if Utc::now() > q.expires_at {
        info!(quote_id = %q.quote_id, "quote expired before escrow; cancelling");
        quotes::update_status(&ctx.db, q.quote_id, QuoteStatus::Cancelled).await?;
        return Ok(());
    }

    match ctx.rollup.element_status(q.note_commitment).await? {
        ElementStatus::Unspent { height, .. } => {
            info!(quote_id = %q.quote_id, height, "escrow detected on rollup");
            quotes::update_status(&ctx.db, q.quote_id, QuoteStatus::EscrowDetected).await?;
        }
        ElementStatus::NotFound => {}
    }
    Ok(())
}

async fn step_pay_invoice(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    info!(quote_id = %q.quote_id, "paying bolt11 invoice");
    // phoenixd-rs's pay_bolt11_invoice blocks until settled. Mark
    // LightningPaying up front so a crash mid-call doesn't lose the row's
    // state.
    quotes::record_payment_started(&ctx.db, q.quote_id, "in-flight").await?;
    match ctx.lightning.pay_invoice(&q.bolt11).await {
        Ok(result) => {
            if let Some(preimage) = result.preimage {
                quotes::record_payment_succeeded(&ctx.db, q.quote_id, preimage).await?;
            } else {
                // Synchronous phoenixd return without preimage shouldn't
                // happen, but defend against it by polling next tick.
                quotes::record_payment_started(&ctx.db, q.quote_id, &result.payment_id).await?;
            }
        }
        Err(e) => {
            warn!(quote_id = %q.quote_id, error = ?e, "pay_invoice errored; will poll status next tick");
            quotes::record_payment_started(&ctx.db, q.quote_id, "in-flight").await?;
            quotes::record_error(&ctx.db, q.quote_id, &e.to_string()).await?;
        }
    }
    Ok(())
}

async fn step_poll_payment(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let hash_hex = hex::encode(q.payment_hash);
    match ctx.lightning.payment_status(&hash_hex).await? {
        LightningPaymentStatus::Pending => Ok(()),
        LightningPaymentStatus::Succeeded { preimage } => {
            info!(quote_id = %q.quote_id, "lightning payment succeeded");
            quotes::record_payment_succeeded(&ctx.db, q.quote_id, preimage).await?;
            Ok(())
        }
        LightningPaymentStatus::Failed => {
            warn!(quote_id = %q.quote_id, "lightning payment failed; quote is refundable");
            quotes::update_status(&ctx.db, q.quote_id, QuoteStatus::Refundable).await?;
            Ok(())
        }
    }
}

async fn step_submit_burn(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let preimage = q
        .preimage
        .ok_or_else(|| eyre::eyre!("LightningPaid without preimage"))?;

    info!(quote_id = %q.quote_id, "generating SlowBurn proof");
    let proof = ctx
        .prover
        .build_and_prove(q, preimage, ctx.service_evm_address)
        .await?;

    info!(quote_id = %q.quote_id, "submitting SlowBurn proof to rollup");
    let resp = ctx.rollup.submit_transaction(proof).await?;
    quotes::record_burn_submitted(&ctx.db, q.quote_id, resp.txn_hash).await?;
    Ok(())
}

async fn step_confirm_burn(ctx: &SettlementContext, q: &Quote) -> eyre::Result<()> {
    let txn_hash = match q.burn_txn_hash {
        Some(h) => h,
        None => return Ok(()),
    };
    if ctx.rollup.transaction_height(txn_hash).await?.is_some() {
        info!(quote_id = %q.quote_id, "SlowBurn confirmed");
        quotes::update_status(&ctx.db, q.quote_id, QuoteStatus::SlowBurnConfirmed).await?;
    }
    Ok(())
}
