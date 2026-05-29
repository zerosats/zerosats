use chrono::{DateTime, Utc};
use element::Element;
use serde::{Deserialize, Serialize};

/// Per-quote state in the offramp settlement flow.
///
/// The state machine flows top to bottom: `EscrowRequested` →
/// `EscrowDetected` → ( `LightningPaying` → `LightningPaid` → `ClaimSubmitted` →
/// `LightningPaid`) or `Refundable` (terminal failure-to-pay).
/// `Cancelled` is a manual terminal exit from `Pending` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum QuoteStatus {
    EscrowRequested,
    EscrowDetected,
    LightningPaying,
    LightningPaid,
    ClaimSubmitted,
    Refundable,
    Cancelled,
}

impl QuoteStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::LightningPaid | Self::Refundable | Self::Cancelled
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::EscrowRequested => "EscrowRequested",
            Self::EscrowDetected => "EscrowDetected",
            Self::LightningPaying => "LightningPaying",
            Self::LightningPaid => "LightningPaid",
            Self::ClaimSubmitted => "ClaimSubmitted",
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
            "Refundable" => Self::Refundable,
            "Cancelled" => Self::Cancelled,
            other => return Err(format!("unknown QuoteStatus {other:?}")),
        })
    }
}

/// One offramp quote row, mirrors the `quotes` table.
#[derive(Debug, Clone)]
pub struct Quote {
    pub payment_hash: [u8; 32],
    pub bolt11: String,
    pub preimage: Option<[u8; 32]>,
    pub note_commitment: Element,   // was Element
    pub note_kind: Element,        // added
    pub note_secret: Element,      // NEW FIELD
    pub amount: Element,
    pub user_address: Element,
    pub zero_block: [u8; 32],
    pub n_blocks: Element, // now nullable
    pub claim_address: Option<Element>,
    pub status: QuoteStatus,
    pub last_error: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}