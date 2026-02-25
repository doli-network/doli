//! Network service implementation
//!
//! Manages the libp2p swarm and provides high-level network operations.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    autonat, dcutr,
    gossipsub::{self, IdentTopic},
    identify, kad,
    multiaddr::Protocol,
    relay,
    request_response::{self, ResponseChannel},
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use doli_core::{
    decode_producer_set, encode_producer_set, is_legacy_bincode_format, Block, BlockHeader,
    ProducerAnnouncement, ProducerBloomFilter, Transaction,
};

use crate::behaviour::{DoliBehaviour, DoliBehaviourEvent};
use crate::config::NetworkConfig;
use crate::discovery::new_kademlia;
use crate::gossip::{
    new_gossipsub, subscribe_to_topics, BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC,
    PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use crate::peer::PeerInfo;
use crate::peer_cache::PeerCache;
use crate::protocols::{StatusRequest, StatusResponse, SyncRequest, SyncResponse};
use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::transport::build_transport;
use crypto::PublicKey;

/// Events from the network
#[derive(Debug)]
pub enum NetworkEvent {
    /// New peer connected
    PeerConnected(PeerId),
    /// Peer disconnected
    PeerDisconnected(PeerId),
    /// New block received via gossip
    NewBlock(Block),
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
    /// Reconfigure gossipsub topic subscriptions for a new tier
    ReconfigureTier { tier: u8, region: Option<u32> },
}

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
        let (event_tx, event_rx) = mpsc::channel(1024);
        let (command_tx, command_rx) = mpsc::channel(1024);

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
        // All topics are subscribed to ensure full connectivity at startup.
        // Tier-aware mesh parameters (new_gossipsub_for_tier / subscribe_to_topics_for_tier)
        // are available for future hot-reconfiguration when the node's tier is computed
        // at the first epoch boundary after block sync.
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

        // Build connection limits (1 connection per peer, max_peers total)
        let limits = libp2p::connection_limits::ConnectionLimits::default()
            .with_max_established_per_peer(Some(1))
            .with_max_established_incoming(Some(config.max_peers as u32))
            .with_max_established_outgoing(Some(config.max_peers as u32));
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

        // Load cached peers and dial them before bootstrap
        let mut dialed_cached = false;
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
                        let _ = swarm.dial(addr);
                    }
                    dialed_cached = true;
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

        // Connect to bootstrap nodes (skip if we already dialed cached peers)
        if !dialed_cached {
            for addr in &config.bootstrap_nodes {
                if let Ok(multiaddr) = addr.parse::<Multiaddr>() {
                    let _ = command_tx.send(NetworkCommand::Connect(multiaddr)).await;
                }
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
    pub async fn broadcast_block(&self, block: Block) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastBlock(block))
            .await
            .map_err(|_| NetworkError::ChannelClosed)?;
        Ok(())
    }

    /// Broadcast a block header to the network (lightweight pre-announcement)
    pub async fn broadcast_header(&self, header: BlockHeader) -> Result<(), NetworkError> {
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

    /// Reconfigure gossipsub topic subscriptions for a new tier
    pub async fn reconfigure_tier(
        &self,
        tier: u8,
        region: Option<u32>,
    ) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::ReconfigureTier { tier, region })
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

    /// Get command sender for external use
    pub fn command_sender(&self) -> mpsc::Sender<NetworkCommand> {
        self.command_tx.clone()
    }
}

/// Check if a multiaddr contains only routable (non-local) IP addresses.
///
/// Filters out loopback (127.x), unspecified (0.0.0.0), and link-local (169.254.x)
/// addresses that should not be advertised to remote peers via Identify/Kademlia.
fn is_routable_address(addr: &Multiaddr) -> bool {
    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => {
                if ip.is_loopback() || ip.is_unspecified() || ip.is_link_local() {
                    return false;
                }
            }
            Protocol::Ip6(ip) => {
                if ip.is_loopback() || ip.is_unspecified() {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// Run the swarm event loop
async fn run_swarm(
    mut swarm: Swarm<DoliBehaviour>,
    mut command_rx: mpsc::Receiver<NetworkCommand>,
    event_tx: mpsc::Sender<NetworkEvent>,
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: NetworkConfig,
    peer_cache_path: Option<PathBuf>,
) {
    // Periodic DHT refresh: re-run Kademlia bootstrap every 60s so that peers
    // discover each other through shared bootstrap nodes. Without this, a node
    // only queries the DHT on initial connection (identify event), missing peers
    // that join later.
    let mut dht_refresh = tokio::time::interval(std::time::Duration::from_secs(60));
    dht_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Rate limiter: protects against excessive gossip from individual peers
    let mut rate_limiter = RateLimiter::new(RateLimitConfig::default());

    // Cleanup stale rate-limit entries every 5 minutes
    let mut rate_limit_cleanup = tokio::time::interval(std::time::Duration::from_secs(300));
    rate_limit_cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // TX batching: buffer outbound transactions and flush every 100ms
    let mut tx_batch: Vec<Transaction> = Vec::new();
    let mut tx_flush = tokio::time::interval(Duration::from_millis(100));
    tx_flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // Handle swarm events
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm, &event_tx, &peers, &config, &peer_cache_path, &mut rate_limiter).await;
            }

            // Handle commands — intercept BroadcastTransaction for batching
            Some(command) = command_rx.recv() => {
                if let NetworkCommand::BroadcastTransaction(tx) = command {
                    tx_batch.push(tx);
                    if tx_batch.len() >= 50 {
                        let batch = std::mem::take(&mut tx_batch);
                        let data = crate::gossip::encode_tx_batch(&batch);
                        let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            warn!("Failed to flush tx batch: {}", e);
                        }
                    }
                } else {
                    handle_command(command, &mut swarm, &config).await;
                }
            }

            // Flush buffered transactions every 100ms
            _ = tx_flush.tick() => {
                if !tx_batch.is_empty() {
                    let batch = std::mem::take(&mut tx_batch);
                    let data = crate::gossip::encode_tx_batch(&batch);
                    let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                        warn!("Failed to flush tx batch: {}", e);
                    }
                }
            }

            // Periodic DHT peer discovery
            _ = dht_refresh.tick() => {
                if !config.no_dht {
                    match swarm.behaviour_mut().kademlia.bootstrap() {
                        Ok(query_id) => {
                            info!("[DHT] Periodic bootstrap started (query={:?})", query_id);
                        }
                        Err(e) => {
                            warn!("[DHT] Bootstrap failed: {:?} — no peers in routing table", e);
                        }
                    }
                }
            }

            // Periodic rate limiter cleanup
            _ = rate_limit_cleanup.tick() => {
                rate_limiter.cleanup(Duration::from_secs(600));
            }
        }
    }
}

/// Handle swarm events
async fn handle_swarm_event(
    event: SwarmEvent<DoliBehaviourEvent>,
    swarm: &mut Swarm<DoliBehaviour>,
    event_tx: &mpsc::Sender<NetworkEvent>,
    peers: &Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: &NetworkConfig,
    peer_cache_path: &Option<PathBuf>,
    rate_limiter: &mut RateLimiter,
) {
    match event {
        SwarmEvent::ConnectionEstablished {
            peer_id,
            endpoint,
            num_established,
            ..
        } => {
            info!(
                "Connected to peer: {} via {:?} (num_established={})",
                peer_id, endpoint, num_established
            );

            // Only register peer on first connection (dedup)
            if num_established.get() == 1 {
                let mut peers = peers.write().await;
                if peers.len() < config.max_peers {
                    let addr = endpoint.get_remote_address().to_string();
                    peers.insert(peer_id, PeerInfo::new(peer_id.to_string(), addr));

                    let _ = event_tx.send(NetworkEvent::PeerConnected(peer_id)).await;
                }
            }
        }

        SwarmEvent::ConnectionClosed {
            peer_id,
            cause,
            num_established,
            ..
        } => {
            info!(
                "Connection closed to peer: {} cause: {:?} (num_established={})",
                peer_id, cause, num_established
            );

            // Only remove peer when no connections remain
            if num_established == 0 {
                let mut peers = peers.write().await;
                peers.remove(&peer_id);

                // Clean up rate limiter state for disconnected peer
                rate_limiter.remove_peer(&peer_id);

                let _ = event_tx.send(NetworkEvent::PeerDisconnected(peer_id)).await;
            }
        }

        SwarmEvent::Behaviour(behaviour_event) => {
            handle_behaviour_event(
                behaviour_event,
                swarm,
                event_tx,
                peers,
                config,
                peer_cache_path,
                rate_limiter,
            )
            .await;
        }

        SwarmEvent::NewListenAddr { address, .. } => {
            info!("Listening on: {}", address);
        }

        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            warn!("Failed to connect to peer {:?}: {}", peer_id, error);
        }

        _ => {}
    }
}

/// Handle behaviour events
async fn handle_behaviour_event(
    event: DoliBehaviourEvent,
    swarm: &mut Swarm<DoliBehaviour>,
    event_tx: &mpsc::Sender<NetworkEvent>,
    peers: &Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: &NetworkConfig,
    peer_cache_path: &Option<PathBuf>,
    rate_limiter: &mut RateLimiter,
) {
    match event {
        DoliBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message_id: _,
            message,
        }) => {
            let topic = message.topic.as_str();
            let msg_size = message.data.len();
            debug!(
                "Received gossip message on topic {} from {}",
                topic, propagation_source
            );

            match topic {
                BLOCKS_TOPIC => {
                    if !rate_limiter.check_block(&propagation_source) {
                        warn!(
                            "Rate limit: dropping block from {} (block rate exceeded)",
                            propagation_source
                        );
                        return;
                    }
                    if let Some(block) = Block::deserialize(&message.data) {
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let _ = event_tx.send(NetworkEvent::NewBlock(block)).await;
                    } else {
                        warn!("Failed to deserialize block from {}", propagation_source);
                    }
                }
                TRANSACTIONS_TOPIC => {
                    if !rate_limiter.check_transaction(&propagation_source) {
                        warn!(
                            "Rate limit: dropping tx from {} (tx rate exceeded)",
                            propagation_source
                        );
                        return;
                    }
                    if let Some(txs) = crate::gossip::decode_tx_message(&message.data) {
                        rate_limiter.record_transaction(&propagation_source, msg_size);
                        for tx in txs {
                            let _ = event_tx.send(NetworkEvent::NewTransaction(tx)).await;
                        }
                    } else {
                        warn!(
                            "Failed to deserialize transaction from {}",
                            propagation_source
                        );
                    }
                }
                PRODUCERS_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        warn!(
                            "Rate limit: dropping producer msg from {} (request rate exceeded)",
                            propagation_source
                        );
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);

                    // Try to detect format and decode appropriately
                    if is_legacy_bincode_format(&message.data) {
                        // Legacy bincode format: Vec<PublicKey>
                        match bincode::deserialize::<Vec<PublicKey>>(&message.data) {
                            Ok(pubkeys) => {
                                debug!(
                                    "Received legacy producer list ({} producers) from {}",
                                    pubkeys.len(),
                                    propagation_source
                                );
                                let _ = event_tx
                                    .send(NetworkEvent::ProducersAnnounced(pubkeys))
                                    .await;
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to deserialize legacy producer list from {}: {}",
                                    propagation_source, e
                                );
                            }
                        }
                    } else {
                        // New protobuf format: ProducerSet
                        match decode_producer_set(&message.data) {
                            Ok(announcements) => {
                                debug!(
                                    "Received producer announcements ({} producers) from {}",
                                    announcements.len(),
                                    propagation_source
                                );
                                let _ = event_tx
                                    .send(NetworkEvent::ProducerAnnouncementsReceived(
                                        announcements,
                                    ))
                                    .await;
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to decode producer announcements from {}: {}",
                                    propagation_source, e
                                );
                            }
                        }
                    }
                }
                VOTES_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);
                    debug!(
                        "Received vote message ({} bytes) from {}",
                        msg_size, propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewVote(message.data.clone()))
                        .await;
                }
                HEARTBEATS_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);
                    debug!(
                        "Received heartbeat ({} bytes) from {}",
                        msg_size, propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewHeartbeat(message.data.clone()))
                        .await;
                }
                topic if topic == crate::gossip::ATTESTATION_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);
                    debug!(
                        "Received attestation ({} bytes) from {}",
                        msg_size, propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewAttestation(message.data.clone()))
                        .await;
                }
                HEADERS_TOPIC => {
                    if !rate_limiter.check_block(&propagation_source) {
                        return;
                    }
                    if let Some(header) = BlockHeader::deserialize(&message.data) {
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let _ = event_tx.send(NetworkEvent::NewHeader(header)).await;
                    } else {
                        warn!("Failed to deserialize header from {}", propagation_source);
                    }
                }
                topic if topic == crate::gossip::TIER1_BLOCKS_TOPIC => {
                    // Tier 1 dense-mesh block: same payload as regular blocks
                    if !rate_limiter.check_block(&propagation_source) {
                        return;
                    }
                    if let Some(block) = Block::deserialize(&message.data) {
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let _ = event_tx.send(NetworkEvent::NewBlock(block)).await;
                    } else {
                        warn!("Failed to deserialize t1 block from {}", propagation_source);
                    }
                }
                topic if topic.starts_with("/doli/r") && topic.ends_with("/blocks/1") => {
                    // Regional block (Tier 2 sharding): same payload as regular blocks
                    if !rate_limiter.check_block(&propagation_source) {
                        return;
                    }
                    if let Some(block) = Block::deserialize(&message.data) {
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let _ = event_tx.send(NetworkEvent::NewBlock(block)).await;
                    }
                }
                _ => {}
            }
        }

        DoliBehaviourEvent::Kademlia(kad::Event::RoutingUpdated { peer, .. }) => {
            info!("[DHT] Routing updated for peer: {}", peer);
            // Dial the discovered peer to establish a connection
            if !swarm.is_connected(&peer) {
                info!("[DHT] Dialing discovered peer {}", peer);
                let _ = swarm.dial(peer);
            }
        }
        DoliBehaviourEvent::Kademlia(_) => {
            // Other Kademlia events (query progress, etc.) — no action needed
        }

        DoliBehaviourEvent::Identify(identify::Event::Received { peer_id, info }) => {
            debug!(
                "Received identify info from {}: {:?}",
                peer_id, info.agent_version
            );

            // Filter out non-routable addresses (loopback, unspecified, link-local)
            // so remote peers don't learn 127.0.0.1 from multi-node hosts.
            let routable_addrs: Vec<Multiaddr> = info
                .listen_addrs
                .into_iter()
                .filter(|addr| {
                    let routable = is_routable_address(addr);
                    if !routable {
                        debug!("Filtered non-routable address from {}: {}", peer_id, addr);
                    }
                    routable
                })
                .collect();

            // Cache the peer's routable addresses for fast reconnection after restart
            if let Some(ref path) = peer_cache_path {
                if let Some(addr) = routable_addrs.first() {
                    let full_addr = format!("{}/p2p/{}", addr, peer_id);
                    let mut cache = PeerCache::load(path).unwrap_or_default();
                    cache.add(&peer_id.to_string(), &full_addr);
                    cache.save(path);
                }
            }

            // Add the peer's routable addresses to kademlia (unless DHT is disabled)
            if !config.no_dht {
                let addr_count = routable_addrs.len();
                for addr in routable_addrs {
                    info!("[DHT] Adding address for peer {}: {}", peer_id, addr);
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }
                match swarm.behaviour_mut().kademlia.bootstrap() {
                    Ok(_) => info!(
                        "[DHT] Bootstrap triggered after identify ({} addrs from {})",
                        addr_count, peer_id
                    ),
                    Err(e) => warn!("[DHT] Bootstrap failed after identify: {:?}", e),
                }
            }
        }

        DoliBehaviourEvent::Status(request_response::Event::Message { peer, message }) => {
            match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    debug!("Received status request from {}", peer);

                    // Validate network ID on incoming request
                    if request.network_id != config.network_id {
                        warn!(
                            "Network mismatch with peer {}: we are on network {}, they are on {}",
                            peer, config.network_id, request.network_id
                        );
                        let _ = event_tx
                            .send(NetworkEvent::NetworkMismatch {
                                peer_id: peer,
                                our_network_id: config.network_id,
                                their_network_id: request.network_id,
                            })
                            .await;
                        // Disconnect the peer
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate genesis hash
                    if request.genesis_hash != config.genesis_hash {
                        warn!(
                            "Genesis hash mismatch with peer {}: different chain fork",
                            peer
                        );
                        let _ = event_tx
                            .send(NetworkEvent::GenesisMismatch { peer_id: peer })
                            .await;
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    let _ = event_tx
                        .send(NetworkEvent::StatusRequest {
                            peer_id: peer,
                            request,
                            channel,
                        })
                        .await;
                }
                request_response::Message::Response { response, .. } => {
                    debug!("Received status response from {}", peer);

                    // Validate network ID
                    if response.network_id != config.network_id {
                        warn!(
                            "Network mismatch with peer {}: we are on network {}, they are on {}",
                            peer, config.network_id, response.network_id
                        );
                        let _ = event_tx
                            .send(NetworkEvent::NetworkMismatch {
                                peer_id: peer,
                                our_network_id: config.network_id,
                                their_network_id: response.network_id,
                            })
                            .await;
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate genesis hash
                    if response.genesis_hash != config.genesis_hash {
                        warn!(
                            "Genesis hash mismatch with peer {}: different chain fork",
                            peer
                        );
                        let _ = event_tx
                            .send(NetworkEvent::GenesisMismatch { peer_id: peer })
                            .await;
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Update peer info
                    let mut peers = peers.write().await;
                    if let Some(peer_info) = peers.get_mut(&peer) {
                        peer_info.best_height = response.best_height;
                        peer_info.best_hash = response.best_hash;
                        peer_info.touch();
                    }

                    let _ = event_tx
                        .send(NetworkEvent::PeerStatus {
                            peer_id: peer,
                            status: response,
                        })
                        .await;
                }
            }
        }

        DoliBehaviourEvent::Sync(request_response::Event::Message { peer, message }) => {
            match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    info!(
                        "[SYNC_DEBUG] Received sync request from peer={}, request={:?}",
                        peer, request
                    );
                    let _ = event_tx
                        .send(NetworkEvent::SyncRequest {
                            peer_id: peer,
                            request,
                            channel,
                        })
                        .await;
                }
                request_response::Message::Response { response, .. } => {
                    info!(
                        "[SYNC_DEBUG] Received sync response from peer={}, response_type={}",
                        peer,
                        response.type_name()
                    );
                    let _ = event_tx
                        .send(NetworkEvent::SyncResponse {
                            peer_id: peer,
                            response,
                        })
                        .await;
                }
            }
        }

        DoliBehaviourEvent::RelayClient(event) => {
            info!("[RELAY] Client: {:?}", event);
        }

        DoliBehaviourEvent::RelayServer(event) => {
            info!("[RELAY] Server: {:?}", event);
        }

        DoliBehaviourEvent::Dcutr(event) => {
            info!("[DCUTR] {:?}", event);
        }

        DoliBehaviourEvent::Autonat(autonat::Event::StatusChanged { new, .. }) => match new {
            autonat::NatStatus::Public(addr) => {
                info!("[NAT] Public address detected: {}", addr);
            }
            autonat::NatStatus::Private => {
                warn!("[NAT] Behind NAT — relying on relay for connectivity");
            }
            autonat::NatStatus::Unknown => {
                debug!("[NAT] NAT status unknown, waiting for probes");
            }
        },
        DoliBehaviourEvent::Autonat(_) => {}

        _ => {}
    }
}

/// Handle network commands
async fn handle_command(
    command: NetworkCommand,
    swarm: &mut Swarm<DoliBehaviour>,
    config: &NetworkConfig,
) {
    match command {
        NetworkCommand::BroadcastBlock(block) => {
            let data = block.serialize();
            let topic = IdentTopic::new(BLOCKS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                warn!("Failed to broadcast block: {}", e);
            }
        }

        NetworkCommand::BroadcastHeader(header) => {
            let data = header.serialize();
            let topic = IdentTopic::new(HEADERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                warn!("Failed to broadcast header: {}", e);
            }
        }

        NetworkCommand::BroadcastTransaction(tx) => {
            let data = tx.serialize();
            let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                warn!("Failed to broadcast transaction: {}", e);
            }
        }

        NetworkCommand::RequestStatus { peer_id, request } => {
            swarm.behaviour_mut().status.send_request(&peer_id, request);
        }

        NetworkCommand::RequestSync { peer_id, request } => {
            info!(
                "[SYNC_DEBUG] Sending sync request to peer={}, request={:?}",
                peer_id, request
            );
            swarm.behaviour_mut().sync.send_request(&peer_id, request);
        }

        NetworkCommand::SendStatusResponse { channel, response } => {
            let _ = swarm
                .behaviour_mut()
                .status
                .send_response(channel, response);
        }

        NetworkCommand::SendSyncResponse { channel, response } => {
            info!(
                "[SYNC_DEBUG] Sending sync response via channel, response_type={}",
                response.type_name()
            );
            let _ = swarm.behaviour_mut().sync.send_response(channel, response);
        }

        NetworkCommand::Connect(addr) => {
            // Skip dial if we're already connected to this peer (defense-in-depth)
            let already_connected = addr
                .iter()
                .find_map(|proto| match proto {
                    libp2p::multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
                    _ => None,
                })
                .is_some_and(|peer_id| swarm.is_connected(&peer_id));

            if already_connected {
                debug!("Skipping dial to {} — already connected", addr);
            } else if let Err(e) = swarm.dial(addr.clone()) {
                warn!("Failed to dial {}: {}", addr, e);
            }
        }

        NetworkCommand::Disconnect(peer_id) => {
            let _ = swarm.disconnect_peer_id(peer_id);
        }

        NetworkCommand::Bootstrap => {
            if config.no_dht {
                debug!("DHT bootstrap skipped (--no-dht enabled)");
            } else if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
                warn!("Failed to bootstrap kademlia: {:?}", e);
            }
        }

        NetworkCommand::BroadcastProducers(data) => {
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                // Duplicate is expected if we broadcast the same list frequently
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast producer list: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastProducerAnnouncements(announcements) => {
            let data = encode_producer_set(&announcements);
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast producer announcements: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastProducerDigest(digest) => {
            // For now, we use gossipsub for digest broadcast
            // In the future, this could use a dedicated request-response protocol
            let data = doli_core::encode_digest(&digest);
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast producer digest: {}", e);
                }
            }
        }

        NetworkCommand::SendProducerDelta {
            peer_id,
            announcements,
        } => {
            // For direct peer communication, we use a direct gossip message
            // In the future, this could use request-response protocol
            let data = encode_producer_set(&announcements);
            debug!(
                "Sending producer delta ({} announcements) to {}",
                announcements.len(),
                peer_id
            );
            // Publish to gossip (all peers receive, but we log the intended target)
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to send producer delta to {}: {}", peer_id, e);
                }
            }
        }

        NetworkCommand::BroadcastVote(vote_data) => {
            let topic = IdentTopic::new(VOTES_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, vote_data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast vote: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastHeartbeat(heartbeat_data) => {
            let topic = IdentTopic::new(HEARTBEATS_TOPIC);
            if let Err(e) = swarm
                .behaviour_mut()
                .gossipsub
                .publish(topic, heartbeat_data)
            {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast heartbeat: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastAttestation(attestation_data) => {
            let topic = IdentTopic::new(crate::gossip::ATTESTATION_TOPIC);
            if let Err(e) = swarm
                .behaviour_mut()
                .gossipsub
                .publish(topic, attestation_data)
            {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast attestation: {}", e);
                }
            }
        }

        NetworkCommand::ReconfigureTier { tier, region } => {
            info!("Reconfiguring gossipsub topics for tier {}", tier);
            let gs = &mut swarm.behaviour_mut().gossipsub;
            if let Err(e) = crate::gossip::reconfigure_topics_for_tier(gs, tier, region) {
                warn!("Failed to reconfigure tier topics: {}", e);
            }
        }
    }
}

/// Load keypair from file
fn load_keypair(path: &std::path::Path) -> Result<libp2p::identity::Keypair, NetworkError> {
    let bytes = std::fs::read(path)
        .map_err(|e| NetworkError::Other(format!("Failed to read keypair: {}", e)))?;
    libp2p::identity::Keypair::from_protobuf_encoding(&bytes)
        .map_err(|e| NetworkError::Other(format!("Failed to decode keypair: {}", e)))
}

/// Save keypair to file
fn save_keypair(
    path: &std::path::Path,
    keypair: &libp2p::identity::Keypair,
) -> Result<(), NetworkError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NetworkError::Other(format!("Failed to create directory: {}", e)))?;
    }
    let bytes = keypair
        .to_protobuf_encoding()
        .map_err(|e| NetworkError::Other(format!("Failed to encode keypair: {}", e)))?;
    std::fs::write(path, bytes)
        .map_err(|e| NetworkError::Other(format!("Failed to write keypair: {}", e)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;
    use doli_core::{encode_producer_set, ProducerAnnouncement, ProducerBloomFilter};

    #[test]
    fn test_network_event_announcement_type() {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let event = NetworkEvent::ProducerAnnouncementsReceived(vec![ann.clone()]);

        if let NetworkEvent::ProducerAnnouncementsReceived(anns) = event {
            assert_eq!(anns.len(), 1);
            assert!(anns[0].verify());
        } else {
            panic!("Wrong event type");
        }
    }

    #[test]
    fn test_network_event_legacy_producers() {
        let keypair = KeyPair::generate();
        let pubkey = *keypair.public_key();
        let event = NetworkEvent::ProducersAnnounced(vec![pubkey]);

        if let NetworkEvent::ProducersAnnounced(pubkeys) = event {
            assert_eq!(pubkeys.len(), 1);
            assert_eq!(pubkeys[0], pubkey);
        } else {
            panic!("Wrong event type");
        }
    }

    #[test]
    fn test_network_event_digest_received() {
        let bloom = ProducerBloomFilter::new(100);
        let peer_id = PeerId::random();
        let event = NetworkEvent::ProducerDigestReceived {
            peer_id,
            digest: bloom.clone(),
        };

        if let NetworkEvent::ProducerDigestReceived {
            peer_id: pid,
            digest,
        } = event
        {
            assert_eq!(pid, peer_id);
            assert_eq!(digest.element_count(), 0);
        } else {
            panic!("Wrong event type");
        }
    }

    #[test]
    fn test_network_command_broadcast_announcements() {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let command = NetworkCommand::BroadcastProducerAnnouncements(vec![ann.clone()]);

        if let NetworkCommand::BroadcastProducerAnnouncements(anns) = command {
            assert_eq!(anns.len(), 1);
            assert!(anns[0].verify());
        } else {
            panic!("Wrong command type");
        }
    }

    #[test]
    fn test_network_command_broadcast_digest() {
        let bloom = ProducerBloomFilter::new(100);
        let command = NetworkCommand::BroadcastProducerDigest(bloom.clone());

        if let NetworkCommand::BroadcastProducerDigest(digest) = command {
            assert_eq!(digest.element_count(), 0);
        } else {
            panic!("Wrong command type");
        }
    }

    #[test]
    fn test_network_command_send_delta() {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let peer_id = PeerId::random();
        let command = NetworkCommand::SendProducerDelta {
            peer_id,
            announcements: vec![ann.clone()],
        };

        if let NetworkCommand::SendProducerDelta {
            peer_id: pid,
            announcements,
        } = command
        {
            assert_eq!(pid, peer_id);
            assert_eq!(announcements.len(), 1);
        } else {
            panic!("Wrong command type");
        }
    }

    #[test]
    fn test_gossip_message_encoding() {
        let keypair = KeyPair::generate();
        let anns = vec![ProducerAnnouncement::new(&keypair, 1, 0)];
        let bytes = encode_producer_set(&anns);

        // Should be reasonable size: ~130 bytes for single announcement
        assert!(
            bytes.len() < 200,
            "Single announcement {} bytes, expected < 200",
            bytes.len()
        );
    }

    #[test]
    fn test_format_detection() {
        // Legacy bincode format
        let keypair = KeyPair::generate();
        let pubkeys = vec![*keypair.public_key()];
        let bincode_bytes = bincode::serialize(&pubkeys).unwrap();
        assert!(is_legacy_bincode_format(&bincode_bytes));

        // New protobuf format
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let proto_bytes = encode_producer_set(&[ann]);
        assert!(!is_legacy_bincode_format(&proto_bytes));
    }

    #[test]
    fn test_is_routable_rejects_loopback() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/30303".parse().unwrap();
        assert!(!is_routable_address(&addr));

        let addr6: Multiaddr = "/ip6/::1/tcp/30303".parse().unwrap();
        assert!(!is_routable_address(&addr6));
    }

    #[test]
    fn test_is_routable_accepts_public() {
        let addr: Multiaddr = "/ip4/72.60.228.233/tcp/30303".parse().unwrap();
        assert!(is_routable_address(&addr));

        let addr2: Multiaddr = "/ip4/147.93.84.44/tcp/30303".parse().unwrap();
        assert!(is_routable_address(&addr2));
    }

    #[test]
    fn test_is_routable_rejects_unspecified() {
        let addr: Multiaddr = "/ip4/0.0.0.0/tcp/30303".parse().unwrap();
        assert!(!is_routable_address(&addr));

        let addr6: Multiaddr = "/ip6/::/tcp/30303".parse().unwrap();
        assert!(!is_routable_address(&addr6));
    }

    #[test]
    fn test_is_routable_rejects_link_local() {
        let addr: Multiaddr = "/ip4/169.254.1.1/tcp/30303".parse().unwrap();
        assert!(!is_routable_address(&addr));
    }
}
