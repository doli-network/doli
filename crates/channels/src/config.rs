//! Channel configuration.

use doli_core::{Amount, BlockHeight};
use serde::{Deserialize, Serialize};

/// Channel configuration parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Dispute window in blocks. The delay before a unilateral close
    /// can be claimed. Default: 144 blocks (~24 min at 10s slots).
    pub dispute_window: BlockHeight,

    /// Minimum channel capacity in base units.
    pub min_channel_capacity: Amount,

    /// Channel reserve: minimum balance each side must maintain.
    /// Prevents costless close attacks. Default: 1% of capacity.
    pub reserve_percent: u8,

    /// Maximum number of in-flight HTLCs per channel.
    pub max_htlcs: u16,

    /// HTLC minimum value in base units.
    pub htlc_minimum: Amount,

    /// Fee rate for on-chain transactions (units per byte).
    pub fee_rate: Amount,

    /// Maximum HTLC expiry delta in blocks.
    pub max_htlc_expiry_delta: BlockHeight,

    /// Confirmation depth required for funding tx.
    pub funding_confirmations: u32,

    /// RPC endpoint for chain interaction.
    pub rpc_url: String,

    /// Poll interval in seconds for chain monitoring.
    pub poll_interval_secs: u64,

    /// Path to the channel store file.
    pub store_path: String,
}

impl ChannelConfig {
    /// Default configuration for mainnet.
    pub fn mainnet(rpc_url: &str) -> Self {
        Self {
            dispute_window: 144,           // ~24 min at 10s slots
            min_channel_capacity: 100_000, // 0.001 DOLI
            reserve_percent: 1,
            max_htlcs: 30,
            htlc_minimum: 1_000, // 0.00001 DOLI
            fee_rate: 1,
            max_htlc_expiry_delta: 2016, // ~5.6 hours
            funding_confirmations: 3,
            rpc_url: rpc_url.to_string(),
            poll_interval_secs: 10,
            store_path: "channels.json".to_string(),
        }
    }

    /// Default configuration for testnet (faster dispute windows).
    pub fn testnet(rpc_url: &str) -> Self {
        Self {
            dispute_window: 20, // ~3.3 min
            min_channel_capacity: 10_000,
            reserve_percent: 1,
            max_htlcs: 30,
            htlc_minimum: 100,
            fee_rate: 1,
            max_htlc_expiry_delta: 720, // ~2 hours
            funding_confirmations: 1,
            rpc_url: rpc_url.to_string(),
            poll_interval_secs: 5,
            store_path: "channels-testnet.json".to_string(),
        }
    }

    /// Calculate the reserve amount for a given channel capacity.
    pub fn reserve_for_capacity(&self, capacity: Amount) -> Amount {
        capacity * self.reserve_percent as u64 / 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_defaults() {
        let cfg = ChannelConfig::mainnet("http://localhost:8080");
        assert_eq!(cfg.dispute_window, 144);
        assert_eq!(cfg.funding_confirmations, 3);
        assert_eq!(cfg.rpc_url, "http://localhost:8080");
    }

    #[test]
    fn testnet_defaults() {
        let cfg = ChannelConfig::testnet("http://localhost:9090");
        assert_eq!(cfg.dispute_window, 20);
        assert_eq!(cfg.funding_confirmations, 1);
    }

    #[test]
    fn reserve_for_capacity() {
        let cfg = ChannelConfig::mainnet("http://localhost:8080");
        assert_eq!(cfg.reserve_for_capacity(1_000_000), 10_000); // 1%
        assert_eq!(cfg.reserve_for_capacity(100), 1);
        assert_eq!(cfg.reserve_for_capacity(0), 0);
    }

    #[test]
    fn config_serde_roundtrip() {
        let cfg = ChannelConfig::mainnet("http://localhost:8080");
        let json = serde_json::to_string(&cfg).unwrap();
        let decoded: ChannelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.dispute_window, cfg.dispute_window);
        assert_eq!(decoded.rpc_url, cfg.rpc_url);
    }
}
