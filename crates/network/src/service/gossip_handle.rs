//! Receive stamp of a gossip message and the gossip-event → topic mapping
//! (INC-I-237 M0). Observation only: nothing here steers a verdict.

use std::time::Instant;

use crate::gossip::staleness::GossipTopic;

use super::types::NetworkEvent;

/// Receive stamp of one gossip message, taken once by the swarm and cloned
/// into every event the message produces.
#[derive(Debug, Clone)]
pub struct GossipHandle {
    pub recv_at: Instant,
}

impl GossipHandle {
    pub fn now() -> Self {
        Self {
            recv_at: Instant::now(),
        }
    }
}

impl NetworkEvent {
    /// Gossip topic of a gossip event; `None` for control events.
    pub fn gossip_topic(&self) -> Option<GossipTopic> {
        match self {
            NetworkEvent::NewBlock(..) => Some(GossipTopic::Blocks),
            NetworkEvent::NewTransaction(..) | NetworkEvent::TxAnnouncement { .. } => {
                Some(GossipTopic::Transactions)
            }
            NetworkEvent::NewVote(..) => Some(GossipTopic::Votes),
            NetworkEvent::NewAttestation(..) => Some(GossipTopic::Attestations),
            NetworkEvent::ProducersAnnounced(..)
            | NetworkEvent::ProducerAnnouncementsReceived(..)
            | NetworkEvent::ProducerDigestReceived { .. } => Some(GossipTopic::Producers),
            NetworkEvent::PeerConnected(..)
            | NetworkEvent::PeerDisconnected(..)
            | NetworkEvent::StatusRequest { .. }
            | NetworkEvent::SyncRequest { .. }
            | NetworkEvent::SyncResponse { .. }
            | NetworkEvent::PeerStatus { .. }
            | NetworkEvent::NetworkMismatch { .. }
            | NetworkEvent::GenesisMismatch { .. }
            | NetworkEvent::VersionMismatch { .. }
            | NetworkEvent::TxFetchRequest { .. }
            | NetworkEvent::TxFetchResponse { .. } => None,
        }
    }

    /// Receive stamp of a gossip event; `None` for control events.
    pub fn gossip_handle(&self) -> Option<&GossipHandle> {
        match self {
            NetworkEvent::NewBlock(_, _, h)
            | NetworkEvent::NewTransaction(_, h)
            | NetworkEvent::TxAnnouncement { handle: h, .. }
            | NetworkEvent::NewVote(_, h)
            | NetworkEvent::NewAttestation(_, _, h)
            | NetworkEvent::ProducersAnnounced(_, h)
            | NetworkEvent::ProducerAnnouncementsReceived(_, h)
            | NetworkEvent::ProducerDigestReceived { handle: h, .. } => Some(h),
            NetworkEvent::PeerConnected(..)
            | NetworkEvent::PeerDisconnected(..)
            | NetworkEvent::StatusRequest { .. }
            | NetworkEvent::SyncRequest { .. }
            | NetworkEvent::SyncResponse { .. }
            | NetworkEvent::PeerStatus { .. }
            | NetworkEvent::NetworkMismatch { .. }
            | NetworkEvent::GenesisMismatch { .. }
            | NetworkEvent::VersionMismatch { .. }
            | NetworkEvent::TxFetchRequest { .. }
            | NetworkEvent::TxFetchResponse { .. } => None,
        }
    }
}

impl GossipTopic {
    /// Prometheus `topic` label value.
    pub fn metric_label(self) -> &'static str {
        match self {
            GossipTopic::Blocks => "blocks",
            GossipTopic::Producers => "producers",
            GossipTopic::Attestations => "attestations",
            GossipTopic::Votes => "votes",
            GossipTopic::Transactions => "txs",
        }
    }
}
