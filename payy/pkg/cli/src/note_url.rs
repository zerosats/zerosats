use element::Element;
use serde::{Deserialize, Serialize};
use zk_primitives::{
    Note, generate_note_kind_bridge_evm
};
use rand::rngs::OsRng;
use rand::RngCore;
use web3::types::H160;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize)]
pub struct CipheraURL {
    pub currency: u8,
    pub private_key: Element,
    pub value: Element,
}

impl From<&CipheraURL> for Note {
    fn from(value: &CipheraURL) -> Self {
        let psi = value
            .psi
            .unwrap_or_else(|| random_element());

        let contract = match value.currency {
            1 => citrea_wcbtc_note_kind(),
            2 => citrea_usdc_note_kind(),
            _ => unreachable!("currency code must be 1 or 2"),
        };

        Note {
            kind: Element::new(2),
            contract,
            address: value.private_key,
            psi: Element::ZERO,
            value: value.value,
        }
    }
}

impl From<&Note> for CipheraURL {
    fn from(note: &Note) -> Self {
        Self {
            currency: citrea_currency_from_contract(note.contract),
            private_key: note.address,
            value: note.value,
        }
    }
}

impl CipheraURL {
    #[must_use]
    pub fn address(&self) -> Element {
        Note::from(self).address
    }

    #[must_use]
    pub fn commitment(&self) -> Element {
        Note::from(self).commitment()
    }
    #[must_use]
    pub fn encode_address(&self) -> String {
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
pub fn decode_address(address: &str) -> CipheraURL {
    let address_bytes = bs58::decode(address)
        .into_vec()
        .expect("Failed to decode base58 payload");

    let mut rest = &address_bytes[..];

    let currency = rest[0];
    rest = &rest[1..];

    let public_key_bytes: [u8; 32] = rest[..32]
        .try_into()
        .expect("Not enough bytes for private_key");

    let private_key = Element::from_be_bytes(public_key_bytes);
    rest = &rest[32..];

    let psi = match version {
        0 => {
            let psi_bytes: [u8; 32] = rest[..32].try_into().expect("Not enough bytes for psi");
            rest = &rest[32..];
            Some(Element::from_be_bytes(psi_bytes))
        }
        _ => unreachable!("only version 1, 2 or 3 is supported"),
    };

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
        psi,
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
        let note = Note {
            kind: Element::new(2),
            contract: citrea_wcbtc_note_kind(),
            address: hash_merge([Element::new(101), Element::ZERO]),
            psi: Element::ZERO,
            value: Element::new(1),
        };

        let a: CipheraURL = (&note).into();

        println!("to be encoded: {:?}", a);

        let encoded = a.encode_address();

        println!("encoded: {encoded}");

        let decoded_note = Note::from(&decode_address(&encoded));

        println!("decoded: {:?}", decoded_note);

        // Verify
        assert_eq!(decoded_note.kind, note.kind);
        assert_eq!(decoded_note.contract, note.contract);
        assert_eq!(decoded_note.value, note.value);
        assert_eq!(decoded_note.address, note.address);
    }

    #[test]
    fn test_roundtrip_from_usdc_note() {
        let note = Note {
            kind: Element::new(2),
            contract: citrea_usdc_note_kind(),
            address: hash_merge([Element::new(101), Element::ZERO]),
            psi: Element::ZERO,
            value: Element::new(1),
        };

        let a: CipheraURL = (&note).into();

        println!("to be encoded: {:?}", a);

        let encoded = a.encode_address();

        println!("encoded: {encoded}");

        let decoded_note = Note::from(&decode_address(&encoded));

        println!("decoded: {:?}", decoded_note);

        // Verify
        assert_eq!(decoded_note.kind, note.kind);
        assert_eq!(decoded_note.contract, note.contract);
        assert_eq!(decoded_note.value, note.value);
        assert_eq!(decoded_note.address, note.address);
    }
}
