use async_trait::async_trait;
use zk_primitives::TimeLock;

/// Fetches the current Bitcoin chain tip as a ready `TimeLock` anchor (the
/// `zero_block` already in the circuit's little-endian orientation, plus the
/// required `n_blocks` of work). Lives behind a trait so the HTTP layer can
/// mock it in tests; the real implementation is `bitcoin_clock::BitcoinClock`
/// (mainnet only).
#[async_trait]
pub trait ChainTipClient: Send + Sync {
    async fn tip_lock(&self, n_blocks: u64) -> eyre::Result<TimeLock>;
}

#[async_trait]
impl ChainTipClient for bitcoin_clock::BitcoinClock {
    async fn tip_lock(&self, n_blocks: u64) -> eyre::Result<TimeLock> {
        bitcoin_clock::BitcoinClock::tip_lock(self, n_blocks).await
    }
}
