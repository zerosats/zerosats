use serde::{Deserialize, Serialize};

/// Network response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkResponse {
    /// Base chain contract
    pub rollup_contract: String,
    /// Base chain ID
    pub chain_id: u64,
    /// Active escrow manager address (hex-encoded, `0x`-prefixed)
    pub escrow_manager: String,
    /// Node software version (semver)
    pub node_version: String,
    /// Required Noir (`nargo`) version for the circuits
    pub circuits_nargo_version: String,
    /// Required Barretenberg (`bb`) version for the circuits
    pub circuits_bb_version: String,
}
