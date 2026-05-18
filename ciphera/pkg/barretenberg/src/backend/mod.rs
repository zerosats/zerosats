pub mod bb_cli;
#[cfg(feature = "bb_rs")]
mod bb_rs;

use crate::Result;

pub trait Backend {
    fn prove(
        program: &[u8],
        key: &[u8],
        witness: &[u8],
        oracle_hash_keccak: bool,
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
