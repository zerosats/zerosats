use element::Element;
use serde::{Deserialize, Serialize};
use zk_primitives::{
    InputNote, Note, generate_note_kind_bridge_evm
};
use rand::rngs::OsRng;
use rand::RngCore;
use web3::types::H160;
use std::str::FromStr;
use crate::address::{citrea_wcbtc_note_kind, citrea_usdc_note_kind, citrea_currency_from_contract};
use hash::hash_merge;

#[derive(Debug, Serialize, Deserialize)]
pub struct CipheraURL {
    pub currency: u8,
    pub private_key: Element,
    pub value: Element,
}

impl From<&CipheraURL> for InputNote {
    fn from(value: &CipheraURL) -> Self {
        let contract = match value.currency {
            1 => citrea_wcbtc_note_kind(),
            2 => citrea_usdc_note_kind(),
            _ => unreachable!("currency code must be 1 or 2"),
        };
        InputNote {
            secret_key: value.private_key,
            note: Note {
                kind: Element::new(2),
                contract,
                address: hash_merge([value.private_key, Element::ZERO]),
                psi: hash_merge([value.private_key, value.private_key]),
                value: value.value,
            },
        }
    }
}

impl From<&InputNote> for CipheraURL {
    fn from(note: &InputNote) -> Self {
        Self {
            currency: citrea_currency_from_contract(note.note.contract),
            private_key: note.secret_key,
            value: note.note.value,
        }
    }
}

impl CipheraURL {
    #[must_use]
    pub fn commitment(&self) -> Element {
        InputNote::from(self).note.commitment()
    }
    #[must_use]
    pub fn encode_url(&self) -> String {
        let mut bytes = Vec::new();

        bytes.push(self.currency);

        bytes.extend_from_slice(&self.private_key.to_be_bytes());

        let value_bytes = self.value.to_be_bytes();
        let leading_zeros = value_bytes.iter().take_while(|&&b| b == 0).count();
        #[allow(clippy::cast_possible_truncation)]
        bytes.push(leading_zeros as u8);
        bytes.extend_from_slice(&value_bytes[leading_zeros..]);

        bs58::encode(bytes).into_string()
    }
}

#[must_use]
pub fn decode_url(address: &str) -> CipheraURL {
    let url_bytes = bs58::decode(address)
        .into_vec()
        .expect("Failed to decode base58 payload");

    let mut rest = &url_bytes[..];

    let currency = rest[0];
    rest = &rest[1..];

    let private_key_bytes: [u8; 32] = rest[..32]
        .try_into()
        .expect("Not enough bytes for private_key");

    let private_key = Element::from_be_bytes(private_key_bytes);
    rest = &rest[32..];

    let leading_zeros = rest[0] as usize;
    rest = &rest[1..];

    let value_len = 32 - leading_zeros;
    let value_without_leading_zeros = &rest[..value_len];
    rest = &rest[value_len..];

    let mut value_bytes = [0u8; 32];
    value_bytes[leading_zeros..].copy_from_slice(value_without_leading_zeros);
    value_bytes[leading_zeros..].copy_from_slice(value_without_leading_zeros);
    let value = Element::from_be_bytes(value_bytes);

    CipheraURL {
        currency,
        private_key,
        value,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use zk_primitives::Note;
    use hash::hash_merge;

    #[test]
    fn test_roundtrip_from_wcbtc_note() {
        let input_note = InputNote {
            secret_key: Element::new(101),
            note: Note {
                kind: Element::new(2),
                contract: citrea_wcbtc_note_kind(),
                address: hash_merge([Element::new(101), Element::ZERO]),
                psi: Element::ZERO,
                value: Element::new(1),
            },
        };

        let a: CipheraURL = (&input_note).into();

        println!("to be encoded: {:?}", a);

        let encoded = a.encode_url();

        let url_bytes = bs58::decode(encoded.clone())
            .into_vec()
            .expect("Failed to decode base58 payload");
        let size = url_bytes.len();

        println!("encoded: {encoded}, byte length {size}");

        let decoded_note = InputNote::from(&decode_url(&encoded));

        println!("decoded: {:?}", decoded_note);

        // Verify
        assert_eq!(decoded_note.note.kind, input_note.note.kind);
        assert_eq!(decoded_note.note.contract, input_note.note.contract);
        assert_eq!(decoded_note.note.value, input_note.note.value);
        assert_eq!(decoded_note.note.address, input_note.note.address);
    }

    #[test]
    fn test_roundtrip_from_usdc_note() {
        let input_note = InputNote {
            secret_key: Element::new(101),
            note: Note {
                kind: Element::new(2),
                contract: citrea_usdc_note_kind(),
                address: hash_merge([Element::new(101), Element::ZERO]),
                psi: Element::ZERO,
                value: Element::MAX,
            },
        };

        let a: CipheraURL = (&input_note).into();

        println!("to be encoded: {:?}", a);

        let encoded = a.encode_url();

        let url_bytes = bs58::decode(encoded.clone())
            .into_vec()
            .expect("Failed to decode base58 payload");
        let size = url_bytes.len();

        println!("encoded: {encoded}, byte length {size}");

        let decoded_note = InputNote::from(&decode_url(&encoded));

        println!("decoded: {:?}", decoded_note);

        // Verify
        assert_eq!(decoded_note.note.kind, input_note.note.kind);
        assert_eq!(decoded_note.note.contract, input_note.note.contract);
        assert_eq!(decoded_note.note.value, input_note.note.value);
        assert_eq!(decoded_note.note.address, input_note.note.address);
    }
}
