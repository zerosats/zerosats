use chrono::{DateTime, Utc};
use element::Element;

/// Per-onramp state.
///
/// The service generates the preimage up front, so it can both lock the
/// deposit note to it and reclaim it. Flow:
///   * `InvoicePending` -> a bolt11 has been issued; the worker commits a
///     preimage-locked note on chain, funded from the service balance.
///   * `Committed` -> the note is on chain. While the invoice is unpaid the
///     preimage stays secret. On payment -> `Settled` (preimage revealed,
///     user redeems). Past the TTL with no payment -> the worker spends the
///     note back (`Refunding`).
///   * `Settled` (terminal) -> paid; preimage exposed via the status route.
///   * `Refunding` -> refund Send submitted; `Refunded` (terminal) once it
///     confirms and the liquidity is back in the service ledger.
///
/// `Failed` is the unrecoverable terminal exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnrampStatus {
    InvoicePending,
    Committed,
    Settled,
    Refunding,
    Refunded,
    Failed,
}

impl OnrampStatus {
    /// Terminal states the onramp worker stops touching.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Settled | Self::Refunded | Self::Failed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvoicePending => "InvoicePending",
            Self::Committed => "Committed",
            Self::Settled => "Settled",
            Self::Refunding => "Refunding",
            Self::Refunded => "Refunded",
            Self::Failed => "Failed",
        }
    }
}

impl std::str::FromStr for OnrampStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "InvoicePending" => Self::InvoicePending,
            "Committed" => Self::Committed,
            "Settled" => Self::Settled,
            "Refunding" => Self::Refunding,
            "Refunded" => Self::Refunded,
            "Failed" => Self::Failed,
            other => return Err(format!("unknown OnrampStatus {other:?}")),
        })
    }
}

/// One onramp row, mirrors the `onramps` table. Keyed by `payment_hash`
/// (the bolt11 invoice hash), which doubles as the `/onramp/{payment_hash}`
/// path parameter.
#[derive(Debug, Clone)]
pub struct Onramp {
    pub payment_hash: [u8; 32],
    pub bolt11: String,
    /// Note value in wei (sat * 1e10).
    pub amount: Element,
    pub note_kind: Element,
    /// The **service-generated** preimage that keys the deposit note. Set
    /// at creation, but only revealed to the user (via the status route)
    /// once the invoice is `Settled`.
    pub preimage: [u8; 32],
    /// Commitment of the preimage-locked note, set once committed on chain.
    pub note_commitment: Option<Element>,
    /// Ciphera tx hash of the funding (commit) Send.
    pub txn_hash: Option<Element>,
    /// Ciphera tx hash of the refund Send, set when refunding on timeout.
    pub refund_txn_hash: Option<Element>,
    pub status: OnrampStatus,
    pub last_error: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
