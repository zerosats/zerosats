mod backend;
mod circuits;
mod execute;
mod prove;
mod traits;
pub mod verify;

pub use circuits::AGG_UTXO_VERIFICATION_KEY_HASH;
pub use traits::{Prove, Verify};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
