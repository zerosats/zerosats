use chrono::{DateTime, Utc};
use element::Element;
use serde::{Deserialize, Serialize};

/// Per-quote state in the offramp settlement flow.
///
/// The state machine flows top to bottom:
///   `EscrowRequested` (user asked for a quote; HTLC commitment computed,
///       awaiting on-chain funding)
///   → `EscrowDetected` (note commitment seen in a rollup tx)
///   → `LightningPaying`
///   → `LightningPaid` (preimage stored; ready to redeem)
///   → `ClaimSubmitted` (escrow-claim proof sent to rollup)
///   → `ClaimConfirmed` (terminal success)
///
/// Failure exits: `Refundable` (Lightning hard-failed; user can refund via
/// the timelock branch), `Cancelled` (manual or expiry-driven from a
/// pre-payment state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum QuoteStatus {
    EscrowRequested,
    EscrowDetected,
    LightningPaying,
    LightningPaid,
    ClaimSubmitted,
    ClaimConfirmed,
    Refundable,
    Cancelled,
}

impl QuoteStatus {
    /// Terminal states are the ones the settlement worker stops touching.
    /// `LightningPaid` and `ClaimSubmitted` look "done" but still need to
    /// drive subsequent steps, so they are intentionally **not** terminal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::ClaimConfirmed | Self::Refundable | Self::Cancelled
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::EscrowRequested => "EscrowRequested",
            Self::EscrowDetected => "EscrowDetected",
            Self::LightningPaying => "LightningPaying",
            Self::LightningPaid => "LightningPaid",
            Self::ClaimSubmitted => "ClaimSubmitted",
            Self::ClaimConfirmed => "ClaimConfirmed",
            Self::Refundable => "Refundable",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl std::str::FromStr for QuoteStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "EscrowRequested" => Self::EscrowRequested,
            "EscrowDetected" => Self::EscrowDetected,
            "LightningPaying" => Self::LightningPaying,
            "LightningPaid" => Self::LightningPaid,
            "ClaimSubmitted" => Self::ClaimSubmitted,
            "ClaimConfirmed" => Self::ClaimConfirmed,
            "Refundable" => Self::Refundable,
            "Cancelled" => Self::Cancelled,
            other => return Err(format!("unknown QuoteStatus {other:?}")),
        })
    }
}

/// One offramp quote row, mirrors the `quotes` table. Identity column is
/// `payment_hash` -- it is unique per quote (and rejected on duplicate
/// insert) so we don't need a separate UUID.
#[derive(Debug, Clone)]
pub struct Quote {
    /// SHA-256 hash from the bolt11 invoice. Primary key + URL parameter
    /// for the GET/cancel routes.
    pub payment_hash: [u8; 32],
    pub bolt11: String,
    /// Set once Lightning settles; remains `None` until then.
    pub preimage: Option<[u8; 32]>,
    pub note_commitment: Element,
    pub note_kind: Element,
    /// Random 32-byte secret bound into the HTLC's `psi` derivation.
    /// Re-used by the worker to reconstruct the note at proof time.
    pub note_secret: Element,
    pub amount: Element,
    /// User-provided Ciphera address (= `get_address_for_private_key(user_sk)`).
    /// Bound into the HTLC refund branch so only the user can refund.
    pub user_address: Element,
    pub zero_block: [u8; 32],
    pub n_blocks: Element,
    /// Rollup tx hash returned by `submit_escrow_transaction` once the
    /// claim proof is on chain; `None` until the worker reaches
    /// `ClaimSubmitted`.
    pub claim_address: Option<Element>,
    pub status: QuoteStatus,
    pub last_error: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
