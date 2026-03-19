use crate::U256;

#[derive(Debug, Clone)]
pub struct Eip1559Fees {
    /// Base fee per gas (burned to the network)
    pub base_fee: U256,
    /// Priority fee per gas (tip to validators)
    pub priority_fee: U256,
    /// Maximum total fee per gas
    pub max_fee: U256,
}

#[derive(Debug, Clone, Copy)]
pub enum FeeStrategy {
    /// For cheapest possible transactions
    Lowest,
    /// For non-urgent transactions
    Slow,
    /// For standard transactions (default)
    Standard,
    /// For time-sensitive transactions
    Fast,
}

impl Default for FeeStrategy {
    fn default() -> Self {
        Self::Standard
    }
}

impl FeeStrategy {
    /// Get priority fee percentile for this strategy
    pub fn percentile(&self) -> f64 {
        match self {
            Self::Lowest => 1.0,    // Cheaper
            Self::Slow => 25.0,     // Cheaper
            Self::Standard => 50.0, // Balanced
            Self::Fast => 90.0,     // Fast
        }
    }

    /// Get base fee buffer for this strategy
    pub fn base_fee_buffer_percent(&self) -> u64 {
        match self {
            Self::Lowest => 101,   // 1% buffer
            Self::Slow => 110,     // 10% buffer
            Self::Standard => 125, // 25% buffer
            Self::Fast => 150,     // 50% buffer
        }
    }
}
