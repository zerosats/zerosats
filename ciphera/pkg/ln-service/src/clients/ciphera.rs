use eyre::Result;
use async_trait::async_trait;
use element::Element;
use node_interface::{ElementsResponseSingle, TransactionRequest, TransactionResponse};
use reqwest::{Client, StatusCode};
use tracing::warn;
use zk_primitives::{EscrowProof, LeafProof, UtxoProof};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementStatus {
    /// Note commitment is currently unspent in the ciphera state tree.
    Unspent { height: u64, txn_hash: Element },
    /// Rollup node has never seen this element.
    NotFound,
}

/// Node rejections the settlement worker needs to treat specially.
/// Everything else stays an opaque `eyre` error.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    /// `409 output-commitments-exists`: the node already has this
    /// transaction's output note(s) in its tree, i.e. the transaction
    /// (or an identical one) already landed. The contained hex commitments
    /// are the colliding elements the node reported.
    #[error("output commitments already exist: {0:?}")]
    OutputCommitmentsExist(Vec<String>),
}

/// Pull the colliding commitments out of a node error body when it is the
/// `output-commitments-exists` rejection; `None` for any other body.
fn output_commitments_exists(body: &str) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let err = v.get("error")?;
    if err.get("reason")?.as_str()? != "output-commitments-exists" {
        return None;
    }
    let elements = err
        .get("data")
        .and_then(|d| d.get("elements"))
        .and_then(|e| e.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    Some(elements)
}

/// Thin wrapper around the external ciphera node's `/v0` HTTP surface. We
/// need three calls: submit a transaction, look up an element commitment,
/// and look up a transaction by hash.
#[async_trait]
pub trait CipheraClient: Send + Sync {
    async fn submit_transaction(&self, proof: UtxoProof) -> eyre::Result<TransactionResponse>;
    /// Submit an `EscrowProof` (HTLC claim / refund). Same node
    /// `/v0/transaction` endpoint as [`CipheraClient::submit_transaction`],
    /// tagged as `LeafProof::Escrow` so the node routes it through the
    /// escrow leaf VK and the prover into the `agg_escrow` aggregator
    /// bucket.
    async fn submit_escrow_transaction(
        &self,
        proof: EscrowProof,
    ) -> eyre::Result<TransactionResponse>;
    async fn element_status(&self, element: Element) -> eyre::Result<ElementStatus>;
    /// `Ok(Some(height))` if the transaction is in a block, `Ok(None)` if not
    /// yet included.
    async fn transaction_height(&self, txn_hash: Element) -> eyre::Result<Option<u64>>;
}

#[derive(Debug, Clone)]
pub struct ReqwestCipheraClient {
    base_url: String,
    http: Client,
}

impl ReqwestCipheraClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        // Trim trailing slashes so the `{base}/v0/transaction` joins
        // don't produce a `//` that proxies tend to 301-normalize. A
        // redirect there is silently fatal for POSTs: reqwest's default
        // policy rewrites POST -> GET across 301/302/303, and the node's
        // `/v0/transaction` is POST-only, so the redirected request comes
        // back as 405. GET reads survive the same redirect, which makes
        // the failure look like a node-feature gap rather than a URL one.
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()?;

        Ok(Self {
            base_url,
            http,
        })
    }

    /// Submit any leaf proof to the node's `/v0/transaction` endpoint.
    /// Both the utxo and escrow submission variants funnel through here
    /// so the wire shape (tagged `LeafProof`) and error handling live in
    /// one place -- mirrors the CLI client's `submit_leaf`.
    async fn submit_leaf(&self, proof: LeafProof) -> eyre::Result<TransactionResponse> {
        let url = format!("{}/v0/transaction", self.base_url);
        let body = TransactionRequest { proof };
        let resp = self.http.post(&url).json(&body).send().await?;

        let status = resp.status();
        let final_url = resp.url().clone();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            // Idempotency signal: a retried, already-applied transaction
            // comes back as 409 output-commitments-exists. Surface it as a
            // typed error so the worker can treat it as success.
            if status == StatusCode::CONFLICT {
                if let Some(elements) = output_commitments_exists(&text) {
                    return Err(SubmitError::OutputCommitmentsExist(elements).into());
                }
            }
            eyre::bail!(
                "node rejected {url} (final url {final_url}) with {status}: {text}"
            );
        }
        Ok(resp.json().await?)
    }
}

#[async_trait]
impl CipheraClient for ReqwestCipheraClient {
    async fn submit_transaction(&self, proof: UtxoProof) -> eyre::Result<TransactionResponse> {
        self.submit_leaf(LeafProof::Utxo(proof)).await
    }

    async fn submit_escrow_transaction(
        &self,
        proof: EscrowProof,
    ) -> eyre::Result<TransactionResponse> {
        self.submit_leaf(LeafProof::Escrow(proof)).await
    }

    async fn element_status(&self, element: Element) -> eyre::Result<ElementStatus> {
        let url = format!("{}/v0/elements/{}", self.base_url, element);
        let resp = self.http.get(url).send().await?;
        match resp.status() {
            StatusCode::OK => {
                let single: ElementsResponseSingle = resp.json().await?;
                Ok(ElementStatus::Unspent {
                    height: single.height,
                    txn_hash: single.txn_hash,
                })
            }
            StatusCode::NOT_FOUND => Ok(ElementStatus::NotFound),
            other => {
                let text = resp.text().await.unwrap_or_default();
                eyre::bail!("unexpected status from /v0/elements: {other}: {text}");
            }
        }
    }

    async fn transaction_height(&self, txn_hash: Element) -> eyre::Result<Option<u64>> {
        let url = format!("{}/v0/transactions/{}", self.base_url, txn_hash);
        // A transport error here (node restarting / briefly unreachable)
        // just means "can't tell if it's confirmed yet" -- treat it as
        // not-yet-confirmed so the confirm step keeps polling instead of
        // flipping the quote into an error state every tick.
        let resp = match self.http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!(%url, error = %e, "transaction_height request failed; treating as pending");
                return Ok(None);
            }
        };
        match resp.status() {
            StatusCode::OK => {
                // Node shape is `GetTxnResponse { txn: { block_height, .. } }`
                // -- `block_height` is nested under `txn`, not top-level.
                // We only need to know it exists + pull the height.
                let value: serde_json::Value = resp.json().await?;
                Ok(value
                    .get("txn")
                    .and_then(|t| t.get("block_height"))
                    .and_then(|v| v.as_u64()))
            }
            StatusCode::NOT_FOUND => Ok(None),
            other => {
                let text = resp.text().await.unwrap_or_default();
                eyre::bail!("unexpected status from /v0/transactions: {other}: {text}");
            }
        }
    }
}
