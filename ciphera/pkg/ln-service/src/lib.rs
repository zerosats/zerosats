//! Offramp service: ciphera notes -> Lightning, redeemed via SlowBurn.
//!
//! See `/workspace/offramp-service-plan.md` for the historical design notes.

pub mod clients;
pub mod config;
pub mod db;
pub mod domain;
pub mod http;
pub mod settlement;

pub use config::Config;
