use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

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

    async fn payment_status(&self, payment_hash_hex: &str) -> eyre::Result<LightningPaymentStatus>;
}

#[derive(Debug, Clone)]
pub struct PayResult {
    pub payment_id: String,
    pub payment_hash_hex: String,
    pub preimage: Option<[u8; 32]>,
}

/// phoenixd HTTP client. Endpoints used:
///   - `POST /payinvoice` (form body, basic auth) -- pays a bolt11 invoice.
///   - `GET  /payments/outgoing/{payment_hash}`   -- polls a payment by hash.
#[derive(Clone)]
pub struct PhoenixdClient {
    http: Client,
    api_url: String,
    api_password: String,
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
        Ok(Self {
            http: Client::new(),
            api_url: api_url.trim_end_matches('/').to_string(),
            api_password: api_password.to_string(),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PayInvoiceResponse {
    payment_id: String,
    payment_hash: String,
    payment_preimage: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetOutgoingResponse {
    payment_hash: String,
    preimage: String,
    is_paid: bool,
    completed_at: Option<u64>,
}

#[async_trait]
impl LightningClient for PhoenixdClient {
    async fn pay_invoice(&self, bolt11: &str) -> eyre::Result<PayResult> {
        let url = format!("{}/payinvoice", self.api_url);
        let resp = self
            .http
            .post(url)
            .basic_auth("", Some(&self.api_password))
            .form(&[("invoice", bolt11)])
            .send()
            .await?
            .error_for_status()?
            .json::<PayInvoiceResponse>()
            .await?;

        let preimage = parse_preimage(&resp.payment_preimage)?;
        Ok(PayResult {
            payment_id: resp.payment_id,
            payment_hash_hex: resp.payment_hash,
            preimage: Some(preimage),
        })
    }

    async fn payment_status(&self, payment_hash_hex: &str) -> eyre::Result<LightningPaymentStatus> {
        let url = format!("{}/payments/outgoing/{}", self.api_url, payment_hash_hex);
        let resp = self
            .http
            .get(url)
            .basic_auth("", Some(&self.api_password))
            .send()
            .await?;
        match resp.status() {
            StatusCode::OK => {
                let body: GetOutgoingResponse = resp.json().await?;
                if body.is_paid {
                    let preimage = parse_preimage(&body.preimage)?;
                    Ok(LightningPaymentStatus::Succeeded { preimage })
                } else if body.completed_at.is_some() {
                    Ok(LightningPaymentStatus::Failed)
                } else {
                    Ok(LightningPaymentStatus::Pending)
                }
            }
            StatusCode::NOT_FOUND => Ok(LightningPaymentStatus::Pending),
            other => {
                let text = resp.text().await.unwrap_or_default();
                eyre::bail!("phoenixd /payments/outgoing returned {other}: {text}");
            }
        }
    }
}

fn parse_preimage(hex_str: &str) -> eyre::Result<[u8; 32]> {
    let bytes = hex::decode(hex_str.trim_start_matches("0x"))?;
    if bytes.len() != 32 {
        eyre::bail!("expected 32-byte preimage, got {} bytes", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[allow(dead_code)]
fn _shape_check(p: PayInvoiceResponse) -> &'static str {
    // Silence unused-field lints; payment_hash is the field we'd reach for
    // when payment_hash_hex round-tripping matters for a future invariant
    // check.
    if p.payment_hash.is_empty() {
        "empty"
    } else {
        "ok"
    }
}

#[allow(dead_code)]
fn _shape_check_outgoing(p: GetOutgoingResponse) -> bool {
    !p.payment_hash.is_empty() || p.is_paid
}
