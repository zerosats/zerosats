pub mod bb_cli;
#[cfg(feature = "bb_rs")]
mod bb_rs;

use crate::Result;

/// Maps to BB 4.0's `--verifier_target` flag.
/// Replaces the old `recursive: bool` + `oracle_hash_keccak: bool` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifierTarget {
    /// Default UltraHonk (poseidon2, ZK, no recursion artifacts)
    Default,
    /// Proof will be recursively verified in another Noir circuit (poseidon2, ZK)
    NoirRecursive,
    /// Final proof verified on EVM via Solidity (keccak, ZK)
    Evm,
}

pub trait Backend {
    fn prove(
        program: &[u8],
        bytecode: &[u8],
        key: &[u8],
        witness: &[u8],
        target: VerifierTarget,
    ) -> Result<Vec<u8>>;
    /// Verify a proof. `proof` is `public_inputs || raw_proof`, split at `public_inputs_len` bytes.
    fn verify(proof: &[u8], key: &[u8], target: VerifierTarget, public_inputs_len: usize) -> Result<()>;
}

#[cfg(feature = "bb_rs")]
pub type DefaultBackend = bb_rs::BindingBackend;
#[cfg(not(feature = "bb_rs"))]
pub type DefaultBackend = bb_cli::CliBackend;
