#![deny(clippy::disallowed_methods)]

mod client;
mod constants;
mod eip7702;
mod erc20;
mod error;
mod rollup;
mod social_recovery;
#[cfg(test)]
mod tests;
pub mod util;
pub mod wallet;

pub use client::{Client, ConfirmationType};
pub use eip7702::Eip7702Account;
pub use erc20::ERC20Contract;
pub use error::{Error, Result};
pub use rollup::RollupContract;
pub use social_recovery::{GuardianConfig, GuardianEntry, SocialRecoveryContract};

pub use web3::{
    signing::SecretKey,
    types::{Address, H256, U256},
};

pub use secp256k1::SecretKey as Secp256k1SecretKey;
