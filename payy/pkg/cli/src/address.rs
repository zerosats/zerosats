use element::Element;
use serde::{Deserialize, Serialize};
use zk_primitives::{
    Note, generate_note_kind_bridge_evm
};
use rand::rngs::OsRng;
use rand::RngCore;
use web3::types::H160;

#[derive(Debug, Serialize, Deserialize)]
pub struct CipheraAddress {
    pub version: u8,
    pub kind: u8,
    pub public_key: Element,
    pub psi: Option<Element>,
    pub value: Element,
}

pub fn random_element() -> Element {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    Element::from_be_bytes(bytes)
}

#[must_use]
pub fn citrea_wcbtc_note_kind() -> Element {
    let chain = 5115u64; // Citrea testnet
    let address =
        H160::from_slice(&hex::decode("8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93").unwrap());

    generate_note_kind_bridge_evm(chain, address)
}

#[must_use]
pub fn citrea_usdc_note_kind() -> Element {
    let chain = 5115u64; // Citrea testnet
    let address =
        H160::from_slice(&hex::decode("52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6").unwrap());

    generate_note_kind_bridge_evm(chain, address)
}

impl From<&CipheraAddress> for Note {
    fn from(value: &CipheraAddress) -> Self {
        let psi = value
            .psi
            .unwrap_or_else(|| random_element());
        Note {
            kind: Element::new(2),
            contract: citrea_wcbtc_note_kind(),
            address: value.public_key,
            psi,
            value: value.value,
        }
    }
}

impl From<&Note> for CipheraAddress {
    fn from(note: &Note) -> Self {
        Self {
            version: 0,
            public_key: note.address,
            psi: Some(note.psi),
            value: note.value,
        }
    }
}

impl CipheraAddress {
    #[must_use]
    pub fn address(&self) -> Element {
        Note::from(self).address
    }

    #[must_use]
    pub fn commitment(&self) -> Element {
        Note::from(self).commitment()
    }

    #[must_use]
    pub fn psi(&self) -> Element {
        match self.version {
            0 => self.psi.expect("version 1 should have explicit psi"),
            _ => unreachable!("only version 1, 2 or 3 is supported"),
        }
    }

    #[must_use]
    pub fn encode_address(&self) -> String {
        let mut bytes = Vec::new();

        bytes.push(self.version);

        bytes.extend_from_slice(&self.public_key.to_be_bytes());

        if let Some(psi) = &self.psi {
            if self.version == 0 {
                bytes.extend_from_slice(&psi.to_be_bytes());
            }
        }

        let value_bytes = self.value.to_be_bytes();
        let leading_zeros = value_bytes.iter().take_while(|&&b| b == 0).count();
        #[allow(clippy::cast_possible_truncation)]
        bytes.push(leading_zeros as u8);
        bytes.extend_from_slice(&value_bytes[leading_zeros..]);

        bs58::encode(bytes).into_string()
    }
}

#[must_use]
pub fn decode_address(address: &str) -> CipheraAddress {
    let address_bytes = bs58::decode(address)
        .into_vec()
        .expect("Failed to decode base58 payload");

    let mut rest = &address_bytes[..];

    let version = rest[0];
    rest = &rest[1..];

    let public_key_bytes: [u8; 32] = rest[..32]
        .try_into()
        .expect("Not enough bytes for public_key");
    let public_key = Element::from_be_bytes(public_key_bytes);
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
    let value = Element::from_be_bytes(value_bytes);

    CipheraAddress {
        version,
        public_key,
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
    fn test_roundtrip_from_note() {
        let note = Note {
            kind: Element::new(2),
            contract: citrea_wcbtc_note_kind(),
            address: hash_merge([Element::new(101), Element::ZERO]),
            psi: Element::ZERO,
            value: Element::new(1),
        };

        let a: CipheraAddress = (&note).into();

        let encoded = a.encode_address();

        println!("encoded: {encoded}");

        let decoded_note = Note::from(&decode_address(&encoded));

        // Verify
        assert_eq!(decoded_note.address, note.address);
        assert_eq!(decoded_note.value, note.value);
        assert_eq!(decoded_note.address, note.address);
        assert_eq!(decoded_note.psi, note.psi);
    }
}
