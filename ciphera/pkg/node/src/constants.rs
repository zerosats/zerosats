/// Expected block production time in ms.
/// Set higher for testenv to batch more transactions per block.
pub const MIN_BLOCK_PRODUCTION_DELAY: u64 = 10_000;

/// Maximum time to delay block production without approvals is ms.
pub const MAX_BLOCK_PRODUCTION_DELAY: u64 = 15_000;

/// Maximum time until skipping the previous block is ms.
pub const MAX_BLOCK_WAIT_DELAY: u64 = 30_000;

/// Depth of merkle tree
pub const MERKLE_TREE_DEPTH: usize = 161;
