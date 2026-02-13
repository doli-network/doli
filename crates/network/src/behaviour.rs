//! Composite network behaviour
//!
//! Combines all libp2p behaviours into a single network behaviour.

use libp2p::{
    connection_limits,
    gossipsub::{self, Behaviour as Gossipsub},
    identify::{self, Behaviour as Identify},
    kad::{self, store::MemoryStore, Behaviour as Kademlia},
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
}

impl DoliBehaviour {
    /// Create a new DOLI behaviour
    pub fn new(
        gossipsub: Gossipsub,
        kademlia: Kademlia<MemoryStore>,
        local_public_key: libp2p::identity::PublicKey,
        connection_limits: connection_limits::Behaviour,
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
        }
    }
}

/// Events from the network behaviour
#[derive(Debug)]
pub enum BehaviourEvent {
    /// GossipSub event
    Gossipsub(gossipsub::Event),
    /// Kademlia event
    Kademlia(kad::Event),
    /// Identify event
    Identify(identify::Event),
    /// Status request-response event
    Status(
        request_response::Event<crate::protocols::StatusRequest, crate::protocols::StatusResponse>,
    ),
    /// Sync request-response event
    Sync(request_response::Event<crate::protocols::SyncRequest, crate::protocols::SyncResponse>),
}

impl From<gossipsub::Event> for BehaviourEvent {
    fn from(event: gossipsub::Event) -> Self {
        BehaviourEvent::Gossipsub(event)
    }
}

impl From<kad::Event> for BehaviourEvent {
    fn from(event: kad::Event) -> Self {
        BehaviourEvent::Kademlia(event)
    }
}

impl From<identify::Event> for BehaviourEvent {
    fn from(event: identify::Event) -> Self {
        BehaviourEvent::Identify(event)
    }
}

impl
    From<request_response::Event<crate::protocols::StatusRequest, crate::protocols::StatusResponse>>
    for BehaviourEvent
{
    fn from(
        event: request_response::Event<
            crate::protocols::StatusRequest,
            crate::protocols::StatusResponse,
        >,
    ) -> Self {
        BehaviourEvent::Status(event)
    }
}

impl From<request_response::Event<crate::protocols::SyncRequest, crate::protocols::SyncResponse>>
    for BehaviourEvent
{
    fn from(
        event: request_response::Event<
            crate::protocols::SyncRequest,
            crate::protocols::SyncResponse,
        >,
    ) -> Self {
        BehaviourEvent::Sync(event)
    }
}
