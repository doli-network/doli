//! `NetworkService` accessors and the node-facing command senders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use doli_core::{ProducerAnnouncement, ProducerBloomFilter};
use libp2p::request_response::ResponseChannel;
use libp2p::{Multiaddr, PeerId};
use tokio::sync::{mpsc, RwLock};

use super::backpressure::GossipShedMetrics;
use super::{
    LaneBudgets, LaneStats, NetworkCommand, NetworkError, NetworkEvent, NetworkService, Retention,
};
use crate::peer::PeerInfo;
use crate::protocols::{StatusRequest, StatusResponse, SyncRequest, SyncResponse, TxFetchResponse};

impl NetworkService {
    /// INC-I-049: Update the best slot after applying a block.
    /// Read by the network rate limiter to exempt candidate-next blocks
    /// from per-peer rate limiting.
    pub fn update_best_slot(&self, slot: u32) {
        self.best_slot.store(slot, Ordering::Relaxed);
    }

    /// INC-I-114: Access gossip block load-shedding metrics.
    /// Used by M2 watchdog and RPC layer to monitor drop counts.
    pub fn gossip_shed_metrics(&self) -> Arc<GossipShedMetrics> {
        self.gossip_shed_metrics.clone()
    }

    /// The memory-watchdog shed flag the gossip block path reads.
    pub fn memory_shed_flag(&self) -> Arc<AtomicBool> {
        self.memory_shed_flag.clone()
    }
    /// Lane byte counters and drop counts (read by node metrics).
    pub fn lane_stats(&self) -> Arc<LaneStats> {
        self.lane_stats.clone()
    }

    /// Swarm cache sizes, refreshed by the swarm task (read by node metrics).
    pub fn retention(&self) -> Arc<Retention> {
        self.retention.clone()
    }

    /// Lane budgets this service runs with.
    pub fn lane_budgets(&self) -> LaneBudgets {
        self.lane_budgets
    }

    /// Get the next network event, consensus lane first
    pub async fn next_event(&mut self) -> Option<NetworkEvent> {
        self.lanes.recv().await
    }

    /// Next queued network event without blocking (drains blocks before production).
    pub fn try_next_event(&mut self) -> Option<NetworkEvent> {
        self.lanes.try_recv()
    }

    /// Broadcast a block to the network
    pub async fn broadcast_block(&self, block: doli_core::Block) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastBlock(block))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Broadcast an attestation to the network (finality gadget)
    pub async fn broadcast_attestation(&self, data: Vec<u8>) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastAttestation(data))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Broadcast producer announcements to the network (new format)
    /// Uses protobuf encoding for forward compatibility
    pub async fn broadcast_producer_announcements(
        &self,
        announcements: Vec<ProducerAnnouncement>,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastProducerAnnouncements(
                announcements,
            ))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Broadcast producer set digest for delta sync
    /// Peers will respond with only the producers not in the digest
    pub async fn broadcast_producer_digest(
        &self,
        digest: ProducerBloomFilter,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastProducerDigest(digest))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Request status from a peer
    pub async fn request_status(
        &self,
        peer_id: PeerId,
        request: StatusRequest,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::RequestStatus { peer_id, request })
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Request sync from a peer
    pub async fn request_sync(
        &self,
        peer_id: PeerId,
        request: SyncRequest,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::RequestSync { peer_id, request })
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Send status response
    pub async fn send_status_response(
        &self,
        channel: ResponseChannel<StatusResponse>,
        response: StatusResponse,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::SendStatusResponse { channel, response })
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Send sync response
    pub async fn send_sync_response(
        &self,
        channel: ResponseChannel<SyncResponse>,
        response: SyncResponse,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::SendSyncResponse { channel, response })
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Request transactions from a peer by hash (announce-request pattern)
    pub async fn request_tx_fetch(
        &self,
        peer_id: PeerId,
        hashes: Vec<crypto::Hash>,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::RequestTxFetch { peer_id, hashes })
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Send transaction fetch response (announce-request pattern)
    pub async fn send_tx_fetch_response(
        &self,
        channel: ResponseChannel<TxFetchResponse>,
        response: TxFetchResponse,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::SendTxFetchResponse { channel, response })
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Connect to a peer
    pub async fn connect(&self, address: &str) -> Result<(), NetworkError> {
        let multiaddr: Multiaddr = address
            .parse()
            .map_err(|_| NetworkError::ConnectionFailed(format!("Invalid address: {}", address)))?;
        self.command_tx
            .send(NetworkCommand::Connect(multiaddr))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Bootstrap the DHT
    pub async fn bootstrap(&self) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::Bootstrap)
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Get connected peer count
    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    /// Get local peer ID
    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    /// Get shared peers map for sync peer-count queries
    pub fn peers_arc(&self) -> Arc<RwLock<HashMap<PeerId, PeerInfo>>> {
        self.peers.clone()
    }

    /// Disconnect from a specific peer
    pub async fn disconnect(&self, peer_id: PeerId) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::Disconnect(peer_id))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Get command sender for external use
    pub fn command_sender(&self) -> mpsc::Sender<NetworkCommand> {
        self.command_tx.clone()
    }
}
