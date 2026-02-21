//! Composite network behaviour
//!
//! Combines all libp2p behaviours into a single network behaviour.

use libp2p::{
    autonat, connection_limits, dcutr,
    gossipsub::Behaviour as Gossipsub,
    identify::{self, Behaviour as Identify},
    kad::{store::MemoryStore, Behaviour as Kademlia},
    relay,
    request_response::{self, Behaviour as RequestResponse, ProtocolSupport},
    swarm::NetworkBehaviour,
    StreamProtocol,
};
use std::time::Duration;

use crate::protocols::{StatusCodec, SyncCodec};

/// Protocol version string
pub const PROTOCOL_VERSION: &str = "doli/1.0.0";

/// User agent string
pub const USER_AGENT: &str = concat!("doli-node/", env!("CARGO_PKG_VERSION"));

/// Composite network behaviour for DOLI
#[derive(NetworkBehaviour)]
pub struct DoliBehaviour {
    /// Connection limits (checked before other behaviours)
    pub connection_limits: connection_limits::Behaviour,

    /// GossipSub for block and transaction propagation
    pub gossipsub: Gossipsub,

    /// Kademlia DHT for peer discovery
    pub kademlia: Kademlia<MemoryStore>,

    /// Identify protocol for peer info exchange
    pub identify: Identify,

    /// Request-response for status exchange
    pub status: RequestResponse<StatusCodec>,

    /// Request-response for sync protocol
    pub sync: RequestResponse<SyncCodec>,

    /// Relay client — use relays for NAT traversal
    pub relay_client: relay::client::Behaviour,

    /// Relay server — act as a relay for other nodes (bootstrap/public nodes)
    pub relay_server: relay::Behaviour,

    /// DCUtR — direct connection upgrade through relay (hole punching)
    pub dcutr: dcutr::Behaviour,

    /// AutoNAT — automatic NAT status detection
    pub autonat: autonat::Behaviour,
}

impl DoliBehaviour {
    /// Create a new DOLI behaviour
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gossipsub: Gossipsub,
        kademlia: Kademlia<MemoryStore>,
        local_public_key: libp2p::identity::PublicKey,
        connection_limits: connection_limits::Behaviour,
        relay_client: relay::client::Behaviour,
        relay_server: relay::Behaviour,
        dcutr: dcutr::Behaviour,
        autonat: autonat::Behaviour,
    ) -> Self {
        // Identify behaviour configuration
        let identify = Identify::new(
            identify::Config::new(PROTOCOL_VERSION.to_string(), local_public_key)
                .with_agent_version(USER_AGENT.to_string()),
        );

        // Status request-response configuration
        let status = RequestResponse::new(
            [(
                StreamProtocol::new(crate::protocols::status::STATUS_PROTOCOL),
                ProtocolSupport::Full,
            )],
            request_response::Config::default().with_request_timeout(Duration::from_secs(30)),
        );

        // Sync request-response configuration
        let sync = RequestResponse::new(
            [(
                StreamProtocol::new(crate::protocols::sync::SYNC_PROTOCOL),
                ProtocolSupport::Full,
            )],
            request_response::Config::default().with_request_timeout(Duration::from_secs(120)),
        );

        Self {
            connection_limits,
            gossipsub,
            kademlia,
            identify,
            status,
            sync,
            relay_client,
            relay_server,
            dcutr,
            autonat,
        }
    }
}
