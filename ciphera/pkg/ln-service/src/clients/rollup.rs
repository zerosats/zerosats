use async_trait::async_trait;
use element::Element;
use node_interface::{ElementsResponseSingle, TransactionRequest, TransactionResponse};
use reqwest::{Client, StatusCode};
use zk_primitives::{EscrowProof, UtxoProof};

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
    /// Submit an `EscrowProof`. The current node `/v0/transaction`
    /// endpoint is `UtxoProof`-only; the default impl on
    /// [`ReqwestRollupClient`] reports this as an error so the
    /// settlement worker can park the quote without losing the proof.
    /// Once the node mempool learns to discriminate between leaf-proof
    /// types and the prover wires `agg_escrow` into the `agg_agg`
    /// batch, this stops being a stub and starts hitting whichever
    /// endpoint takes the `EscrowProof` payload (probably
    /// `/v0/transaction` with a tagged proof body).
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
        // The wire `TransactionRequest::proof` is a `LeafProof` after
        // the heterogeneous-aggregation refactor. Existing utxo-only
        // call sites wrap into the `Utxo` variant here; new escrow
        // call sites should go through `submit_escrow_transaction`.
        let body = TransactionRequest {
            proof: zk_primitives::LeafProof::Utxo(proof),
        };
        let resp = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn submit_escrow_transaction(
        &self,
        proof: EscrowProof,
    ) -> eyre::Result<TransactionResponse> {
        // Node-side support for `LeafProof::Escrow` landed alongside
        // this method's redesign; the wire shape and aggregator
        // dispatch are now in place. Same endpoint, tagged variant.
        let url = format!("{}/v0/transaction", self.base_url);
        let body = TransactionRequest {
            proof: zk_primitives::LeafProof::Escrow(proof),
        };
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
