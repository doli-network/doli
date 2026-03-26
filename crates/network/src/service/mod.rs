//! Network service implementation
//!
//! Manages the libp2p swarm and provides high-level network operations.
//!
//! ## Module structure
//!
//! - `types` — Public enums: `NetworkEvent`, `NetworkCommand`, `NetworkError`
//! - `helpers` — Address filtering, keypair persistence, multiaddr manipulation
//! - `swarm_loop` — Main tokio::select! event loop driving the swarm
//! - `swarm_events` — Swarm-level event handling (connections, listen addrs)
//! - `behaviour_events` — Behaviour-level event handling (gossip, identify, status, sync)
//! - `command_handling` — Network command dispatch (publish, dial, request-response)

mod behaviour_events;
mod command_handling;
mod helpers;
mod swarm_events;
mod swarm_loop;
mod types;

#[cfg(test)]
mod tests;

pub use types::{NetworkCommand, NetworkError, NetworkEvent};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use libp2p::{autonat, dcutr, relay, Multiaddr, PeerId, Swarm};
use tokio::sync::{mpsc, RwLock};
use tracing::info;

use doli_core::{ProducerAnnouncement, ProducerBloomFilter, Transaction};

use crate::behaviour::DoliBehaviour;
use crate::config::NetworkConfig;
use crate::discovery::new_kademlia;
use crate::gossip::{new_gossipsub, subscribe_to_topics};
use crate::peer::PeerInfo;
use crate::peer_cache::PeerCache;
use crate::protocols::{StatusRequest, StatusResponse, SyncRequest, SyncResponse, TxFetchResponse};
use crate::transport::build_transport;
use crypto::PublicKey;
use libp2p::request_response::ResponseChannel;

use helpers::{load_keypair, save_keypair, strip_p2p_suffix};
use swarm_loop::run_swarm;

/// Network service handle
pub struct NetworkService {
    /// Configuration
    #[allow(dead_code)]
    config: NetworkConfig,
    /// Connected peers
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    /// Event sender
    #[allow(dead_code)]
    event_tx: mpsc::Sender<NetworkEvent>,
    /// Event receiver
    event_rx: mpsc::Receiver<NetworkEvent>,
    /// Command sender
    command_tx: mpsc::Sender<NetworkCommand>,
    /// Local peer ID
    local_peer_id: PeerId,
}

impl NetworkService {
    /// Create a new network service
    pub async fn new(config: NetworkConfig) -> Result<Self, NetworkError> {
        // SCALE-T2-003: Channel buffers scale with max_peers.
        // At 5000 nodes, the old 1024-slot buffer fills in ~2 seconds causing
        // backpressure that blocks the swarm loop → cascade disconnections.
        // Formula: max(4096, max_peers * 16) for events, max(2048, max_peers * 4) for commands.
        // Capped at 32768 to bound memory (~1MB per channel at max capacity).
        let event_buf = (config.max_peers * 16).clamp(4096, 32768);
        let command_buf = (config.max_peers * 4).clamp(2048, 32768);
        let (event_tx, event_rx) = mpsc::channel(event_buf);
        let (command_tx, command_rx) = mpsc::channel(command_buf);

        let peers = Arc::new(RwLock::new(HashMap::new()));

        // Generate or load keypair
        let keypair = if let Some(ref key_path) = config.node_key_path {
            if key_path.exists() {
                load_keypair(key_path)?
            } else {
                let kp = libp2p::identity::Keypair::generate_ed25519();
                save_keypair(key_path, &kp)?;
                kp
            }
        } else {
            libp2p::identity::Keypair::generate_ed25519()
        };

        let local_peer_id = PeerId::from(keypair.public());
        info!("Local peer ID: {}", local_peer_id);

        // Create relay client (returns transport + behaviour pair)
        let (relay_transport, relay_client) = relay::client::new(local_peer_id);

        // Build transport WITH relay support for NAT traversal
        let transport = build_transport(&keypair, Some(relay_transport))
            .map_err(|e| NetworkError::Other(format!("Transport error: {}", e)))?;

        // Build gossipsub with mesh params from NetworkParams.
        // All topics are subscribed at startup. Topic subscriptions are
        // reconfigured at epoch boundaries when the node's tier is computed.
        let mesh = crate::gossip::MeshConfig {
            mesh_n: config.mesh_n,
            mesh_n_low: config.mesh_n_low,
            mesh_n_high: config.mesh_n_high,
            gossip_lazy: config.gossip_lazy,
        };
        let mut gossipsub = new_gossipsub(&keypair, &mesh)
            .map_err(|e| NetworkError::Other(format!("GossipSub error: {}", e)))?;
        subscribe_to_topics(&mut gossipsub)
            .map_err(|e| NetworkError::Other(format!("Subscribe error: {}", e)))?;

        // Build kademlia
        let kademlia = new_kademlia(local_peer_id);

        // Build connection limits (2 connections per peer, max_peers + bootstrap headroom).
        // Allow 2 per-peer to survive the simultaneous-dial race condition
        // (rust-libp2p#752): when both sides dial each other at the same time,
        // both connections complete the handshake. With limit=1 the second is
        // denied → rapid connect/disconnect loop on co-located nodes.
        // Also required for DCUtR hole-punching (relay + direct coexist briefly).
        //
        // INC-I-013: conn_limit = max_peers*2 + bootstrap_slots.
        // Old formula (mp+bs)*2+10 = 80/dir caused 100GB RAM at 156 nodes (1MB Yamux).
        // With 512KB Yamux: 300 nodes × 60/dir × 2 × 1.5MB = 54GB burst peak.
        // Steady state governed by max_peers eviction, not conn_limit.
        // Must be high enough for burst startup (N nodes dial seed simultaneously)
        // to avoid libp2p internal dial_backoff permanently isolating rejected peers.
        let conn_limit = std::env::var("DOLI_CONN_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or((config.max_peers * 2 + config.bootstrap_slots) as u32);
        let limits = libp2p::connection_limits::ConnectionLimits::default()
            .with_max_established_per_peer(Some(2))
            .with_max_established_incoming(Some(conn_limit))
            .with_max_established_outgoing(Some(conn_limit));
        let connection_limits = libp2p::connection_limits::Behaviour::new(limits);

        // Build relay server — all nodes instantiate it, but reservation limits
        // control whether it actively serves relay circuits.
        let relay_server_config = if config.nat_config.enable_relay_server {
            relay::Config {
                max_reservations: config.nat_config.max_relay_reservations as usize,
                max_circuits_per_peer: config.nat_config.max_circuits_per_peer as usize,
                ..Default::default()
            }
        } else {
            // Disabled: zero reservations means no peers can reserve
            relay::Config {
                max_reservations: 0,
                max_circuits_per_peer: 0,
                ..Default::default()
            }
        };
        let relay_server = relay::Behaviour::new(local_peer_id, relay_server_config);

        // DCUtR — hole punching via relay-coordinated upgrade
        let dcutr = dcutr::Behaviour::new(local_peer_id);

        // AutoNAT — detects whether we're behind NAT
        let autonat_config = autonat::Config {
            boot_delay: std::time::Duration::from_secs(15),
            refresh_interval: std::time::Duration::from_secs(60),
            ..Default::default()
        };
        let autonat = autonat::Behaviour::new(local_peer_id, autonat_config);

        // Build behaviour
        let behaviour = DoliBehaviour::new(
            gossipsub,
            kademlia,
            keypair.public(),
            connection_limits,
            relay_client,
            relay_server,
            dcutr,
            autonat,
        );

        // Build swarm
        let mut swarm = Swarm::new(
            transport,
            behaviour,
            local_peer_id,
            libp2p::swarm::Config::with_tokio_executor()
                .with_idle_connection_timeout(Duration::from_secs(86400)),
        );

        // Listen on configured address
        let listen_addr: Multiaddr = format!(
            "/ip4/{}/tcp/{}",
            config.listen_addr.ip(),
            config.listen_addr.port()
        )
        .parse()
        .map_err(|e| NetworkError::Other(format!("Invalid listen address: {}", e)))?;

        swarm
            .listen_on(listen_addr.clone())
            .map_err(|_| NetworkError::BindError)?;

        // Register explicit external address so Identify advertises it instead of
        // non-routable local addresses (e.g., 127.0.0.1 on multi-node hosts).
        if let Some(ref ext_addr) = config.external_address {
            swarm.add_external_address(ext_addr.clone());
            info!("Registered external address: {}", ext_addr);
        }

        info!("Network service listening on {}", listen_addr);

        // Load cached peers and dial them (in addition to bootstrap nodes).
        // Strip embedded /p2p/<id> suffixes — peer IDs change after chain resets,
        // so we dial by address only and accept whatever peer ID answers.
        if let Some(ref cache_path) = config.peer_cache_path {
            if let Some(cache) = PeerCache::load(cache_path) {
                let addrs = cache.addresses();
                if !addrs.is_empty() {
                    info!(
                        "Dialing {} cached peers from {}",
                        addrs.len(),
                        cache_path.display()
                    );
                    for addr in addrs {
                        let clean = strip_p2p_suffix(&addr);
                        let _ = swarm.dial(clean);
                    }
                }
            }
        }

        // Spawn the swarm event loop
        let peers_clone = peers.clone();
        let event_tx_clone = event_tx.clone();
        let config_clone = config.clone();
        let peer_cache_path = config.peer_cache_path.clone();

        tokio::spawn(async move {
            run_swarm(
                swarm,
                command_rx,
                event_tx_clone,
                peers_clone,
                config_clone,
                peer_cache_path,
            )
            .await;
        });

        // Always connect to bootstrap nodes — even if cached peers exist.
        // Cached peers may be stale or on a minority fork after a rolling restart.
        // Bootstrap/seed nodes are the authoritative network entry points.
        for addr in &config.bootstrap_nodes {
            if let Ok(multiaddr) = addr.parse::<Multiaddr>() {
                let _ = command_tx.send(NetworkCommand::Connect(multiaddr)).await;
            }
        }

        Ok(Self {
            config,
            peers,
            event_tx,
            event_rx,
            command_tx,
            local_peer_id,
        })
    }

    /// Get the next network event
    pub async fn next_event(&mut self) -> Option<NetworkEvent> {
        self.event_rx.recv().await
    }

    /// Try to get a network event without blocking
    ///
    /// Returns None immediately if no event is available.
    /// This is used to drain pending block events before production
    /// to ensure the chain tip is up-to-date before starting VDF computation.
    pub fn try_next_event(&mut self) -> Option<NetworkEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Broadcast a block to the network
    pub async fn broadcast_block(&self, block: doli_core::Block) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastBlock(block))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Broadcast a block header to the network (lightweight pre-announcement)
    pub async fn broadcast_header(
        &self,
        header: doli_core::BlockHeader,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastHeader(header))
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

    /// Broadcast a transaction to the network
    pub async fn broadcast_transaction(&self, tx: Transaction) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastTransaction(tx))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Broadcast producer list to the network (anti-entropy gossip) - legacy format
    /// This broadcasts the full view of known producers for CRDT merge
    /// DEPRECATED: Use broadcast_producer_announcements instead
    pub async fn broadcast_producers(&self, pubkeys: Vec<PublicKey>) -> Result<(), NetworkError> {
        let data = bincode::serialize(&pubkeys).map_err(|e| {
            NetworkError::Other(format!("Failed to serialize producer list: {}", e))
        })?;
        self.command_tx
            .send(NetworkCommand::BroadcastProducers(data))
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

    /// Broadcast a heartbeat to the network.
    ///
    /// NOTE: Deprecated in deterministic scheduler model. Heartbeats are no longer
    /// used for presence tracking. Rewards go 100% to block producer via coinbase.
    /// This function is kept for API compatibility but does nothing.
    #[deprecated(note = "Heartbeats removed in deterministic scheduler model")]
    pub async fn broadcast_heartbeat(&self, _heartbeat_data: Vec<u8>) -> Result<(), NetworkError> {
        // No-op: heartbeats removed in deterministic scheduler model
        Ok(())
    }

    /// Send producer delta to a specific peer
    pub async fn send_producer_delta(
        &self,
        peer_id: PeerId,
        announcements: Vec<ProducerAnnouncement>,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::SendProducerDelta {
                peer_id,
                announcements,
            })
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

    /// Get peer info
    pub async fn get_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        self.peers.read().await.get(peer_id).cloned()
    }

    /// Get all peers
    pub async fn get_peers(&self) -> Vec<PeerInfo> {
        self.peers.read().await.values().cloned().collect()
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
