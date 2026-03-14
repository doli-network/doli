//! GossipSub for block and transaction propagation
//!
//! Implements efficient message propagation using libp2p GossipSub protocol.

mod config;
mod publish;

#[cfg(test)]
mod tests;

pub use config::*;
pub use publish::*;

/// GossipSub topic for new blocks
pub const BLOCKS_TOPIC: &str = "/doli/blocks/1";

/// GossipSub topic for new transactions
pub const TRANSACTIONS_TOPIC: &str = "/doli/txs/1";

/// GossipSub topic for producer announcements (bootstrap protocol)
pub const PRODUCERS_TOPIC: &str = "/doli/producers/1";

/// GossipSub topic for update votes (governance veto system)
pub const VOTES_TOPIC: &str = "/doli/votes/1";

/// GossipSub topic for presence heartbeats (weighted rewards)
pub const HEARTBEATS_TOPIC: &str = "/doli/heartbeats/1";

// ==================== Tiered Architecture Topics ====================

/// Tier 1 block propagation (dense mesh, validators only)
pub const TIER1_BLOCKS_TOPIC: &str = "/doli/t1/blocks/1";

/// Lightweight header topic (all tiers — for Tier 3 header-only validation)
pub const HEADERS_TOPIC: &str = "/doli/headers/1";

/// Attestation topic (Tier 1 + Tier 2 — for finality gadget)
pub const ATTESTATION_TOPIC: &str = "/doli/attestations/1";

/// Generate a regional block topic for Tier 2 sharding.
pub fn region_topic(region: u32) -> String {
    format!("/doli/r{}/blocks/1", region)
}

/// GossipSub errors
#[derive(Debug, thiserror::Error)]
pub enum GossipError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("initialization error: {0}")]
    Init(String),

    #[error("subscribe error: {0}")]
    Subscribe(String),

    #[error("publish error: {0}")]
    Publish(String),
}

/// Gossipsub mesh configuration
pub struct MeshConfig {
    pub mesh_n: usize,
    pub mesh_n_low: usize,
    pub mesh_n_high: usize,
    pub gossip_lazy: usize,
}
