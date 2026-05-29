//! Lightning settlement surface.
//!
//! The actual phoenixd HTTP plumbing in [`Phoenixd`] mirrors the upstream
//! `phoenixd-rs` SDK <https://github.com/evd0kim/phoenixd-rs> verbatim
//! (same `Phoenixd::new`, `pay_bolt11_invoice`, `get_outgoing_invoice`,
//! same response shapes). The struct is inlined here rather than pulled
//! in as a crate dependency because the upstream's
//! `tokio ^1.52.3 / async-trait ^0.1.89 / thiserror ^2.0.18` floor
//! conflicts with this workspace's locked dep graph (see Cargo.toml).
//! When the workspace dep graph clears, swap this module's
//! `Phoenixd` for `phoenixd_rs::Phoenixd` -- the call sites in
//! [`PhoenixdClient`] don't need to change.
//!
//! The local [`LightningClient`] trait + [`LightningPaymentStatus`] enum
//! sit on top of `Phoenixd` and stay stable so `settlement/worker.rs`
//! can keep mocking lightning behaviour in tests.
//!
//! Endpoints exercised:
//!   - `POST /payinvoice`
//!   - `GET  /payments/outgoing/{payment_hash}`

use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

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
}

#[derive(Debug, Clone)]
pub struct PayResult {
    pub payment_hash: [u8; 32],
    pub preimage: Option<[u8; 32]>,
}

// ---------------------------------------------------------------------------
// `phoenixd-rs` API surface, vendored verbatim (MIT). See module docs.
// ---------------------------------------------------------------------------

/// Mirror of upstream `phoenixd_rs::Error`.
#[derive(Debug, Error)]
pub enum PhoenixdError {
    #[error("Not found")]
    NotFound,
    #[error("Invalid Url")]
    InvalidUrl,
    #[error(transparent)]
    ReqwestError(#[from] reqwest::Error),
    #[error(transparent)]
    SerdeError(#[from] serde_json::Error),
}

/// Mirror of upstream `phoenixd_rs::Phoenixd`.
#[derive(Debug, Clone)]
pub struct Phoenixd {
    api_password: String,
    api_url: Url,
    client: Client,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PayInvoiceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_sat: Option<u64>,
    invoice: String,
}

/// Mirror of upstream `phoenixd_rs::PayInvoiceResponse`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayInvoiceResponse {
    pub recipient_amount_sat: u64,
    pub routing_fee_sat: u64,
    pub payment_hash: String,
    pub payment_preimage: String,
}

/// Mirror of upstream `phoenixd_rs::GetOutgoingInvoiceResponse`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetOutgoingInvoiceResponse {
    pub payment_hash: String,
    pub preimage: String,
    pub is_paid: bool,
    pub sent: u64,
    pub fees: u64,
    pub invoice: Option<String>,
    pub completed_at: Option<u64>,
    pub created_at: u64,
}

impl Phoenixd {
    /// `Phoenixd::new(api_password, api_url)`.
    pub fn new(api_password: &str, api_url: &str) -> Result<Self, PhoenixdError> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(PhoenixdError::ReqwestError)?;
        let api_url = Url::from_str(api_url).map_err(|_| PhoenixdError::InvalidUrl)?;
        Ok(Self {
            api_password: api_password.to_string(),
            api_url,
            client,
        })
    }

    /// `Phoenixd::pay_bolt11_invoice(invoice, amount_sat)`.
    pub async fn pay_bolt11_invoice(
        &self,
        invoice: &str,
        amount_sat: Option<u64>,
    ) -> Result<PayInvoiceResponse, PhoenixdError> {
        let url = self
            .api_url
            .join("/payinvoice")
            .map_err(|_| PhoenixdError::InvalidUrl)?;
        let request = PayInvoiceRequest {
            amount_sat,
            invoice: invoice.to_string(),
        };
        let res = self
            .client
            .post(url)
            .basic_auth("", Some(&self.api_password))
            .form(&request)
            .send()
            .await?
            .error_for_status()?
            .json::<PayInvoiceResponse>()
            .await?;
        Ok(res)
    }

    /// `Phoenixd::get_outgoing_invoice(payment_hash)`.
    pub async fn get_outgoing_invoice(
        &self,
        payment_hash: &str,
    ) -> Result<GetOutgoingInvoiceResponse, PhoenixdError> {
        let url = self
            .api_url
            .join(&format!("payments/outgoing/{payment_hash}"))
            .map_err(|_| PhoenixdError::InvalidUrl)?;
        let resp = self
            .client
            .get(url)
            .basic_auth("", Some(&self.api_password))
            .send()
            .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Err(PhoenixdError::NotFound);
        }
        let body = resp
            .error_for_status()?
            .json::<GetOutgoingInvoiceResponse>()
            .await?;
        Ok(body)
    }
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

        let preimage = parse_bytearray(&resp.payment_preimage)?;
        let payment_hash = parse_bytearray(&resp.payment_hash)?;
        Ok(PayResult {
            payment_hash,
            preimage: Some(preimage),
        })
    }

    async fn payment_status(&self, payment_hash_hex: &str) -> eyre::Result<LightningPaymentStatus> {
        // phoenixd-rs maps HTTP 404 to `Error::NotFound`; treat that the
        // same as "still routing, no payment record yet" so the
        // settlement worker can keep polling without erroring.
        match self.phoenixd.get_outgoing_invoice(payment_hash_hex).await {
            Ok(body) => {
                if body.is_paid {
                    let preimage = parse_bytearray(&body.preimage)?;
                    Ok(LightningPaymentStatus::Succeeded { preimage })
                } else if body.completed_at.is_some() {
                    Ok(LightningPaymentStatus::Failed)
                } else {
                    Ok(LightningPaymentStatus::Pending)
                }
            }
            Err(PhoenixdError::NotFound) => Ok(LightningPaymentStatus::Pending),
            Err(other) => Err(eyre::eyre!("phoenixd get_outgoing_invoice: {other:?}")),
        }
    }
}

fn parse_bytearray(hex_str: &str) -> eyre::Result<[u8; 32]> {
    let bytes = hex::decode(hex_str.trim_start_matches("0x"))?;
    if bytes.len() != 32 {
        eyre::bail!("expected 32-byte preimage, got {} bytes", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}
