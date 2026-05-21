use crate::{Result, backend::Backend};
#[allow(unused_imports)]
use acvm::AcirField;
use element::Base;

pub fn verify<B: Backend>(
    key: &[u8],
    public_inputs: &[u8],
    proof: &[u8],
    oracle_hash_keccak: bool,
) -> Result<()> {
    B::verify(public_inputs, proof, key, oracle_hash_keccak)
}

#[derive(Debug, Clone)]
pub struct VerificationKeyHash(pub Base);

#[derive(Debug, Clone)]
pub struct VerificationKey(pub Vec<Base>);

impl VerificationKey {
    /// Decode a binary verification key written by `bb write_vk` into its
    /// constituent BN254 base-field elements. The binary format is a
    /// concatenation of 32-byte big-endian field elements.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() % 32 == 0 {
            let fields = bytes
                .chunks_exact(32)
                .map(Base::from_be_bytes_reduce)
                .collect();
            return Ok(VerificationKey(fields));
        }
        return Err("verification key length {} is not a multiple of 32".to_owned().into())
    }
}
