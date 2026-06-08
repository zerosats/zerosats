use crate::db::quotes;
use crate::domain::{Quote, QuoteStatus};
use crate::http::error::ApiError;
use crate::http::AppState;
use crate::settlement::proof::offramp_htlc_note;
use actix_web::web;
use chrono::{Duration, Utc};
use element::Element;
use lightning_invoice::Bolt11Invoice;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use rand::RngCore;
use rand::rngs::OsRng;
use tracing::debug;

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

    let max_amount_msat = state
        .config
        .max_amount_sat
        .checked_mul(1_000)
        .ok_or_else(|| ApiError::Internal("max_amount_sat overflows msat conversion".into()))?;
    if amount_msat > max_amount_msat {
        let amount_sat = amount_msat / 1_000;
        return Err(ApiError::BadRequest(format!(
            "amount {amount_sat} sat ({amount_msat} msat) exceeds cap {} sat",
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

    // Anchor the HTLC refund branch to the current Bitcoin mainnet tip,
    // returned as a ready `TimeLock` (zero_block already in the circuit's
    // little-endian orientation).
    let lock = state
        .chain_tip
        .tip_lock(state.config.timelock_n_blocks)
        .await
        .map_err(|e| ApiError::Internal(format!("chain tip fetch: {e}")))?;

    // Bind the HTLC claim branch to the *service's* secret key (the
    // service is the redeemer, not the user) and the refund branch to
    // the user's address. The prover at settlement time witnesses the
    // claim branch using `service_secret_key` from
    // `OfframpContext` -- this must agree.
    let note = offramp_htlc_note(
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

    // The quote must expire strictly before the bolt11 invoice does.
    // Otherwise we could accept escrow against a quote that outlives the
    // invoice and then be left unable to pay it -- the user's funds
    // would be locked behind a dead invoice. Reject up front.
    let invoice_expiry = invoice
        .expires_at()
        .and_then(|d| {
            chrono::DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
        })
        .ok_or_else(|| ApiError::BadRequest("bolt11 expiry is out of range".into()))?;
    if expires_at >= invoice_expiry {
        return Err(ApiError::BadRequest(format!(
            "quote expiry ({}) is not earlier than invoice expiry ({})",
            expires_at.to_rfc3339(),
            invoice_expiry.to_rfc3339()
        )));
    }

    let quote = Quote {
        payment_hash,
        status: QuoteStatus::EscrowRequested,
        bolt11: req.bolt11.clone(),
        note_secret,
        user_address,
        note_kind,
        amount,
        zero_block: lock.zero_block,
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

    debug!(payment_hash = %hex::encode(payment_hash), "quoted. Responding with escrow data" );

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
            zero_block: hex::encode(lock.zero_block),
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
    pub ciphera_height: Option<u64>,
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
        QuoteStatus::EscrowRequested => {
            quotes::update_status(&state.db, payment_hash, QuoteStatus::Cancelled).await?;
        }
        QuoteStatus::EscrowDetected => {
            return Err(ApiError::Conflict(
                "escrow is already funded; refund is possible if lightning payment fails".into(),
            ));
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
        ciphera_height: None,
        last_error: q.last_error.clone(),
        expires_at: q.expires_at.to_rfc3339(),
        updated_at: q.updated_at.to_rfc3339(),
    }
}
