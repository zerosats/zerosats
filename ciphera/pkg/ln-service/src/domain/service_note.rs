use chrono::{DateTime, Utc};
use element::Element;

/// A service-owned UTXO note, collected from claimed offramp HTLCs and
/// later spent (via a Utxo Send) to fund onramp notes. This is the
/// closed-loop liquidity source: offramp deposits become onramp
/// withdrawals.
///
/// Keyed by `note_secret`, matching `service_owned_note` in
/// `settlement::proof`:
///   * `address = Poseidon([note_secret, 0])`
///   * `psi     = Poseidon([note_secret, note_secret])`
///   * `value   = Element::from(value)`  (wei)
///
/// so the note can be reconstructed and spent from the stored fields
/// alone.
#[derive(Debug, Clone)]
pub struct ServiceNote {
    /// Note commitment; primary key (dedupes re-recorded claims).
    pub commitment: Element,
    pub note_secret: Element,
    pub note_kind: Element,
    /// Note value in wei. Kept as `u64` (not the raw `Element`) so the DB
    /// can range-select a covering note; amounts are bounded by the
    /// service's per-quote cap, well under `u64::MAX`.
    pub value: u64,
    pub spent: bool,
    /// `payment_hash` of the offramp claim that produced this note.
    pub source_payment_hash: Option<[u8; 32]>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
