//! Public types for the network service layer.
//!
//! Contains the event, command, and error enums that form the network service API.

use libp2p::request_response::ResponseChannel;
use libp2p::{Multiaddr, PeerId};

use crypto::{Hash, PublicKey};
use doli_core::{Block, BlockHeader, ProducerAnnouncement, ProducerBloomFilter, Transaction};

use crate::protocols::{StatusRequest, StatusResponse, SyncRequest, SyncResponse, TxFetchResponse};

/// Genesis mismatch peers are silently rejected for this duration.
pub(super) const GENESIS_MISMATCH_COOLDOWN_SECS: u64 = 86400; // 24 hours

/// Events from the network
#[derive(Debug)]
pub enum NetworkEvent {
    /// New peer connected
    PeerConnected(PeerId),
    /// Peer disconnected
    PeerDisconnected(PeerId),
    /// New block received via gossip (block, propagation source peer)
    NewBlock(Block, PeerId),
    /// New block header received via gossip (lightweight pre-announcement)
    NewHeader(BlockHeader),
    /// New transaction received via gossip
    NewTransaction(Transaction),
    /// Status request received
    StatusRequest {
        peer_id: PeerId,
        request: StatusRequest,
        channel: ResponseChannel<StatusResponse>,
    },
    /// Sync request received
    SyncRequest {
        peer_id: PeerId,
        request: SyncRequest,
        channel: ResponseChannel<SyncResponse>,
    },
    /// Sync response received
    SyncResponse {
        peer_id: PeerId,
        response: SyncResponse,
    },
    /// Peer status received
    PeerStatus {
        peer_id: PeerId,
        status: StatusResponse,
    },
    /// Network mismatch detected - peer is on different network
    NetworkMismatch {
        peer_id: PeerId,
        our_network_id: u32,
        their_network_id: u32,
    },
    /// Genesis hash mismatch detected - peer is on different chain
    GenesisMismatch { peer_id: PeerId },
    /// Protocol version mismatch - peer is running incompatible version
    VersionMismatch {
        peer_id: PeerId,
        our_version: u32,
        their_version: u32,
    },
    /// Producers announced via anti-entropy gossip (bootstrap protocol)
    /// Contains the sender's full view of known producers for CRDT merge
    /// Legacy format - will be deprecated after network migration
    ProducersAnnounced(Vec<PublicKey>),
    /// Producer announcements received via gossip (new format)
    /// Contains cryptographically signed announcements with replay protection
    ProducerAnnouncementsReceived(Vec<ProducerAnnouncement>),
    /// Producer set digest received for delta sync
    /// Peer is requesting only the producers they don't know about
    ProducerDigestReceived {
        peer_id: PeerId,
        digest: ProducerBloomFilter,
    },
    /// Vote message received for governance veto system
    NewVote(Vec<u8>),
    /// Heartbeat received for weighted presence rewards
    NewHeartbeat(Vec<u8>),
    /// Attestation received for finality gadget
    NewAttestation(Vec<u8>),
    /// Transaction hash announcement received (announce-request pattern)
    TxAnnouncement { peer_id: PeerId, hashes: Vec<Hash> },
    /// Peer is requesting transactions by hash (announce-request pattern)
    TxFetchRequest {
        peer_id: PeerId,
        hashes: Vec<Hash>,
        channel: ResponseChannel<TxFetchResponse>,
    },
    /// Peer responded with requested transactions (announce-request pattern)
    TxFetchResponse {
        peer_id: PeerId,
        transactions: Vec<Transaction>,
    },
}

/// Commands to the network
#[derive(Debug)]
pub enum NetworkCommand {
    /// Broadcast a block
    BroadcastBlock(Block),
    /// Broadcast a block header (lightweight pre-announcement)
    BroadcastHeader(BlockHeader),
    /// Broadcast a transaction
    BroadcastTransaction(Transaction),
    /// Request status from a peer
    RequestStatus {
        peer_id: PeerId,
        request: StatusRequest,
    },
    /// Request sync from a peer
    RequestSync {
        peer_id: PeerId,
        request: SyncRequest,
    },
    /// Send status response
    SendStatusResponse {
        channel: ResponseChannel<StatusResponse>,
        response: StatusResponse,
    },
    /// Send sync response
    SendSyncResponse {
        channel: ResponseChannel<SyncResponse>,
        response: SyncResponse,
    },
    /// Connect to a peer
    Connect(Multiaddr),
    /// Disconnect from a peer
    Disconnect(PeerId),
    /// Bootstrap the DHT
    Bootstrap,
    /// Broadcast producer list (anti-entropy gossip) - legacy format
    BroadcastProducers(Vec<u8>),
    /// Broadcast producer announcements (new format with protobuf)
    BroadcastProducerAnnouncements(Vec<ProducerAnnouncement>),
    /// Broadcast producer set digest for delta sync
    BroadcastProducerDigest(ProducerBloomFilter),
    /// Send producer delta to a specific peer
    SendProducerDelta {
        peer_id: PeerId,
        announcements: Vec<ProducerAnnouncement>,
    },
    /// Broadcast a vote message (governance veto system)
    BroadcastVote(Vec<u8>),
    /// Broadcast a heartbeat (weighted presence rewards)
    BroadcastHeartbeat(Vec<u8>),
    /// Broadcast an attestation (finality gadget)
    BroadcastAttestation(Vec<u8>),
    /// Request transactions from a peer by hash (announce-request pattern)
    RequestTxFetch { peer_id: PeerId, hashes: Vec<Hash> },
    /// Send transaction fetch response (announce-request pattern)
    SendTxFetchResponse {
        channel: ResponseChannel<TxFetchResponse>,
        response: TxFetchResponse,
    },
}

/// Network errors
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("failed to bind to address")]
    BindError,

    #[error("connection failed: {0}")]
    ConnectionFailed(String),

    #[error("channel closed")]
    ChannelClosed,

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("network error: {0}")]
    Other(String),
}
