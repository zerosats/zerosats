pub mod bb_cli;
#[cfg(feature = "bb_rs")]
mod bb_rs;

use crate::Result;

pub trait Backend {
    /// Generate a proof for the given ACIR `program` and `witness`.
    ///
    /// `oracle_hash_keccak` selects the Keccak-Honk variant (used for the
    /// outermost aggregate proof verified on-chain by Solidity) over the
    /// default Poseidon-Honk variant.
    ///
    /// `recursive` flags the proof as one that will be verified inside
    /// another circuit. The pinned bb_rs API takes this as the third
    /// argument of `acir_prove_ultra_(keccak_)honk`; the bb CLI takes it
    /// as `--recursive`. `key` is consumed by the CLI path
    /// (`bb prove -k <key>`) and ignored by the bb_rs binding.
    fn prove(
        program: &[u8],
        key: &[u8],
        witness: &[u8],
        oracle_hash_keccak: bool,
        recursive: bool,
    ) -> Result<Vec<u8>>;
    fn verify(
        public_inputs: &[u8],
        proof: &[u8],
        key: &[u8],
        oracle_hash_keccak: bool,
    ) -> Result<()>;
}

#[cfg(feature = "bb_rs")]
pub type DefaultBackend = bb_rs::BindingBackend;
#[cfg(not(feature = "bb_rs"))]
pub type DefaultBackend = bb_cli::CliBackend;
