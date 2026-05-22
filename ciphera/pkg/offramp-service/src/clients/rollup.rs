use async_trait::async_trait;
use element::Element;
use node_interface::{ElementsResponseSingle, TransactionRequest, TransactionResponse};
use reqwest::{Client, StatusCode};
use zk_primitives::UtxoProof;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementStatus {
    /// Note commitment is currently unspent in the rollup state tree.
    Unspent { height: u64, txn_hash: Element },
    /// Rollup node has never seen this element.
    NotFound,
}

/// Thin wrapper around the external rollup node's `/v0` HTTP surface. We
/// need three calls: submit a transaction, look up an element commitment,
/// and look up a transaction by hash.
#[async_trait]
pub trait RollupClient: Send + Sync {
    async fn submit_transaction(&self, proof: UtxoProof) -> eyre::Result<TransactionResponse>;
    async fn element_status(&self, element: Element) -> eyre::Result<ElementStatus>;
    /// `Ok(Some(height))` if the transaction is in a block, `Ok(None)` if not
    /// yet included.
    async fn transaction_height(&self, txn_hash: Element) -> eyre::Result<Option<u64>>;
}

#[derive(Debug, Clone)]
pub struct ReqwestRollupClient {
    base_url: String,
    http: Client,
}

impl ReqwestRollupClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: Client::new(),
        }
    }
}

#[async_trait]
impl RollupClient for ReqwestRollupClient {
    async fn submit_transaction(&self, proof: UtxoProof) -> eyre::Result<TransactionResponse> {
        let url = format!("{}/v0/transaction", self.base_url);
        let body = TransactionRequest { proof };
        let resp = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
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
        let resp = self.http.get(url).send().await?;
        match resp.status() {
            StatusCode::OK => {
                // The full response carries more than we need; we only care
                // that it exists and pull `block_height` opportunistically.
                let value: serde_json::Value = resp.json().await?;
                Ok(value
                    .get("block_height")
                    .and_then(|v| v.as_u64())
                    .or_else(|| value.get("height").and_then(|v| v.as_u64())))
            }
            StatusCode::NOT_FOUND => Ok(None),
            other => {
                let text = resp.text().await.unwrap_or_default();
                eyre::bail!("unexpected status from /v0/transactions: {other}: {text}");
            }
        }
    }
}
