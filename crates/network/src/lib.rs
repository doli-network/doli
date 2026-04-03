//! # doli-network
//!
//! P2P networking layer for the DOLI protocol.
//!
//! This crate implements the peer-to-peer network stack that allows DOLI nodes
//! to discover each other, propagate transactions, and synchronize blocks.
//!
//! ## Architecture
//!
//! The network layer is built on [libp2p](https://libp2p.io/) and provides:
//!
//! - **Peer Discovery**: Kademlia DHT for finding other nodes
//! - **Block Gossip**: Efficient block propagation using GossipSub
//! - **Transaction Pool**: Pending transaction broadcast
//! - **Block Sync**: Request/response protocol for chain synchronization
//!
//! ## Network Topology
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      DOLI Network                            │
//! │                                                              │
//! │  ┌──────────┐     ┌──────────┐     ┌──────────┐             │
//! │  │ Full Node│◄───►│ Full Node│◄───►│ Producer │             │
//! │  └────┬─────┘     └────┬─────┘     └────┬─────┘             │
//! │       │                │                │                    │
//! │       ▼                ▼                ▼                    │
//! │  ┌─────────────────────────────────────────────┐            │
//! │  │              GossipSub Mesh                  │            │
//! │  │  Topics: /doli/blocks, /doli/transactions   │            │
//! │  └─────────────────────────────────────────────┘            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Message Types
//!
//! | Message          | Direction | Description                        |
//! |------------------|-----------|------------------------------------|
//! | `NewBlock`       | Broadcast | Announce a newly produced block    |
//! | `NewTransaction` | Broadcast | Announce a pending transaction     |
//! | `GetBlocks`      | Request   | Request blocks by hash or height   |
//! | `Blocks`         | Response  | Return requested blocks            |
//! | `GetHeaders`     | Request   | Request block headers for sync     |
//! | `Headers`        | Response  | Return requested headers           |
//!
//! ## Configuration
//!
//! ```rust,no_run
//! use network::{NetworkConfig, NetworkService, DEFAULT_PORT};
//!
//! // Create network configuration
//! let config = NetworkConfig {
//!     listen_addr: "0.0.0.0:30300".parse().unwrap(),
//!     bootstrap_nodes: vec![
//!         "/ip4/seed1.doli.network/tcp/30300/p2p/12D3...".to_string(),
//!     ],
//!     max_peers: 50,
//!     ..Default::default()
//! };
//! ```
//!
//! ## Security
//!
//! - All connections use Noise protocol encryption
//! - Peer identities are Ed25519 public keys
//! - Rate limiting prevents DoS attacks
//! - Invalid messages result in peer scoring penalties

pub mod behaviour;
pub mod config;
pub mod discovery;
pub mod gossip;
pub mod messages;
pub mod nat;
pub mod peer;
pub mod peer_cache;
pub mod protocols;
pub mod rate_limit;
pub mod scoring;
pub mod service;
pub mod sync;
pub mod transport;

pub use config::NetworkConfig;
pub use discovery::discv5_service::{Discv5Config, Discv5Service};
pub use nat::{NatConfig, NatInfo, NatStatus};
pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use scoring::{Infraction, PeerScore, PeerScorer, PeerScorerConfig, ScorerStats};
pub use service::{NetworkCommand, NetworkError, NetworkEvent, NetworkService};
pub use sync::{
    EquivocationDetector, EquivocationProof, ForkAction, ProductionAuthorization, RecoveryPhase,
    RecoveryReason, ReorgResult, SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState,
    VerifiedSnapshot, MAX_CONSECUTIVE_RESYNCS,
};

// Re-export libp2p types that are part of our public API
pub use libp2p::identity::Keypair;
pub use libp2p::multiaddr;
pub use libp2p::request_response::ResponseChannel;
pub use libp2p::{Multiaddr, PeerId};

// Re-export discv5 Event for bootnode mode event handling
pub use discv5::Event as Discv5Event;

/// Extract PeerId from a multiaddr string like "/ip4/127.0.0.1/tcp/30300/p2p/12D3KooW..."
pub fn extract_peer_id_from_multiaddr(addr: &str) -> Option<PeerId> {
    let ma: Multiaddr = addr.parse().ok()?;
    ma.iter().find_map(|p| match p {
        libp2p::multiaddr::Protocol::P2p(pid) => Some(pid),
        _ => None,
    })
}

/// Default P2P port for DOLI nodes.
///
/// Nodes listen on this port by default for incoming peer connections.
pub const DEFAULT_PORT: u16 = 30300;

/// Protocol identifier for libp2p.
///
/// This string identifies the DOLI protocol version during
/// the multistream-select negotiation with peers.
pub const PROTOCOL_ID: &str = "/doli/1.0.0";
