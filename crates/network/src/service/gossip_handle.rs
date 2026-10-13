//! Receive stamp of a gossip message, the origin a node verdict reports on
//! (INC-I-237 D1), and the gossip-event → topic mapping.

use std::time::Instant;

use libp2p::gossipsub::MessageId;
use libp2p::PeerId;

use crate::gossip::staleness::GossipTopic;

use super::types::NetworkEvent;

/// Receive stamp of one gossip message; a deferred message also carries the
/// topic, original message id and propagation source its verdict reports on.
#[derive(Debug, Clone)]
pub struct GossipHandle {
    pub recv_at: Instant,
    origin: Option<(GossipTopic, MessageId, PeerId)>,
}

impl GossipHandle {
    /// A handle whose verdict is reported on `msg_id` from `source`.
    pub fn new(topic: GossipTopic, msg_id: MessageId, source: PeerId) -> Self {
        Self {
            recv_at: Instant::now(),
            origin: Some((topic, msg_id, source)),
        }
    }

    /// An unstamped handle: a verdict on it is never reported nor counted.
    pub fn now() -> Self {
        Self {
            recv_at: Instant::now(),
            origin: None,
        }
    }

    pub(super) fn origin(&self) -> Option<&(GossipTopic, MessageId, PeerId)> {
        self.origin.as_ref()
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
