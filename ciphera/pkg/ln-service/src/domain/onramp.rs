use chrono::{DateTime, Utc};
use element::Element;

/// Per-onramp state.
///
/// Flow: the user requests an onramp; the service issues a phoenixd
/// invoice and parks the row in `InvoicePending`. Once the invoice is
/// paid, phoenixd reveals the preimage, which the worker stores
/// (`Paid`). The worker then spends a service-owned note (collected from
/// offramp claims) into a fresh UTXO locked by the preimage's low-half
/// field element (`NoteSubmitted`), and finally confirms it on chain
/// (`NoteConfirmed`).
///
/// Failure exits: `Expired` (invoice never paid within the TTL) and
/// `Failed` (unrecoverable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnrampStatus {
    InvoicePending,
    Paid,
    NoteSubmitted,
    NoteConfirmed,
    Expired,
    Failed,
}

impl OnrampStatus {
    /// Terminal states the onramp worker stops touching.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::NoteConfirmed | Self::Expired | Self::Failed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvoicePending => "InvoicePending",
            Self::Paid => "Paid",
            Self::NoteSubmitted => "NoteSubmitted",
            Self::NoteConfirmed => "NoteConfirmed",
            Self::Expired => "Expired",
            Self::Failed => "Failed",
        }
    }
}

impl std::str::FromStr for OnrampStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "InvoicePending" => Self::InvoicePending,
            "Paid" => Self::Paid,
            "NoteSubmitted" => Self::NoteSubmitted,
            "NoteConfirmed" => Self::NoteConfirmed,
            "Expired" => Self::Expired,
            "Failed" => Self::Failed,
            other => return Err(format!("unknown OnrampStatus {other:?}")),
        })
    }
}

/// One onramp row, mirrors the `onramps` table. Keyed by `payment_hash`
/// (the bolt11 invoice hash), which doubles as the `/onramp/{hash}` path
/// parameter.
#[derive(Debug, Clone)]
pub struct Onramp {
    pub payment_hash: [u8; 32],
    pub bolt11: String,
    /// Note value in wei (sat * 1e10).
    pub amount: Element,
    pub note_kind: Element,
    /// Lightning preimage; `None` until the invoice is paid. Revealed to
    /// the user via `GET /onramp/{payment_hash}` to help them redeem.
    pub preimage: Option<[u8; 32]>,
    /// Commitment of the preimage-locked note, set once the funding Send
    /// is built.
    pub note_commitment: Option<Element>,
    /// Ciphera tx hash of the funding Send, set at `NoteSubmitted`.
    pub txn_hash: Option<Element>,
    pub status: OnrampStatus,
    pub last_error: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
