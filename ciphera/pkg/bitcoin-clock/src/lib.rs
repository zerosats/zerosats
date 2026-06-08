//! Bitcoin "clock": measures elapsed time in **mainnet** Bitcoin blocks for
//! the HTLC escrow timelock. It provides the anchor for a *lock* (the
//! current tip) and the PoW witness for a *refund* (the headers extending
//! the anchor), fetched from a mempool.space-compatible REST API.
//!
//! `bitcoin-clock` measures time only against mainnet blocks.
//!
//! **Byte order matters.** `TimeLock.zero_block` and the Noir circuit's
//! `check_pow_chain` (`noir/bitcoin/src/lib.nr`) use the *internal*
//! little-endian block hash -- the exact bytes that appear in the next
//! header's `prev_block` field. The REST API returns hashes in *display*
//! (big-endian) form, so we reverse on the way in/out and hand callers a
//! ready `TimeLock` / `TimeProof`.

use element::Element;
use eyre::{Result, bail, eyre};
use serde::Deserialize;
use std::time::Duration;
use zk_primitives::{TimeLock, TimeProof};

/// Public mainnet mempool.space API base URL.
pub const MAINNET: &str = "https://mempool.space";

/// The HTLC `TimeProof` carries exactly two headers, so a refund timelock is
/// at most two mainnet blocks of work past the anchor.
const HEADERS: u64 = 2;

/// Client for a mainnet mempool.space-compatible REST API. `base_url` is the
/// part before `/api`.
#[derive(Debug, Clone)]
pub struct BitcoinClock {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct BlockInfo {
    height: u64,
}

impl Default for BitcoinClock {
    fn default() -> Self {
        Self::mainnet()
    }
}

impl BitcoinClock {
    /// Client against the public mainnet mempool.space API.
    pub fn mainnet() -> Self {
        Self::new(MAINNET)
    }

    /// Client against a mainnet mempool.space-compatible API at `base_url`
    /// (e.g. a self-hosted instance). The Bitcoin network must be mainnet --
    /// the timelock is measured in mainnet block work.
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// Current chain tip as a `TimeLock` anchor: `zero_block` is the tip's
    /// internal (little-endian) hash, ready for the circuit; `n_blocks` is
    /// the required work on top of it.
    pub async fn tip_lock(&self, n_blocks: u64) -> Result<TimeLock> {
        let display = self
            .http
            .get(format!("{}/api/blocks/tip/hash", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(TimeLock {
            zero_block: internal_hash(display.trim())?,
            n_blocks: Element::new(n_blocks),
        })
    }

    /// Build the refund `TimeProof` for a lock: the two block headers that
    /// extend `lock.zero_block`. Errors with a clear message if fewer than
    /// two blocks have been mined since the anchor (the timelock has not
    /// elapsed yet).
    pub async fn refund_proof(&self, lock: &TimeLock) -> Result<TimeProof> {
        let anchor_display = display_hash(lock.zero_block);
        let info: BlockInfo = self
            .http
            .get(format!("{}/api/block/{anchor_display}", self.base_url))
            .send()
            .await?
            .error_for_status()
            .map_err(|e| eyre!("anchor block {anchor_display} not found: {e}"))?
            .json()
            .await?;

        let h0 = self.header_at_height(info.height + 1).await?;
        let h1 = self.header_at_height(info.height + 2).await?;

        // Sanity: the first header must chain from the anchor (this is what
        // the circuit's `check_pow_chain` asserts). Catches a byte-order or
        // wrong-anchor mistake before spending a heavy proving cycle.
        if h0[4..36] != lock.zero_block {
            bail!(
                "fetched header does not chain from the lock anchor \
                 (prev_block != zero_block); byte-order or anchor mismatch"
            );
        }

        Ok(TimeProof {
            lock: lock.clone(),
            headers: [h0, h1],
        })
    }

    async fn header_at_height(&self, height: u64) -> Result<[u8; 80]> {
        let resp = self
            .http
            .get(format!("{}/api/block-height/{height}", self.base_url))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            bail!(
                "block at height {height} is not mined yet; the {HEADERS}-block \
                 timelock has not elapsed since the anchor"
            );
        }
        let hash = resp.error_for_status()?.text().await?;
        let header_hex = self
            .http
            .get(format!("{}/api/block/{}/header", self.base_url, hash.trim()))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let bytes = hex::decode(header_hex.trim())?;
        let mut header = [0u8; 80];
        if bytes.len() != header.len() {
            bail!("expected 80-byte header, got {} bytes", bytes.len());
        }
        header.copy_from_slice(&bytes);
        Ok(header)
    }
}

/// mempool display hash (big-endian hex) -> internal little-endian `[u8; 32]`.
fn internal_hash(display_hex: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(display_hex)?;
    if bytes.len() != 32 {
        bail!("expected 32-byte block hash, got {} bytes", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out.reverse();
    Ok(out)
}

/// internal little-endian `[u8; 32]` -> mempool display hash (big-endian hex).
fn display_hash(internal: [u8; 32]) -> String {
    let mut bytes = internal;
    bytes.reverse();
    hex::encode(bytes)
}
