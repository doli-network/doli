//! Mempool policy configuration

/// Mempool policy settings
#[derive(Clone, Debug)]
pub struct MempoolPolicy {
    /// Maximum number of transactions
    pub max_count: usize,
    /// Maximum total size in bytes
    pub max_size: usize,
    /// Minimum fee rate for acceptance (satoshis per byte)
    pub min_fee_rate: u64,
    /// Maximum transaction size
    pub max_tx_size: usize,
    /// Maximum ancestor count
    pub max_ancestors: usize,
    /// Maximum time in mempool (seconds)
    pub max_age: u64,
}

impl Default for MempoolPolicy {
    fn default() -> Self {
        Self {
            max_count: 5000,
            max_size: 10 * 1024 * 1024, // 10 MB
            min_fee_rate: 1,            // 1 sat/byte minimum
            max_tx_size: 100 * 1024,    // 100 KB per transaction
            max_ancestors: 25,
            max_age: 14 * 24 * 60 * 60, // 14 days
        }
    }
}

impl MempoolPolicy {
    /// Create a policy for mainnet
    pub fn mainnet() -> Self {
        Self::default()
    }

    /// Create a policy for testnet (more permissive)
    pub fn testnet() -> Self {
        Self {
            min_fee_rate: 0,
            max_count: 10000,
            ..Default::default()
        }
    }

    /// Create a policy for local development
    pub fn local() -> Self {
        Self {
            min_fee_rate: 0,
            max_count: 1000,
            max_size: 1024 * 1024, // 1 MB
            ..Default::default()
        }
    }
}
