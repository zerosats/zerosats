//! Lightning settlement surface.
//!
//! The phoenixd HTTP plumbing is provided by the upstream `phoenixd-rs`
//! SDK <https://github.com/evd0kim/phoenixd-rs> ([`phoenixd_rs::Phoenixd`]).
//! The local [`LightningClient`] trait + [`LightningPaymentStatus`] enum
//! sit on top of it and stay stable so `settlement/worker.rs` can keep
//! mocking lightning behaviour in tests, and so the concrete error type
//! is collapsed to `eyre::Report` at the settlement-worker boundary.
//!
//! Endpoints exercised (via the SDK):
//!   - `POST /payinvoice`
//!   - `GET  /payments/outgoingbyhash/{payment_hash}`

use async_trait::async_trait;
use phoenixd_rs::{Error as PhoenixdError, InvoiceRequest, Phoenixd};
use tracing::debug;

#[derive(Debug, Clone)]
pub enum LightningPaymentStatus {
    /// Lightning hop hasn't completed; keep polling.
    Pending,
    /// Settled. `preimage` is the 32-byte secret that unlocks the HTLC note.
    Succeeded { preimage: [u8; 32] },
    /// Hard failure; the user keeps their note and will refund via timelock.
    Failed,
}

/// Lightning client surface. Only two operations are required by the
/// settlement worker: kick off a payment for a bolt11 invoice, and poll
/// the status of an in-flight payment we previously kicked off (by
/// payment_hash, which we already know from quote creation).
#[async_trait]
pub trait LightningClient: Send + Sync {
    /// phoenixd's `/payinvoice` is synchronous and returns once the payment
    /// settles or fails. We still split issue/poll because a process restart
    /// mid-call can lose track of an in-flight payment.
    async fn pay_invoice(&self, bolt11: &str) -> eyre::Result<PayResult>;

    async fn payment_status(&self, payment_hash: &str) -> eyre::Result<LightningPaymentStatus>;

    /// Issue a bolt11 invoice for `amount_sat`. phoenixd owns the
    /// preimage; it is only revealed once the invoice is paid (see
    /// [`LightningClient::incoming_preimage`]). Used by the onramp flow.
    async fn create_invoice(
        &self,
        amount_sat: u64,
        description: &str,
    ) -> eyre::Result<CreatedInvoice>;

    /// Return the preimage of a previously-issued *incoming* invoice once
    /// it has been paid; `Ok(None)` while still unpaid / unknown. Lets the
    /// onramp worker learn the preimage phoenixd generated.
    async fn incoming_preimage(&self, payment_hash: &str) -> eyre::Result<Option<[u8; 32]>>;
}

#[derive(Debug, Clone)]
pub struct PayResult {
    pub payment_hash: [u8; 32],
    pub preimage: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub struct CreatedInvoice {
    pub payment_hash: [u8; 32],
    pub bolt11: String,
}

// ---------------------------------------------------------------------------
// Thin trait-shaped wrapper consumed by `settlement/worker.rs`.
// ---------------------------------------------------------------------------

/// Thin wrapper around [`Phoenixd`]. The concrete error type is
/// collapsed to `eyre::Report` here so the rest of the crate keeps a
/// single error type at the settlement-worker boundary.
#[derive(Clone)]
pub struct PhoenixdClient {
    phoenixd: Phoenixd,
    api_url: String,
}

impl std::fmt::Debug for PhoenixdClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhoenixdClient")
            .field("api_url", &self.api_url)
            .finish_non_exhaustive()
    }
}

impl PhoenixdClient {
    pub fn new(api_password: &str, api_url: &str) -> eyre::Result<Self> {
        let trimmed = api_url.trim_end_matches('/');
        let phoenixd = Phoenixd::new(api_password, trimmed)
            .map_err(|e| eyre::eyre!("phoenixd Phoenixd::new: {e:?}"))?;
        Ok(Self {
            phoenixd,
            api_url: trimmed.to_string(),
        })
    }

    /// Exposed for diagnostics / logging.
    pub fn api_url(&self) -> &str {
        &self.api_url
    }
}

#[async_trait]
impl LightningClient for PhoenixdClient {
    async fn pay_invoice(&self, bolt11: &str) -> eyre::Result<PayResult> {
        // amount_sat = None tells phoenixd to use the invoice's encoded
        // amount. The offramp flow always quotes for a fixed amount on
        // a bolt11 with an explicit amount, so there's nothing to
        // override here.
        let resp = self
            .phoenixd
            .pay_bolt11_invoice(bolt11, None)
            .await
            .map_err(|e| eyre::eyre!("phoenixd pay_bolt11_invoice: {e:?}"))?;

        let preimage = parse_bytearray(&resp.payment_preimage, "preimage")?;
        let payment_hash = parse_bytearray(&resp.payment_hash, "payment hash")?;
        Ok(PayResult {
            payment_hash,
            preimage: Some(preimage),
        })
    }

    async fn payment_status(&self, payment_hash_hex: &str) -> eyre::Result<LightningPaymentStatus> {
        // The SDK looks the payment up by hash
        // (`/payments/outgoingbyhash/{hash}`) and maps a clean 404 to
        // `Error::NotFound`. We know the payment hash from quote
        // creation, so this is the right surface.
        match self.phoenixd.get_outgoing_invoice(payment_hash_hex).await {
            Ok(body) => {
                if body.is_paid {
                    let preimage = parse_bytearray(&body.preimage, "preimage")?;
                    Ok(LightningPaymentStatus::Succeeded { preimage })
                } else if body.completed_at.is_some() {
                    Ok(LightningPaymentStatus::Failed)
                } else {
                    Ok(LightningPaymentStatus::Pending)
                }
            }
            // No outgoing record yet -- the pay call is still in flight or
            // never landed. Keep polling.
            Err(PhoenixdError::NotFound) => Ok(LightningPaymentStatus::Pending),
            // phoenixd can answer a not-found lookup with an empty body.
            // The SDK's `make_get` decodes the body before inspecting the
            // status, so that case surfaces as a reqwest decode error
            // rather than `NotFound`. Treat it the same -- no record yet,
            // keep polling -- instead of hard-failing every tick.
            Err(PhoenixdError::ReqwestError(re)) if re.is_decode() => {
                debug!(
                    payment_hash = payment_hash_hex,
                    "phoenixd returned an undecodable body for outgoing payment; \
                     treating as no-record-yet and continuing to poll"
                );
                Ok(LightningPaymentStatus::Pending)
            }
            Err(other) => Err(eyre::eyre!("phoenixd get_outgoing_invoice: {other:?}")),
        }
    }

    async fn create_invoice(
        &self,
        amount_sat: u64,
        description: &str,
    ) -> eyre::Result<CreatedInvoice> {
        let req = InvoiceRequest::new(amount_sat, None, Some(description.to_string()), None, None)
            .map_err(|e| eyre::eyre!("phoenixd InvoiceRequest: {e:?}"))?;
        let resp = self
            .phoenixd
            .create_invoice(req)
            .await
            .map_err(|e| eyre::eyre!("phoenixd create_invoice: {e:?}"))?;
        Ok(CreatedInvoice {
            payment_hash: parse_bytearray(&resp.payment_hash, "payment hash")?,
            bolt11: resp.serialized,
        })
    }

    async fn incoming_preimage(&self, payment_hash_hex: &str) -> eyre::Result<Option<[u8; 32]>> {
        // The SDK's incoming lookup returns `anyhow::Result` (unlike the
        // outgoing one), so we can't match its error variants. Our invoice
        // exists from creation onward, so the normal path is `Ok` with
        // `is_paid` flipping true once settled; genuine errors propagate.
        let body = self
            .phoenixd
            .get_incoming_invoice(payment_hash_hex)
            .await
            .map_err(|e| eyre::eyre!("phoenixd get_incoming_invoice: {e:?}"))?;
        if body.is_paid {
            Ok(Some(parse_bytearray(&body.preimage, "preimage")?))
        } else {
            Ok(None)
        }
    }
}

fn parse_bytearray(hex_str: &str, field_name: &str) -> eyre::Result<[u8; 32]> {
    let bytes = hex::decode(hex_str.trim_start_matches("0x"))?;
    if bytes.len() != 32 {
        eyre::bail!("expected 32-byte {field_name}, got {} bytes", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}
