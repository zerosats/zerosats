use element::Element;
use sha3::{Digest, Sha3_256};
use web3::types::H160;

/// Implement Serialize and Deserialize for an array of elements of a given size
#[macro_export]
macro_rules! impl_serde_for_element_array {
    ($name:ident, $size:expr) => {
        use ::serde::ser::SerializeSeq;

        impl ::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                let mut seq = serializer.serialize_seq(Some($size))?;
                for element in &self.0 {
                    seq.serialize_element(element)?;
                }
                seq.end()
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::de::Deserializer<'de>,
            {
                struct ArrayVisitor;

                impl<'de> ::serde::de::Visitor<'de> for ArrayVisitor {
                    type Value = $name;

                    fn expecting(
                        &self,
                        formatter: &mut ::std::fmt::Formatter,
                    ) -> ::std::fmt::Result {
                        write!(formatter, "a sequence of {} elements", $size)
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: ::serde::de::SeqAccess<'de>,
                    {
                        let mut elements = [element::Element::default(); $size];
                        for i in 0..$size {
                            elements[i] = seq
                                .next_element()?
                                .ok_or_else(|| ::serde::de::Error::invalid_length(i, &self))?;
                        }
                        Ok($name(elements))
                    }
                }

                deserializer.deserialize_seq(ArrayVisitor)
            }
        }
    };
}

/// Converts a slice of bytes into a vector of Elements by splitting the bytes into 32-byte chunks.
/// Each chunk is converted to an Element using Element::from_be_bytes.
///
/// # Arguments
///
/// * `bytes` - A slice of bytes to be converted into Elements.
///
/// # Returns
///
/// A vector of Elements, where each Element is created from a 32-byte chunk of the input.
///
/// # Panics
///
/// Panics if the length of `bytes` is not a multiple of 32.
#[must_use]
#[inline]
pub fn bytes_to_elements(bytes: &[u8]) -> Vec<Element> {
    assert!(
        bytes.len() % 32 == 0,
        "Input bytes length must be a multiple of 32"
    );

    bytes
        .chunks_exact(32)
        .map(|chunk| {
            let chunk_array: [u8; 32] = chunk.try_into().unwrap();
            Element::from_be_bytes(chunk_array)
        })
        .collect()
}

/// Hashes a private key using SHA3-256 and returns the resulting Element.
///
/// # Arguments
///
/// * `private_key` - The private key to be hashed.
///
/// # Returns
///
/// The hashed private key as an Element.
#[must_use]
pub fn hash_private_key_for_psi(private_key: Element) -> Element {
    let mut hasher = Sha3_256::new();
    hasher.update(private_key.to_be_bytes());
    let result = hasher.finalize();

    Element::from_be_bytes(result.into())
}

/// Generates a note kind element from address, chain ID, and note kind format.
/// The format is big endian: <note_kind_format:u16><chain:u160><address:H160><padding:2 bytes>
/// Returns an Element where bytes 31-32 contain the note kind format, bytes 29-30 contain the chain, and bytes 9-28 contain the address.
///
/// # Arguments
///
/// * `note_kind_format` - The note kind format as u8 (will be stored in 2 bytes)
/// * `chain` - The chain ID as u64 (8 bytes)
/// * `address` - The H160 address (20 bytes)
///
/// # Returns
///
/// An Element constructed from the big-endian byte representation.
#[must_use]
pub fn generate_note_kind_bridge_evm(chain: u64, address: H160) -> Element {
    let mut bytes = [0u8; 32];

    // Big endian format: note_kind_format in bytes 0-2, chain in bytes 2-10, address in bytes 10-30
    bytes[0..2].copy_from_slice(&(2u16).to_be_bytes());
    bytes[2..10].copy_from_slice(&chain.to_be_bytes());
    bytes[10..30].copy_from_slice(address.as_bytes());

    Element::from_be_bytes(bytes)
}

/// Generates a note kind element for USDC on Polygon network.
/// Uses the standard bridged asset format for USDC token on Polygon chain.
///
/// # Returns
///
/// An Element representing the note kind for USDC on Polygon with:
/// - note_kind_format: 2 (ETH based bridged asset)
/// - chain: 137 (Polygon)
/// - address: 0x3c499c542cef5e3811e1192ce70d8cc03d5c3359 (USDC contract address)
#[must_use]
pub fn bridged_polygon_usdc_note_kind() -> Element {
    let chain = 137u64; // Polygon chain
    let address =
        H160::from_slice(&hex::decode("3c499c542cef5e3811e1192ce70d8cc03d5c3359").unwrap());

    generate_note_kind_bridge_evm(chain, address)
}

/// Citrea network the note kind targets. Determines both the embedded
/// chain id and the canonical wrapped-cBTC address baked into the note kind.
///
/// Devnet is intentionally not represented: it has no canonical WCBTC
/// deployment and devnet flows construct note kinds from runtime-discovered
/// addresses via [`generate_note_kind_bridge_evm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CitreaNetwork {
    /// Citrea testnet (chain id 5115).
    Testnet,
    /// Citrea mainnet (chain id 4114).
    Mainnet,
}

impl CitreaNetwork {
    /// EVM chain id for this network.
    #[must_use]
    pub const fn chain_id(self) -> u64 {
        match self {
            Self::Testnet => 5115,
            Self::Mainnet => 4114,
        }
    }

    /// Canonical wrapped-cBTC (WCBTC) ERC20 address for this network.
    /// Must stay in lock-step with `scripts/shared.ts::wcbtcAddressFor` on
    /// the contract side — both files reference the same deployment.
    #[must_use]
    pub fn wcbtc_address(self) -> H160 {
        let hex = match self {
            Self::Testnet => "4370e27F7d91D9341bFf232d7Ee8bdfE3a9933a0",
            Self::Mainnet => "3100000000000000000000000000000000000006",
        };
        H160::from_slice(&hex::decode(hex).expect("static hex"))
    }

    /// Canonical citrea-USD (ctUSD) ERC20 address for this network.
    #[must_use]
    pub fn ctusd_address(self) -> H160 {
        let hex = match self {
            Self::Testnet => "52f74a8f9BdD29F77A5EFD7f6cb44DCF6906A4B6",
            Self::Mainnet => "8D82c4E3c936C7B5724A382a9c5a4E6Eb7aB6d5D",
        };
        H160::from_slice(&hex::decode(hex).expect("static hex"))
    }

    /// Recover a network from its EVM chain id, if known. Returns `None`
    /// for any unrecognised chain (including devnet).
    #[must_use]
    pub const fn try_from_chain_id(chain_id: u64) -> Option<Self> {
        match chain_id {
            5115 => Some(Self::Testnet),
            4114 => Some(Self::Mainnet),
            _ => None,
        }
    }
}

/// Generates a note kind element for WCBTC on the chosen Citrea network.
///
/// Layout (per [`generate_note_kind_bridge_evm`]):
/// - note_kind_format: 2 (ETH-based bridged asset)
/// - chain: 5115 (testnet) or 4114 (mainnet)
/// - address: `CitreaNetwork::wcbtc_address()` for the chosen network
#[must_use]
pub fn citrea_wcbtc_note_kind(network: CitreaNetwork) -> Element {
    generate_note_kind_bridge_evm(network.chain_id(), network.wcbtc_address())
}

/// Generates a note kind element for Citrea USD on the chosen Citrea network.
/// 0x000200000000000013fb52f74a8f9BdD29F77A5EFD7f6cb44DCF6906A4B60000
#[must_use]
pub fn citrea_ctusd_note_kind(network: CitreaNetwork) -> Element {
    generate_note_kind_bridge_evm(network.chain_id(), network.ctusd_address())
}

/// Generates a note kind element for USDC on Citrea testnet.
///
/// USDC has no canonical mainnet deployment on Citrea yet, so this helper
/// is testnet-only. When a mainnet USDC contract is registered, add a
/// `CitreaNetwork`-parameterised variant alongside this one.
///
/// # Returns
///
/// An Element representing the note kind for USDC on Citrea testnet with:
/// - note_kind_format: 2
/// - chain: 5115
/// - address: 0x52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6 (test fixture)
#[must_use]
pub fn citrea_testnet_usdc_note_kind() -> Element {
    let chain = CitreaNetwork::Testnet.chain_id();
    let address =
        H160::from_slice(&hex::decode("52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6").unwrap());

    generate_note_kind_bridge_evm(chain, address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_note_kind() {
        // Test with known values
        let chain = 0x1234u64;
        let address = H160::from([
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
        ]);

        let result = generate_note_kind_bridge_evm(chain, address);

        // Verify the structure by extracting bytes
        let result_bytes = result.to_be_bytes();

        // Check note_kind_format is in bytes 0-2
        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());

        // Check chain is in bytes 2-10
        assert_eq!(&result_bytes[2..10], &chain.to_be_bytes());

        // Check address is in bytes 10-30
        assert_eq!(&result_bytes[10..30], address.as_bytes());

        // Check last 2 bytes are zero (padding)
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_generate_note_kind_zero_values() {
        let chain = 0x0000u64;
        let address = H160::zero();

        let result = generate_note_kind_bridge_evm(chain, address);
        let result_bytes = result.to_be_bytes();

        // Check note_kind_format is in bytes 0-2
        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());

        // Check chain is in bytes 2-10 (all zeros)
        assert_eq!(&result_bytes[2..10], &[0u8; 8]);

        // Check address is in bytes 10-30 (all zeros)
        assert_eq!(&result_bytes[10..30], &[0u8; 20]);

        // Check last 2 bytes are zero (padding)
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_generate_note_kind_one_values() {
        let chain = 0x0001u64;
        let address = H160::from([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        ]);

        let result = generate_note_kind_bridge_evm(chain, address);
        let result_bytes = result.to_be_bytes();

        // Check note_kind_format is in bytes 0-2
        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());

        // Check chain is in bytes 2-10
        assert_eq!(&result_bytes[2..10], &chain.to_be_bytes());

        // Check address is in bytes 10-30
        assert_eq!(&result_bytes[10..30], address.as_bytes());

        // Check last 2 bytes are zero (padding)
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_generate_note_kind_max_values() {
        let chain = 0xFFFF_FFFF_FFFF_FFFF_u64;
        let address = H160::from([0xFF; 20]);

        let result = generate_note_kind_bridge_evm(chain, address);
        let result_bytes = result.to_be_bytes();

        // Check note_kind_format is in bytes 0-2
        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());

        // Check chain bytes are 0xFF
        assert_eq!(&result_bytes[2..10], &[0xFF; 8]);

        // Check address bytes are 0xFF
        assert_eq!(&result_bytes[10..30], &[0xFF; 20]);

        // Check last 2 bytes are zero (padding)
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_generate_note_kind_byte_order() {
        // Test that chain bytes are stored in big endian
        let chain = 0x0123_4567_89AB_CDEF_u64;
        let address = H160::zero();

        let result = generate_note_kind_bridge_evm(chain, address);
        let result_bytes = result.to_be_bytes();

        // In big endian, chain should match the to_be_bytes output
        let expected_chain_bytes = chain.to_be_bytes();
        assert_eq!(&result_bytes[2..10], &expected_chain_bytes);
    }

    #[test]
    fn test_bridged_polygon_usdc_note_kind() {
        let result = bridged_polygon_usdc_note_kind();
        let result_bytes = result.to_be_bytes();

        println!("{:?}", result.to_hex());

        // Check note_kind_format is in bytes 0-2
        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());

        // Check chain is in bytes 2-10 (big endian)
        // 137 (Polygon) = 0x89, so as u64 big endian = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x89]
        let expected_chain_bytes = 137u64.to_be_bytes();
        assert_eq!(&result_bytes[2..10], &expected_chain_bytes);

        // Check address is in bytes 10-30
        let expected_address_bytes =
            hex::decode("3c499c542cef5e3811e1192ce70d8cc03d5c3359").unwrap();
        assert_eq!(&result_bytes[10..30], &expected_address_bytes[..]);

        // Check last 2 bytes are zero (padding)
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_citrea_network_chain_ids() {
        assert_eq!(CitreaNetwork::Testnet.chain_id(), 5115);
        assert_eq!(CitreaNetwork::Mainnet.chain_id(), 4114);
    }

    #[test]
    fn test_citrea_network_try_from_chain_id() {
        assert_eq!(
            CitreaNetwork::try_from_chain_id(5115),
            Some(CitreaNetwork::Testnet)
        );
        assert_eq!(
            CitreaNetwork::try_from_chain_id(4114),
            Some(CitreaNetwork::Mainnet)
        );
        assert_eq!(CitreaNetwork::try_from_chain_id(5655), None); // devnet
        assert_eq!(CitreaNetwork::try_from_chain_id(1), None);
    }

    #[test]
    fn test_citrea_testnet_wcbtc_note_kind() {
        let result = citrea_wcbtc_note_kind(CitreaNetwork::Testnet);
        let result_bytes = result.to_be_bytes();

        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());
        assert_eq!(&result_bytes[2..10], &5115u64.to_be_bytes());
        let expected_address_bytes =
            hex::decode("4370e27F7d91D9341bFf232d7Ee8bdfE3a9933a0").unwrap();
        assert_eq!(&result_bytes[10..30], &expected_address_bytes[..]);
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_citrea_mainnet_wcbtc_note_kind() {
        let result = citrea_wcbtc_note_kind(CitreaNetwork::Mainnet);
        let result_bytes = result.to_be_bytes();

        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());
        assert_eq!(&result_bytes[2..10], &4114u64.to_be_bytes());
        let expected_address_bytes =
            hex::decode("3100000000000000000000000000000000000006").unwrap();
        assert_eq!(&result_bytes[10..30], &expected_address_bytes[..]);
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_citrea_testnet_ctusdc_note_kind() {
        let result = citrea_ctusd_note_kind(CitreaNetwork::Testnet);
        let result_bytes = result.to_be_bytes();

        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());
        assert_eq!(&result_bytes[2..10], &5115u64.to_be_bytes());
        let expected_address_bytes =
            hex::decode("52f74a8f9BdD29F77A5EFD7f6cb44DCF6906A4B6").unwrap();
        assert_eq!(&result_bytes[10..30], &expected_address_bytes[..]);
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_citrea_mainnet_ctusd_note_kind() {
        let result = citrea_ctusd_note_kind(CitreaNetwork::Mainnet);
        let result_bytes = result.to_be_bytes();

        assert_eq!(&result_bytes[0..2], &(2u16).to_be_bytes());
        assert_eq!(&result_bytes[2..10], &4114u64.to_be_bytes());
        let expected_address_bytes =
            hex::decode("8D82c4E3c936C7B5724A382a9c5a4E6Eb7aB6d5D").unwrap();
        assert_eq!(&result_bytes[10..30], &expected_address_bytes[..]);
        assert_eq!(&result_bytes[30..32], &[0u8; 2]);
    }

    #[test]
    fn test_testnet_and_mainnet_wcbtc_note_kinds_differ() {
        // Sanity: the two networks must not collide. A single note_kind
        // value must never resolve to a different token on a different
        // network.
        assert_ne!(
            citrea_wcbtc_note_kind(CitreaNetwork::Testnet),
            citrea_wcbtc_note_kind(CitreaNetwork::Mainnet),
        );
    }
}
