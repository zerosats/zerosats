use crate::{
    NoteURLPayload, decode_activity_url_payload, note::Note,
};
use element::Element;
use serde::{Deserialize, Serialize};


/// InputNote is a Note that belongs to the current user, i.e. they have the
/// spending sercret key and can therefore use it as an input, "spending" the note. Extra
/// constraints need to be applied to input notes to ensure they are valid.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InputNote {
    /// The note to spend
    pub note: Note,
    /// Private key
    pub secret_key: Element,
}

impl InputNote {
    /// Create a new input note for the standard Poseidon-key ownership path
    /// (`spend_type == 0`, kinds 1..=4 with an inactive timelock).
    #[must_use]
    pub fn new(note: Note, secret_key: Element) -> Self {
        Self {
            note,
            secret_key,
            ..Self::default()
        }
    }

    /// Create a new padding note
    #[must_use]
    pub fn padding_note() -> Self {
        Self::default()
    }

    /// Generates a new note with given value, for an ephemeral private key, the private key
    /// must only be used once
    #[must_use]
    pub fn new_from_ephemeral_private_key(private_key: Element, value: Element) -> Self {
        Self::new(
            Note::new_from_ephemeral_private_key(private_key, value),
            private_key,
        )
    }

    /// Generates an InputNote from a link string e.g. /s#A0F3...
    #[must_use]
    pub fn new_from_link(link: &str) -> Self {
        InputNote::from(&decode_activity_url_payload(link))
    }

    /// Generates a Ciphera link from the Note + Private Key
    #[must_use]
    pub fn generate_link(&self) -> String {
        let payload: NoteURLPayload = self.into();
        payload.encode_activity_url_payload()
    }
}
