use element::Element;
use primitives::serde::{deserialize_base64, serialize_base64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ToBytes;

/// A signature variant that simultaneously proves Poseidon ownership of an
/// address and confirms the SHA-256 computation for a given 32-byte preimage. This
/// mirrors the Noir `signature32sha` circuit and is used by the kind-6
/// (signature32sha) spend path: it lets a Bitcoin-style hashlock (SHA-256)
/// be tied to a note whose address is committed under Poseidon.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Signature32Sha {
    /// 32-byte secret preimage. Both the Poseidon address and the SHA-256
    /// hash are derived from this single value.
    pub preimage: [u8; 32],
    /// Message to be signed.
    pub message: Element,
}

impl Signature32Sha {
    /// Create a new signature
    #[must_use]
    pub fn new(preimage: [u8; 32], message: Element) -> Self {
        Self { preimage, message }
    }

    /// Get the Poseidon address derived from the preimage. Matches the
    /// `poseidon_hash` public input of the Noir circuit and the address
    /// scheme used by note kinds 5/6.
    #[must_use]
    pub fn address(&self) -> Element {
        let element = Element::from_be_bytes(self.preimage);
        let (high, low) = element.decompose_be();
        hash::hash_merge([high, low])
    }

    /// SHA-256 digest of the preimage. Constrained inside the circuit and
    /// otherwise carried as a private witness.
    #[must_use]
    pub fn sha_hash(&self) -> [u8; 32] {
        Sha256::digest(self.preimage).into()
    }

    /// Get the message hash (Poseidon hash of [high, low, message]).
    #[must_use]
    pub fn message_hash(&self) -> Element {
        let element = Element::from_be_bytes(self.preimage);
        let (high, low) = element.decompose_be();
        hash::hash_merge([high, low, self.message])
    }
}

/// The output zk proof for a `Signature32Sha` circuit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature32ShaProofBytes(
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub Vec<u8>,
);

/// The public inputs for a signature32sha proof
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signature32ShaPublicInput {
    /// The Poseidon address (derived from preimage).
    pub address: Element,
    /// The signed message.
    pub message: Element,
}

impl Signature32ShaPublicInput {
    /// Convert the public inputs to a bytes vector
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.address.to_be_bytes(), self.message.to_be_bytes()].concat()
    }
}

/// The output proof for a `Signature32Sha` proof
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signature32ShaProof {
    /// The proof for the signature32sha circuit.
    pub proof: Signature32ShaProofBytes,
    /// The public inputs for the signature32sha circuit.
    pub public_inputs: Signature32ShaPublicInput,
}

impl Signature32ShaProof {}

impl ToBytes for Signature32ShaProof {
    fn to_bytes(&self) -> Vec<u8> {
        let pi = self.public_inputs.to_bytes();
        let proof = self.proof.0.clone();
        [pi.as_slice(), proof.as_slice()].concat()
    }
}
