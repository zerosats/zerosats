use crate::db::quotes;
use crate::domain::{Quote, QuoteStatus};
use crate::http::AppState;
use crate::http::error::ApiError;
use actix_web::web;
use chrono::{Duration, Utc};
use element::Element;
use lightning_invoice::Bolt11Invoice;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;
use zk_primitives::{Note, TimeLock};

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub bolt11: String,
    pub address: String,
    #[serde(default)]
    pub note_kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NoteResponse {
    pub utxo_kind: String,
    pub note_kind: String,
    pub address: String,
    pub psi: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct TimelockResponse {
    pub zero_block: String,
    pub n_blocks: u64,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    pub quote_id: Uuid,
    pub note: NoteResponse,
    pub timelock: TimelockResponse,
    pub expires_at: String,
    pub service_fee: u64,
}

pub async fn create_offramp(
    state: web::Data<AppState>,
    web::Json(req): web::Json<CreateRequest>,
) -> Result<web::Json<CreateResponse>, ApiError> {
    let invoice = Bolt11Invoice::from_str(&req.bolt11)
        .map_err(|e| ApiError::BadRequest(format!("invalid bolt11: {e}")))?;

    if invoice.is_expired() {
        return Err(ApiError::BadRequest("bolt11 is expired".into()));
    }

    let amount_msat = invoice
        .amount_milli_satoshis()
        .ok_or_else(|| ApiError::BadRequest("bolt11 must specify an amount".into()))?;

    // use amount_sat only for config check
    let amount_sat = amount_msat / 1_000;
    if amount_sat > state.config.max_amount_sat {
        return Err(ApiError::BadRequest(format!(
            "amount {amount_sat} sat exceeds cap {}",
            state.config.max_amount_sat
        )));
    }

    let payment_hash: [u8; 32] = *invoice.payment_hash().as_ref();

    let user_address = Element::from_str(&req.address)
        .map_err(|e| ApiError::BadRequest(format!("invalid address: {e}")))?;

    let note_kind = match &req.note_kind {
        Some(s) => Element::from_str(s)
            .map_err(|e| ApiError::BadRequest(format!("invalid note_kind format: {e}")))?,
        None => state.default_note_kind,
    };

    if note_kind != state.default_note_kind {
        return Err(ApiError::BadRequest(
            "only the configured Citrea WCBTC note kind is allowed".to_string(),
        ));
    }

    // bolt11 value (milli-sats) -> note value (wei, 1e7 multiplier turns msats into wei
    // for a wrapped-BTC token with 18 decimals). This is the canonical sat->wei convention.
    let value_wei = u128::from(amount_msat).checked_mul(10_000_000).ok_or_else(|| {
        ApiError::BadRequest("amount overflow when converting sats to wei".into())
    })?;
    let amount = Element::from(value_wei);

    let zero_block_hash = state
        .chain_tip
        .tip_hash()
        .await
        .map_err(|e| ApiError::Internal(format!("chain tip fetch: {e}")))?;

    let lock = TimeLock {
        zero_block: zero_block_hash,
        n_blocks: Element::new(state.config.timelock_n_blocks),
    };

    let note = Note::new_htlc_for_recipient(user_address, &lock, payment_hash, note_kind, amount);
    let note_commitment = note.commitment();

    let now = Utc::now();
    let expires_at = now + Duration::seconds(state.config.quote_ttl_seconds);
    let quote = Quote {
        quote_id: Uuid::new_v4(),
        status: QuoteStatus::Pending,
        bolt11: req.bolt11.clone(),
        payment_hash,
        user_address,
        note_kind,
        amount,
        zero_block: zero_block_hash,
        n_blocks: lock.n_blocks,
        note_commitment,
        preimage: None,
        lightning_payment_id: None,
        burn_txn_hash: None,
        last_error: None,
        expires_at,
        created_at: now,
        updated_at: now,
    };

    quotes::insert(&state.db, &quote).await?;

    Ok(web::Json(CreateResponse {
        quote_id: quote.quote_id,
        note: NoteResponse {
            utxo_kind: note.utxo_kind.to_string(),
            note_kind: note.note_kind.to_string(),
            address: note.address.to_string(),
            psi: note.psi.to_string(),
            value: note.value.to_string(),
        },
        timelock: TimelockResponse {
            zero_block: hex::encode(zero_block_hash),
            n_blocks: state.config.timelock_n_blocks,
        },
        expires_at: expires_at.to_rfc3339(),
        service_fee: 0,
    }))
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub quote_id: Uuid,
    pub status: String,
    pub payment_hash: String,
    pub preimage: Option<String>,
    pub rollup_height: Option<u64>,
    pub last_error: Option<String>,
    pub expires_at: String,
    pub updated_at: String,
}

pub async fn get_offramp(
    state: web::Data<AppState>,
    path: web::Path<(Uuid,)>,
) -> Result<web::Json<StatusResponse>, ApiError> {
    let (quote_id,) = path.into_inner();
    let q = quotes::get(&state.db, quote_id).await?;
    Ok(web::Json(status_response(&q)))
}

pub async fn cancel_offramp(
    state: web::Data<AppState>,
    path: web::Path<(Uuid,)>,
) -> Result<web::Json<StatusResponse>, ApiError> {
    let (quote_id,) = path.into_inner();
    let q = quotes::get(&state.db, quote_id).await?;

    match q.status {
        QuoteStatus::Pending => {
            quotes::update_status(&state.db, quote_id, QuoteStatus::Cancelled).await?;
        }
        QuoteStatus::EscrowDetected => {
            return Err(ApiError::Conflict(
                "escrow already detected; user must wait for timelock + refund".into(),
            ));
        }
        QuoteStatus::LightningPaying | QuoteStatus::LightningPaid => {
            return Err(ApiError::Conflict(
                "lightning payment in flight or already complete; cannot cancel".into(),
            ));
        }
        QuoteStatus::SlowBurnSubmitted | QuoteStatus::SlowBurnConfirmed => {
            return Err(ApiError::Conflict("burn already in flight".into()));
        }
        QuoteStatus::Cancelled | QuoteStatus::Refundable => {
            // idempotent no-op
        }
    }

    let q = quotes::get(&state.db, quote_id).await?;
    Ok(web::Json(status_response(&q)))
}

fn status_response(q: &Quote) -> StatusResponse {
    StatusResponse {
        quote_id: q.quote_id,
        status: q.status.as_str().into(),
        payment_hash: hex::encode(q.payment_hash),
        preimage: q.preimage.map(hex::encode),
        rollup_height: None,
        last_error: q.last_error.clone(),
        expires_at: q.expires_at.to_rfc3339(),
        updated_at: q.updated_at.to_rfc3339(),
    }
}

