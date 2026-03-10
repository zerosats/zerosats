use serde::{Deserialize, Serialize};

/// Network response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkResponse {
    /// Base chain contract
    pub rollup_contract: String,
    /// Base chain Id
    pub chain_id: u64,
}
