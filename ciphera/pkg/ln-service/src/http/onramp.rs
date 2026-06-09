use crate::db::{onramps, service_notes};
use crate::domain::{Onramp, OnrampStatus};
use crate::http::AppState;
use crate::http::error::ApiError;
use crate::settlement::onramp::{preimage_field, preimage_locked_note};
use actix_web::web;
use chrono::{Duration, Utc};
use element::Element;
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateQuery {
    /// Amount to onramp, in satoshis.
    pub amount_sat: u64,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    /// Hex-encoded payment hash; also the `/onramp/{payment_hash}` key.
    pub payment_hash: String,
    /// bolt11 invoice the user pays. Paying it both settles the onramp and
    /// reveals the preimage that unlocks the note.
    pub bolt11: String,
    pub amount_sat: u64,
    pub expires_at: String,
    pub status: String,
}

/// `GET /v0/onramp?amount_sat=N` — issue a Lightning invoice and park an
/// onramp row. The user pays the returned bolt11; the worker then mints a
/// preimage-locked note funded from the service's balance.
pub async fn create_onramp(
    state: web::Data<AppState>,
    query: web::Query<CreateQuery>,
) -> Result<web::Json<CreateResponse>, ApiError> {
    let amount_sat = query.amount_sat;
    if amount_sat == 0 {
        return Err(ApiError::BadRequest("amount_sat must be > 0".into()));
    }
    if amount_sat > state.config.max_amount_sat {
        return Err(ApiError::BadRequest(format!(
            "amount {amount_sat} sat exceeds cap {}",
            state.config.max_amount_sat
        )));
    }

    // sat -> wei (1 sat = 1e10 wei), the canonical note-value convention.
    let value_wei = u128::from(amount_sat)
        .checked_mul(10_000_000_000)
        .ok_or_else(|| ApiError::BadRequest("amount overflow converting sats to wei".into()))?;
    let amount = Element::from(value_wei);

    // Liquidity precheck: don't hand out an invoice the worker can't fund.
    // The funding Send spends a single service note, so we need one
    // unspent note that covers the amount. This is best-effort (no
    // reservation) -- the worker still guards against a concurrent spend
    // -- but it stops the common "paid an invoice, deposit can't settle"
    // failure where the service simply has no liquidity.
    let need = u64::try_from(value_wei).map_err(|_| {
        ApiError::Unavailable(format!("no service note can cover {value_wei} wei"))
    })?;
    let covering = service_notes::select_available(&state.db, state.default_note_kind, need)
        .await
        .map_err(|e| ApiError::Internal(format!("liquidity check: {e}")))?;
    if covering.is_none() {
        return Err(ApiError::Unavailable(format!(
            "insufficient service liquidity: no unspent note covers {amount_sat} sat"
        )));
    }

    let invoice = state
        .lightning
        .create_invoice(amount_sat, "ciphera onramp")
        .await
        .map_err(|e| ApiError::Internal(format!("invoice creation: {e}")))?;

    // Service-generated preimage that keys the deposit note. Kept secret
    // (stored, but not returned) until the invoice is paid; the worker
    // also uses it to refund the note on timeout.
    let mut preimage = [0u8; 32];
    OsRng.fill_bytes(&mut preimage);

    let now = Utc::now();
    let expires_at = now + Duration::seconds(state.config.quote_ttl_seconds);
    let onramp = Onramp {
        payment_hash: invoice.payment_hash,
        bolt11: invoice.bolt11.clone(),
        amount,
        note_kind: state.default_note_kind,
        preimage,
        note_commitment: None,
        txn_hash: None,
        refund_txn_hash: None,
        status: OnrampStatus::InvoicePending,
        last_error: None,
        expires_at,
        created_at: now,
        updated_at: now,
    };
    onramps::insert(&state.db, &onramp).await?;

    Ok(web::Json(CreateResponse {
        payment_hash: hex::encode(invoice.payment_hash),
        bolt11: invoice.bolt11,
        amount_sat,
        expires_at: expires_at.to_rfc3339(),
        status: OnrampStatus::InvoicePending.as_str().into(),
    }))
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
pub struct StatusResponse {
    pub payment_hash: String,
    pub status: String,
    /// Lightning preimage, hex-encoded, once the invoice is paid. The user
    /// derives the note's spend key from this (low 16 bytes) to redeem.
    pub preimage: Option<String>,
    /// The committed deposit note (commitment fields), once funded on
    /// chain. Mirrors the offramp's `note` so the CLI reconstructs the note
    /// from authoritative service data rather than re-deriving it. Safe to
    /// expose pre-payment: address/psi are one-way hashes of the secret
    /// preimage, exactly as already implied by `note_commitment`.
    pub note: Option<NoteResponse>,
    /// Commitment of the committed deposit note.
    pub note_commitment: Option<String>,
    pub bolt11: String,
    pub updated_at: String,
}

/// `GET /v0/onramp/{payment_hash}` — status, and the preimage once the
/// invoice is paid, to help the user redeem the note.
pub async fn get_onramp(
    state: web::Data<AppState>,
    path: web::Path<(String,)>,
) -> Result<web::Json<StatusResponse>, ApiError> {
    let (payment_hash_hex,) = path.into_inner();
    let mut payment_hash = [0u8; 32];
    hex::decode_to_slice(&payment_hash_hex, &mut payment_hash)
        .map_err(|_| ApiError::InvalidPaymentHash)?;

    let o = onramps::get(&state.db, payment_hash).await?;
    // Reveal the preimage ONLY once the invoice is paid (`Settled`).
    // Exposing it earlier would let anyone spend the committed note for
    // free, before paying.
    let preimage = if o.status == OnrampStatus::Settled {
        Some(hex::encode(o.preimage))
    } else {
        None
    };
    // Once committed, expose the note's commitment fields, reconstructed
    // from the same inputs the worker used. The CLI redeems against these.
    let note = o.note_commitment.map(|_| {
        let n = preimage_locked_note(preimage_field(o.preimage), o.note_kind, o.amount);
        NoteResponse {
            utxo_kind: n.utxo_kind.to_string(),
            note_kind: n.note_kind.to_string(),
            address: n.address.to_string(),
            psi: n.psi.to_string(),
            value: n.value.to_string(),
        }
    });
    Ok(web::Json(StatusResponse {
        payment_hash: hex::encode(o.payment_hash),
        status: o.status.as_str().into(),
        preimage,
        note,
        note_commitment: o.note_commitment.map(|c| c.to_string()),
        bolt11: o.bolt11,
        updated_at: o.updated_at.to_rfc3339(),
    }))
}
