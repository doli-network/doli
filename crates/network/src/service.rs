//! Network service implementation
//!
//! Manages the libp2p swarm and provides high-level network operations.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic},
    identify, kad,
    request_response::{self, ResponseChannel},
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use doli_core::{
    decode_producer_set, encode_producer_set, is_legacy_bincode_format, Block,
    ProducerAnnouncement, ProducerBloomFilter, Transaction,
};

use crate::behaviour::{DoliBehaviour, DoliBehaviourEvent};
use crate::config::NetworkConfig;
use crate::discovery::new_kademlia;
use crate::gossip::{
    new_gossipsub, subscribe_to_topics, BLOCKS_TOPIC, HEARTBEATS_TOPIC, PRODUCERS_TOPIC,
    TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use crate::peer::PeerInfo;
use crate::protocols::{StatusRequest, StatusResponse, SyncRequest, SyncResponse};
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
}

/// Commands to the network
#[derive(Debug)]
pub enum NetworkCommand {
    /// Broadcast a block
    BroadcastBlock(Block),
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
}

/// Network service handle
pub struct NetworkService {
    /// Configuration
    config: NetworkConfig,
    /// Connected peers
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    /// Event sender
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

        // Build transport
        let transport = build_transport(&keypair)
            .map_err(|e| NetworkError::Other(format!("Transport error: {}", e)))?;

        // Build gossipsub
        let mut gossipsub = new_gossipsub(&keypair)
            .map_err(|e| NetworkError::Other(format!("GossipSub error: {}", e)))?;
        subscribe_to_topics(&mut gossipsub)
            .map_err(|e| NetworkError::Other(format!("Subscribe error: {}", e)))?;

        // Build kademlia
        let kademlia = new_kademlia(local_peer_id);

        // Build behaviour
        let behaviour = DoliBehaviour::new(gossipsub, kademlia, keypair.public());

        // Build swarm
        let mut swarm = Swarm::new(
            transport,
            behaviour,
            local_peer_id,
            libp2p::swarm::Config::with_tokio_executor()
                .with_idle_connection_timeout(Duration::from_secs(60)),
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

        info!("Network service listening on {}", listen_addr);

        // Spawn the swarm event loop
        let peers_clone = peers.clone();
        let event_tx_clone = event_tx.clone();
        let config_clone = config.clone();

        tokio::spawn(async move {
            run_swarm(swarm, command_rx, event_tx_clone, peers_clone, config_clone).await;
        });

        // Connect to bootstrap nodes
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
    pub async fn broadcast_block(&self, block: Block) -> Result<(), NetworkError> {
        self.command_tx
            .send(NetworkCommand::BroadcastBlock(block))
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

/// Run the swarm event loop
async fn run_swarm(
    mut swarm: Swarm<DoliBehaviour>,
    mut command_rx: mpsc::Receiver<NetworkCommand>,
    event_tx: mpsc::Sender<NetworkEvent>,
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: NetworkConfig,
) {
    loop {
        tokio::select! {
            // Handle swarm events
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm, &event_tx, &peers, &config).await;
            }

            // Handle commands
            Some(command) = command_rx.recv() => {
                handle_command(command, &mut swarm, &config).await;
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
) {
    match event {
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            info!("Connected to peer: {} via {:?}", peer_id, endpoint);

            // Add peer
            let mut peers = peers.write().await;
            if peers.len() < config.max_peers {
                let addr = endpoint.get_remote_address().to_string();
                peers.insert(peer_id, PeerInfo::new(peer_id.to_string(), addr));

                let _ = event_tx.send(NetworkEvent::PeerConnected(peer_id)).await;
            }
        }

        SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
            info!("Disconnected from peer: {} cause: {:?}", peer_id, cause);

            let mut peers = peers.write().await;
            peers.remove(&peer_id);

            let _ = event_tx.send(NetworkEvent::PeerDisconnected(peer_id)).await;
        }

        SwarmEvent::Behaviour(behaviour_event) => {
            handle_behaviour_event(behaviour_event, swarm, event_tx, peers, config).await;
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
) {
    match event {
        DoliBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message_id: _,
            message,
        }) => {
            let topic = message.topic.as_str();
            debug!(
                "Received gossip message on topic {} from {}",
                topic, propagation_source
            );

            match topic {
                BLOCKS_TOPIC => {
                    if let Some(block) = Block::deserialize(&message.data) {
                        let _ = event_tx.send(NetworkEvent::NewBlock(block)).await;
                    } else {
                        warn!("Failed to deserialize block from {}", propagation_source);
                    }
                }
                TRANSACTIONS_TOPIC => {
                    if let Some(tx) = Transaction::deserialize(&message.data) {
                        let _ = event_tx.send(NetworkEvent::NewTransaction(tx)).await;
                    } else {
                        warn!(
                            "Failed to deserialize transaction from {}",
                            propagation_source
                        );
                    }
                }
                PRODUCERS_TOPIC => {
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
                    // Forward raw vote data to the updater service
                    debug!(
                        "Received vote message ({} bytes) from {}",
                        message.data.len(),
                        propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewVote(message.data.clone()))
                        .await;
                }
                HEARTBEATS_TOPIC => {
                    // Forward raw heartbeat data for presence tracking
                    debug!(
                        "Received heartbeat ({} bytes) from {}",
                        message.data.len(),
                        propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewHeartbeat(message.data.clone()))
                        .await;
                }
                _ => {}
            }
        }

        DoliBehaviourEvent::Kademlia(kad::Event::RoutingUpdated { peer, .. }) => {
            debug!("Kademlia routing updated for peer: {}", peer);
        }

        DoliBehaviourEvent::Identify(identify::Event::Received { peer_id, info }) => {
            debug!(
                "Received identify info from {}: {:?}",
                peer_id, info.agent_version
            );

            // Add the peer's listen addresses to kademlia (unless DHT is disabled)
            if !config.no_dht {
                for addr in info.listen_addrs {
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
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
                    debug!("Received sync request from {}", peer);
                    let _ = event_tx
                        .send(NetworkEvent::SyncRequest {
                            peer_id: peer,
                            request,
                            channel,
                        })
                        .await;
                }
                request_response::Message::Response { response, .. } => {
                    debug!("Received sync response from {}", peer);
                    let _ = event_tx
                        .send(NetworkEvent::SyncResponse {
                            peer_id: peer,
                            response,
                        })
                        .await;
                }
            }
        }

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
            swarm.behaviour_mut().sync.send_request(&peer_id, request);
        }

        NetworkCommand::SendStatusResponse { channel, response } => {
            let _ = swarm
                .behaviour_mut()
                .status
                .send_response(channel, response);
        }

        NetworkCommand::SendSyncResponse { channel, response } => {
            let _ = swarm.behaviour_mut().sync.send_response(channel, response);
        }

        NetworkCommand::Connect(addr) => {
            if let Err(e) = swarm.dial(addr.clone()) {
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
}
