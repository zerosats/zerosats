//! Offramp service: rollup notes -> Lightning, redeemed via SlowBurn.
//!
//! See `/workspace/offramp-service-plan.md` for the full design.

pub mod clients;
pub mod config;
pub mod db;
pub mod domain;
pub mod http;
pub mod settlement;

pub use config::Config;
