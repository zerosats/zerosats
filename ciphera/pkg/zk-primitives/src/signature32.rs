use element::Element;
use primitives::serde::{deserialize_base64, serialize_base64};
use serde::{Deserialize, Serialize};

use crate::ToBytes;

/// A signature is a message signed by a secret key (32-byte preimage),
/// often used to authenticate a user's possession of an address
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Signature32 {
    /// 32-byte secret key preimage for the address, required to spend a note
    pub preimage: [u8; 32],
    /// Message to be signed
    pub message: Element,
}

impl Signature32 {
    /// Create a new signature
    #[must_use]
    pub fn new(preimage: [u8; 32], message: Element) -> Self {
        Self { preimage, message }
    }

    /// Get the address (derived from preimage)
    #[must_use]
    pub fn hash(&self) -> Element {
        let element = Element::from_be_bytes(self.preimage);
        let (high, low) = element.decompose_be();
        hash::hash_merge([high, low])
    }

    /// Get the message hash
    #[must_use]
    pub fn message_hash(&self) -> Element {
        let element = Element::from_be_bytes(self.preimage);
        let (high, low) = element.decompose_be();
        hash::hash_merge([high, low, self.message])
    }
}

/// The output zk proof for a Signature32 circuit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature32ProofBytes(
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub Vec<u8>,
);

/// The public inputs for a signature proof
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signature32PublicInput {
    /// The address of the sender (derived from preimage)
    pub address: Element,
    /// The message to be signed
    pub message: Element,
}

impl Signature32PublicInput {
    /// Convert the public inputs to a bytes vector
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.address.to_be_bytes(), self.message.to_be_bytes()].concat()
    }
}

/// The output proof for a signature proof
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signature32Proof {
    /// The proof for the signature proof
    pub proof: Signature32ProofBytes,
    /// The public inputs for the signature proof
    pub public_inputs: Signature32PublicInput,
}

impl Signature32Proof {}

impl ToBytes for Signature32Proof {
    /// Convert the signature proof to a bytes vector
    fn to_bytes(&self) -> Vec<u8> {
        // TODO: move to impl detail of proving backend
        let pi = self.public_inputs.to_bytes();
        let proof = self.proof.0.clone();
        [pi.as_slice(), proof.as_slice()].concat()
    }
}