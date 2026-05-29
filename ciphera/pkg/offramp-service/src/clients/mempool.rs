use async_trait::async_trait;
use reqwest::Client;

/// Fetches the current Bitcoin chain-tip block hash (the 32-byte hash, *not*
/// the 80-byte header). This is what TimeLock's `zero_block` field stores
/// and what the refund branch commits to via `Poseidon(field_pair(zero_block))`.
#[async_trait]
pub trait ChainTipClient: Send + Sync {
    /// Returns the 32-byte block hash of the current chain tip, in the
    /// *internal byte order* that mempool.space returns. The TimeLock
    /// commitment hashes these bytes as-is, so the only invariant is that
    /// the prover (off-chain) uses the same orientation when assembling
    /// the TimeProof headers.
    async fn tip_hash(&self) -> eyre::Result<[u8; 32]>;
}

/// Real impl backed by mempool.space's `GET /api/blocks/tip/hash` endpoint.
#[derive(Debug, Clone)]
pub struct MempoolClient {
    base_url: String,
    http: Client,
}

impl MempoolClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: Client::new(),
        }
    }
}

#[async_trait]
impl ChainTipClient for MempoolClient {
    async fn tip_hash(&self) -> eyre::Result<[u8; 32]> {
        let tip_hash_hex: String = self
            .http
            .get(format!("{}/api/blocks/tip/hash", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let bytes = hex::decode(tip_hash_hex.trim())?;
        if bytes.len() != 32 {
            eyre::bail!("expected 32-byte block hash, got {} bytes", bytes.len());
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}
