//! Unit conversion utilities for cBTC token amounts.
//!
//! The Ciphera CLI accepts amounts in **satoshis** (the smallest Bitcoin unit),
//! while the on-chain ERC-20 representation uses **wei** (the 18-decimal base unit
//! of Wrapped cBTC).
//!
//! Relationship:
//!   1 BTC  = 100,000,000 satoshis  (10^8)
//!   1 BTC  = 10^18 wei             (ERC-20 with 18 decimals)
//!   1 sat  = 10^10 wei

/// Number of satoshis in one BTC.
pub const SATS_PER_BTC: u64 = 100_000_000;

/// Number of wei in one satoshi.
///
/// Derived from: 1 BTC = 10^8 sats = 10^18 wei  →  1 sat = 10^10 wei.
pub const WEI_PER_SAT: u64 = 10_000_000_000;

/// Convert a satoshi amount to its wei equivalent.
///
/// # Panics
/// Panics in debug mode if the multiplication overflows `u64` (requires > ~1.8e9 BTC).
pub fn sats_to_wei(sats: u64) -> u64 {
    sats.checked_mul(WEI_PER_SAT)
        .expect("sats_to_wei: overflow converting sats to wei")
}

/// Convert a wei amount to the nearest whole satoshi, truncating any sub-satoshi remainder.
pub fn wei_to_sats(wei: u64) -> u64 {
    wei / WEI_PER_SAT
}

/// Convert a satoshi amount (as `u128`) to its wei equivalent.
pub fn sats_to_wei_u128(sats: u128) -> u128 {
    sats * WEI_PER_SAT as u128
}

/// Convert a wei amount (as `u128`) to the nearest whole satoshi.
pub fn wei_to_sats_u128(wei: u128) -> u128 {
    wei / WEI_PER_SAT as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sats_to_wei_zero() {
        assert_eq!(sats_to_wei(0), 0);
    }

    #[test]
    fn test_sats_to_wei_one_sat() {
        assert_eq!(sats_to_wei(1), WEI_PER_SAT);
    }

    #[test]
    fn test_sats_to_wei_one_btc_in_sats() {
        // 1 BTC expressed as sats → wei should equal 10^18
        assert_eq!(sats_to_wei(SATS_PER_BTC), 1_000_000_000_000_000_000u64);
    }

    #[test]
    fn test_wei_to_sats_zero() {
        assert_eq!(wei_to_sats(0), 0);
    }

    #[test]
    fn test_wei_to_sats_one_sat() {
        assert_eq!(wei_to_sats(WEI_PER_SAT), 1);
    }

    #[test]
    fn test_wei_to_sats_truncates() {
        assert_eq!(wei_to_sats(WEI_PER_SAT - 1), 0);
        assert_eq!(wei_to_sats(WEI_PER_SAT + 1), 1);
    }

    #[test]
    fn test_roundtrip() {
        let sats = 1_000u64;
        assert_eq!(wei_to_sats(sats_to_wei(sats)), sats);
    }

    #[test]
    fn test_u128_sats_to_wei() {
        assert_eq!(sats_to_wei_u128(1), WEI_PER_SAT as u128);
    }

    #[test]
    fn test_u128_wei_to_sats() {
        assert_eq!(wei_to_sats_u128(WEI_PER_SAT as u128), 1);
    }

    #[test]
    fn test_u128_roundtrip() {
        let sats: u128 = 5_000;
        assert_eq!(wei_to_sats_u128(sats_to_wei_u128(sats)), sats);
    }
}
