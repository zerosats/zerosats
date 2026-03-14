use crate::{Result, backend::{Backend, VerifierTarget}};
use element::Base;

pub fn verify<B: Backend>(key: &[u8], proof: &[u8], target: VerifierTarget, public_inputs_len: usize) -> Result<()> {
    B::verify(proof, key, target, public_inputs_len)
}

#[derive(Debug, Clone)]
pub struct VerificationKeyHash(pub Base);

#[derive(Debug, Clone)]
pub struct VerificationKey(pub Vec<Base>);
