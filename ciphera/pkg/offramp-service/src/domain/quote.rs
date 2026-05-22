use chrono::{DateTime, Utc};
use element::Element;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Per-quote state in the offramp settlement flow.
///
/// The state machine flows top to bottom: `Pending` → `EscrowDetected` →
/// `LightningPaying` → (`LightningPaid` → `SlowBurnSubmitted` →
/// `SlowBurnConfirmed`) or `Refundable` (terminal failure-to-pay).
/// `Cancelled` is a manual terminal exit from `Pending` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum QuoteStatus {
    Pending,
    EscrowDetected,
    LightningPaying,
    LightningPaid,
    SlowBurnSubmitted,
    SlowBurnConfirmed,
    Refundable,
    Cancelled,
}

impl QuoteStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::SlowBurnConfirmed | Self::Refundable | Self::Cancelled
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::EscrowDetected => "EscrowDetected",
            Self::LightningPaying => "LightningPaying",
            Self::LightningPaid => "LightningPaid",
            Self::SlowBurnSubmitted => "SlowBurnSubmitted",
            Self::SlowBurnConfirmed => "SlowBurnConfirmed",
            Self::Refundable => "Refundable",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl std::str::FromStr for QuoteStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Pending" => Self::Pending,
            "EscrowDetected" => Self::EscrowDetected,
            "LightningPaying" => Self::LightningPaying,
            "LightningPaid" => Self::LightningPaid,
            "SlowBurnSubmitted" => Self::SlowBurnSubmitted,
            "SlowBurnConfirmed" => Self::SlowBurnConfirmed,
            "Refundable" => Self::Refundable,
            "Cancelled" => Self::Cancelled,
            other => return Err(format!("unknown QuoteStatus {other:?}")),
        })
    }
}

/// One offramp quote row, mirrors the `quotes` table.
#[derive(Debug, Clone)]
pub struct Quote {
    pub quote_id: Uuid,
    pub status: QuoteStatus,
    pub bolt11: String,
    pub payment_hash: [u8; 32],
    pub user_address: Element,
    pub note_kind: Element,
    pub amount: Element,
    pub zero_block: [u8; 32],
    pub n_blocks: Element,
    pub note_commitment: Element,
    pub preimage: Option<[u8; 32]>,
    pub lightning_payment_id: Option<String>,
    pub burn_txn_hash: Option<Element>,
    pub last_error: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
