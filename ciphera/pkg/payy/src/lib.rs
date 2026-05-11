//! Payy module: note-URL encoding/decoding and send-side UTXO proof building.
//!
//! - [`NoteURLPayload`] is the versioned base58 payload used in Payy share links
//!   (`/s#<payload>`). Convert to/from [`zk_primitives::InputNote`] via
//!   [`input_note_from_payload`] / [`payload_from_input_note`].
//! - [`prove_send`] wraps `zk_primitives::Utxo::new_send` + `barretenberg::Prove`
//!   to emit a send `UtxoProof` ready for node submission.

mod note_url;
mod utxo;

pub use note_url::{
    NoteURLPayload, NoteUrlDecodeError, NoteUrlDecodeResult, decode_activity_url_payload,
    input_note_from_payload, payload_from_input_note, try_decode_activity_url_payload,
};
pub use utxo::prove_send;
