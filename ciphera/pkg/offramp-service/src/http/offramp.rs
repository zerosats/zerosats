use crate::db::quotes;
use crate::domain::{Quote, QuoteStatus};
use crate::http::error::ApiError;
use crate::http::AppState;
use crate::settlement::proof::htlc_note_for_service_claim;
use actix_web::web;
use chrono::{Duration, Utc};
use element::Element;
use lightning_invoice::Bolt11Invoice;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use zk_primitives::TimeLock;
use rand::RngCore;
use rand::rngs::OsRng;

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
    /// Hex-encoded payment hash. Doubles as the quote's primary key for
    /// the GET / cancel routes (`/v0/offramp/{payment_hash}`).
    pub payment_hash: String,
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

    // Bind the HTLC claim branch to the *service's* secret key (the
    // service is the redeemer, not the user) and the refund branch to
    // the user's address. The prover at settlement time witnesses the
    // claim branch using `service_secret_key` from
    // `SettlementContext` -- this must agree.
    let note = htlc_note_for_service_claim(
        state.config.service_secret_key,
        user_address,
        &lock,
        payment_hash,
        note_kind,
        amount,
    );
    let note_commitment = note.commitment();

    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let note_secret = Element::from_be_bytes(bytes);

    let now = Utc::now();
    let expires_at = now + Duration::seconds(state.config.quote_ttl_seconds);
    let quote = Quote {
        payment_hash,
        status: QuoteStatus::EscrowRequested,
        bolt11: req.bolt11.clone(),
        note_secret,
        user_address,
        note_kind,
        amount,
        zero_block: zero_block_hash,
        n_blocks: lock.n_blocks,
        note_commitment,
        preimage: None,
        claim_address: None,
        last_error: None,
        expires_at,
        created_at: now,
        updated_at: now,
    };

    quotes::insert(&state.db, &quote).await?;

    Ok(web::Json(CreateResponse {
        payment_hash: hex::encode(payment_hash),
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
    path: web::Path<(String,)>,
) -> Result<web::Json<StatusResponse>, ApiError> {
    let (payment_hash_hex,) = path.into_inner();

    let mut payment_hash = [0u8; 32];
    hex::decode_to_slice(&payment_hash_hex, &mut payment_hash)
        .map_err(|_| ApiError::InvalidPaymentHash)?; // Map error to your API error typ

    let q = quotes::get(&state.db, payment_hash).await?;
    Ok(web::Json(status_response(&q)))
}

pub async fn cancel_offramp(
    state: web::Data<AppState>,
    path: web::Path<(String,)>,
) -> Result<web::Json<StatusResponse>, ApiError> {
    let (payment_hash_hex,) = path.into_inner();

    let mut payment_hash = [0u8; 32];
    hex::decode_to_slice(&payment_hash_hex, &mut payment_hash)
        .map_err(|_| ApiError::InvalidPaymentHash)?; // Map error to your API error typ

    let q = quotes::get(&state.db, payment_hash).await?;

    match q.status {
        // Pre-payment states: cancellation is just a status flip.
        QuoteStatus::EscrowRequested | QuoteStatus::EscrowDetected => {
            quotes::update_status(&state.db, payment_hash, QuoteStatus::Cancelled).await?;
        }
        // Lightning is in flight or already settled; we can't pull it
        // back without orphaning a Lightning payment.
        QuoteStatus::LightningPaying | QuoteStatus::LightningPaid => {
            return Err(ApiError::Conflict(
                "lightning payment in flight or already complete; cannot cancel".into(),
            ));
        }
        // Claim proof is already on its way / on chain.
        QuoteStatus::ClaimSubmitted | QuoteStatus::ClaimConfirmed => {
            return Err(ApiError::Conflict(
                "escrow claim already submitted; cannot cancel".into(),
            ));
        }
        // Already terminal: idempotent no-op.
        QuoteStatus::Cancelled | QuoteStatus::Refundable => {}
    }

    let q = quotes::get(&state.db, payment_hash).await?;
    Ok(web::Json(status_response(&q)))
}

fn status_response(q: &Quote) -> StatusResponse {
    StatusResponse {
        status: q.status.as_str().into(),
        payment_hash: hex::encode(q.payment_hash),
        preimage: q.preimage.map(hex::encode),
        rollup_height: None,
        last_error: q.last_error.clone(),
        expires_at: q.expires_at.to_rfc3339(),
        updated_at: q.updated_at.to_rfc3339(),
    }
}

