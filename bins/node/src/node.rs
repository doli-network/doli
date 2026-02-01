//! Node implementation
//!
//! Integrates all DOLI components: storage, networking, RPC, mempool, and sync.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crypto::hash::{hash as crypto_hash, hash_concat, hash_with_domain};
use crypto::{Hash, KeyPair, PublicKey, ADDRESS_DOMAIN};
use doli_core::block::BlockBuilder;
use doli_core::consensus::{
    self, construct_vdf_input, select_producer_for_slot, ConsensusParams, UNBONDING_PERIOD,
};
use doli_core::tpop::calibration::VdfCalibrator;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::TxType;
use doli_core::types::UNITS_PER_COIN;
use doli_core::{
    AdaptiveGossip, Block, BlockHeader, MergeResult, Network, ProducerAnnouncement, ProducerGSet,
    Transaction,
};
use network::{
    EquivocationDetector, EquivocationProof, NetworkConfig, NetworkEvent, NetworkService,
    ReorgResult, SyncConfig, SyncManager,
};
use rpc::{Mempool, MempoolPolicy, RpcContext, RpcServer, RpcServerConfig, SyncStatus};
use storage::{BlockStore, ChainState, ClaimRecord, ClaimRegistry, ProducerSet, UtxoSet};
use updater::is_using_placeholder_keys;

use crate::updater as node_updater;
use vdf::{VdfOutput, VdfProof};

use crate::config::NodeConfig;
use crate::heartbeat_pool::HeartbeatPool;
use crate::producer::SignedSlotsDb;

/// The main node struct
pub struct Node {
    /// Configuration
    config: NodeConfig,
    /// Consensus parameters
    params: ConsensusParams,
    /// Block storage
    block_store: Arc<BlockStore>,
    /// UTXO set
    utxo_set: Arc<RwLock<UtxoSet>>,
    /// Chain state
    chain_state: Arc<RwLock<ChainState>>,
    /// Producer set
    producer_set: Arc<RwLock<ProducerSet>>,
    /// Mempool
    mempool: Arc<RwLock<Mempool>>,
    /// Network service
    network: Option<NetworkService>,
    /// Sync manager
    sync_manager: Arc<RwLock<SyncManager>>,
    /// Shutdown flag
    shutdown: Arc<RwLock<bool>>,
    /// Producer key (if producing blocks)
    producer_key: Option<KeyPair>,
    /// Last slot we produced a block for (to avoid double-producing)
    last_produced_slot: u32,
    /// Last time we checked for production opportunity
    last_production_check: Instant,
    /// All known producers (persists across epochs for round-robin)
    known_producers: Arc<RwLock<Vec<PublicKey>>>,
    /// Time when we first connected to a peer (for discovery grace period)
    first_peer_connected: Option<Instant>,
    /// Equivocation detector for slashing double-signers
    equivocation_detector: Arc<RwLock<EquivocationDetector>>,
    /// VDF calibrator for dynamic iteration adjustment
    vdf_calibrator: Arc<RwLock<VdfCalibrator>>,
    /// Cache of blocks received during forks (for reorg execution)
    fork_block_cache: Arc<RwLock<HashMap<Hash, Block>>>,
    /// Last time we triggered a forced resync (cooldown to prevent loops)
    last_resync_time: Option<Instant>,
    /// Time when the bootstrap producer list last changed (for stability check)
    last_producer_list_change: Option<Instant>,
    /// Producer discovery CRDT with cryptographic announcements
    producer_gset: Arc<RwLock<ProducerGSet>>,
    /// Adaptive gossip controller for smart interval management
    adaptive_gossip: Arc<RwLock<AdaptiveGossip>>,
    /// Our current producer announcement (if we are a producer)
    our_announcement: Arc<RwLock<Option<ProducerAnnouncement>>>,
    /// Sequence number for our announcements (monotonically increasing)
    announcement_sequence: Arc<AtomicU64>,
    /// Signed slots database (prevents double-signing after restart)
    signed_slots_db: Option<SignedSlotsDb>,
    /// Blocks applied since last state save (for periodic persistence)
    blocks_since_save: u64,
    /// Heartbeat pool for collecting presence proofs
    heartbeat_pool: HeartbeatPool,
    /// Claim registry for tracking epoch reward claims
    claim_registry: Arc<RwLock<ClaimRegistry>>,
}

/// How often to save state (every N blocks applied)
const STATE_SAVE_INTERVAL: u64 = 10;

impl Node {
    /// Create a new node
    ///
    /// If `producer_set` is Some, uses the provided ProducerSet (shared with update service).
    /// Otherwise, loads from disk or creates a new one.
    ///
    /// If `signed_slots_db` is Some, uses it to prevent double-signing after restart.
    ///
    /// If `shutdown_flag` is Some, uses the provided flag for graceful shutdown signaling.
    /// Otherwise, creates a new flag internally.
    pub async fn new(
        config: NodeConfig,
        producer_key: Option<KeyPair>,
        producer_set: Option<Arc<RwLock<ProducerSet>>>,
        signed_slots_db: Option<SignedSlotsDb>,
        shutdown_flag: Option<Arc<RwLock<bool>>>,
    ) -> Result<Self> {
        let mut params = ConsensusParams::for_network(config.network);

        // Open storage
        let blocks_path = config.data_dir.join("blocks");
        let block_store = Arc::new(BlockStore::open(&blocks_path)?);

        // Load or create UTXO set
        let utxo_path = config.data_dir.join("utxo");
        let utxo_set = if utxo_path.exists() {
            UtxoSet::load(&utxo_path)?
        } else {
            UtxoSet::new()
        };
        let utxo_set = Arc::new(RwLock::new(utxo_set));

        // Load or create chain state
        let state_path = config.data_dir.join("chain_state.bin");
        let chain_state = if state_path.exists() {
            ChainState::load(&state_path)?
        } else {
            let genesis_hash = crypto::hash::hash(b"DOLI Genesis");
            ChainState::new(genesis_hash)
        };
        let genesis_hash = chain_state.genesis_hash;

        // For devnet, handle dynamic genesis_time
        // Use stored genesis_timestamp if available, otherwise set to current time
        if config.network == Network::Devnet && params.genesis_time == 0 {
            use std::time::{SystemTime, UNIX_EPOCH};
            if chain_state.genesis_timestamp != 0 {
                // Use stored genesis timestamp from previous run
                params.genesis_time = chain_state.genesis_timestamp;
                info!(
                    "Devnet genesis time loaded from state: {}",
                    params.genesis_time
                );
            } else {
                // New devnet - set genesis time to current timestamp (rounded to slot)
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                params.genesis_time = now - (now % params.slot_duration);
                info!("Devnet genesis time initialized: {}", params.genesis_time);
            }
        }

        let chain_state = Arc::new(RwLock::new(chain_state));

        // Load or create producer set (use provided one if available)
        let producer_set = if let Some(set) = producer_set {
            set
        } else {
            let producers_path = config.data_dir.join("producers.bin");
            let set = if producers_path.exists() {
                ProducerSet::load(&producers_path)?
            } else {
                // For testnet: initialize with genesis producers
                if config.network == Network::Testnet {
                    use doli_core::genesis::testnet_genesis_producers;
                    let genesis_producers = testnet_genesis_producers();
                    if !genesis_producers.is_empty() {
                        info!(
                            "Initializing testnet with {} genesis producers",
                            genesis_producers.len()
                        );
                        ProducerSet::with_genesis_producers(genesis_producers)
                    } else {
                        ProducerSet::new()
                    }
                } else {
                    ProducerSet::new()
                }
            };
            Arc::new(RwLock::new(set))
        };

        // Load or create claim registry
        let claims_path = config.data_dir.join("claims.bin");
        let claim_registry = ClaimRegistry::load(&claims_path)?;
        let claim_registry = Arc::new(RwLock::new(claim_registry));

        // Create mempool
        let mempool_policy = match config.network {
            Network::Mainnet => MempoolPolicy::mainnet(),
            Network::Testnet | Network::Devnet => MempoolPolicy::testnet(),
        };
        let mempool = Arc::new(RwLock::new(Mempool::new(
            mempool_policy,
            params.clone(),
            config.network,
        )));

        // Create sync manager
        let sync_config = SyncConfig::default();
        let sync_manager = Arc::new(RwLock::new(SyncManager::new(sync_config, genesis_hash)));

        if producer_key.is_some() {
            info!("Block production enabled");
        }

        // Create equivocation detector
        let equivocation_detector = Arc::new(RwLock::new(EquivocationDetector::new()));

        // Create VDF calibrator with network-specific target time
        // VDF is the BOTTLENECK in Proof of Time: ~80% of slot duration
        let vdf_target_ms = config.network.vdf_target_time_ms();
        let mut vdf_calibrator = VdfCalibrator::for_network(vdf_target_ms);
        if producer_key.is_some() {
            info!(
                "Calibrating VDF for {:?} (target: {}ms = {}s)...",
                config.network,
                vdf_target_ms,
                vdf_target_ms / 1000
            );
            let calibration_time = vdf_calibrator.calibrate_now();
            info!(
                "VDF calibrated: {} iterations (calibration took {:?})",
                vdf_calibrator.iterations(),
                calibration_time
            );
        }
        let vdf_calibrator = Arc::new(RwLock::new(vdf_calibrator));

        // Initialize producer discovery CRDT with persistence
        let gset_path = config.data_dir.join("producer_gset.bin");
        let network_id = config.network.id();
        let producer_gset = Arc::new(RwLock::new(ProducerGSet::new_with_persistence(
            network_id, gset_path,
        )));

        // Initialize adaptive gossip controller
        let adaptive_gossip = Arc::new(RwLock::new(AdaptiveGossip::new()));

        // Use provided shutdown flag or create a new one
        let shutdown = shutdown_flag.unwrap_or_else(|| Arc::new(RwLock::new(false)));

        Ok(Self {
            config,
            params,
            block_store,
            utxo_set,
            chain_state,
            producer_set,
            mempool,
            network: None,
            sync_manager,
            shutdown,
            producer_key,
            last_produced_slot: 0,
            last_production_check: Instant::now(),
            known_producers: Arc::new(RwLock::new(Vec::new())),
            first_peer_connected: None,
            equivocation_detector,
            vdf_calibrator,
            fork_block_cache: Arc::new(RwLock::new(HashMap::new())),
            last_resync_time: None,
            last_producer_list_change: None,
            producer_gset,
            adaptive_gossip,
            our_announcement: Arc::new(RwLock::new(None)),
            announcement_sequence: Arc::new(AtomicU64::new(0)),
            signed_slots_db,
            blocks_since_save: 0,
            heartbeat_pool: HeartbeatPool::new(),
            claim_registry,
        })
    }

    /// Run the node
    pub async fn run(&mut self) -> Result<()> {
        info!("Node starting...");

        // Check for placeholder maintainer keys on mainnet
        if self.config.network == Network::Mainnet && is_using_placeholder_keys() {
            error!("CRITICAL: Placeholder maintainer keys detected!");
            error!("This node is NOT suitable for mainnet operation.");
            error!("Replace MAINTAINER_KEYS in doli-updater/src/lib.rs with real Ed25519 keys.");
            return Err(anyhow::anyhow!(
                "Cannot start mainnet node with placeholder maintainer keys"
            ));
        } else if is_using_placeholder_keys() {
            warn!("Using placeholder maintainer keys - this is OK for testnet/devnet");
        }

        // Start network service
        self.start_network().await?;

        // BOOTSTRAP PRODUCER SELF-REGISTRATION: Register ourselves as a producer immediately.
        // This is critical for round-robin slot assignment - if we don't know about ourselves,
        // we might produce blocks in slots assigned to other producers.
        if let Some(ref key) = self.producer_key {
            if self.config.network == Network::Testnet || self.config.network == Network::Devnet {
                let our_pubkey = key.public_key().clone();
                let mut known = self.known_producers.write().await;
                if !known.contains(&our_pubkey) {
                    known.push(our_pubkey.clone());
                    known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                    let pubkey_hash = crypto_hash(our_pubkey.as_bytes());
                    info!(
                        "Registered self as bootstrap producer: {} (now {} known)",
                        &pubkey_hash.to_hex()[..16],
                        known.len()
                    );
                    // Mark producer list as changed for stability tracking
                    self.last_producer_list_change = Some(Instant::now());
                }
                let broadcast_list = known.clone();
                drop(known);

                // IMMEDIATE ANTI-ENTROPY: Broadcast our full view right away.
                // This is state-based gossip - we send our entire known producer list,
                // and receivers will merge it with their own list.
                if let Some(ref network) = self.network {
                    info!(
                        "Broadcasting initial anti-entropy producer list ({} producers)",
                        broadcast_list.len()
                    );
                    let _ = network.broadcast_producers(broadcast_list).await;
                }
            }
        }

        // Start RPC server if enabled
        if self.config.rpc.enabled {
            self.start_rpc().await?;
        }

        // Main event loop
        self.run_event_loop().await?;

        // Graceful shutdown: save all state before exiting
        self.shutdown().await?;

        Ok(())
    }

    /// Start the network service
    async fn start_network(&mut self) -> Result<()> {
        let listen_addr: SocketAddr = self.config.listen_addr.parse()?;
        let genesis_hash = self.chain_state.read().await.genesis_hash;

        let mut network_config = NetworkConfig::for_network(self.config.network, genesis_hash);
        network_config.listen_addr = listen_addr;
        network_config.bootstrap_nodes = self.config.bootstrap_nodes.clone();
        network_config.max_peers = self.config.max_peers;
        network_config.no_dht = self.config.no_dht;

        info!(
            "Starting network service on {} (network={}, id={})",
            listen_addr,
            self.config.network.name(),
            self.config.network.id()
        );
        let network = NetworkService::new(network_config).await?;
        self.network = Some(network);

        info!("Network service started");
        Ok(())
    }

    /// Start the RPC server
    async fn start_rpc(&self) -> Result<()> {
        let listen_addr: SocketAddr = self.config.rpc.listen_addr.parse()?;

        let rpc_config = RpcServerConfig {
            listen_addr,
            enable_cors: true,
            allowed_origins: vec!["*".to_string()],
        };

        // Create sync status callback
        let sync_manager_for_rpc = self.sync_manager.clone();
        let chain_state_for_rpc = self.chain_state.clone();
        let sync_status_fn = move || {
            // Get sync state synchronously by creating a small runtime
            // This is acceptable for RPC as it's called infrequently
            let sync_manager = sync_manager_for_rpc.clone();
            let chain_state = chain_state_for_rpc.clone();

            // Use try_read to avoid blocking if lock is held
            let is_syncing = match sync_manager.try_read() {
                Ok(guard) => guard.state().is_syncing(),
                Err(_) => false, // Default to not syncing if lock unavailable
            };

            // Calculate progress if syncing
            let progress = if is_syncing {
                match (sync_manager.try_read(), chain_state.try_read()) {
                    (Ok(sync_guard), Ok(chain_guard)) => {
                        let local_height = chain_guard.best_height;
                        let target_height = sync_guard.best_peer_height();
                        if target_height > 0 {
                            let progress = (local_height as f64 / target_height as f64) * 100.0;
                            Some(progress.min(100.0))
                        } else {
                            Some(0.0)
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };

            SyncStatus {
                is_syncing,
                progress,
            }
        };

        // Create RPC context with references to node state
        let context = RpcContext::new(
            self.chain_state.clone(),
            self.block_store.clone(),
            self.utxo_set.clone(),
            self.mempool.clone(),
            self.params.clone(),
        )
        .with_network(self.config.network.name().to_string())
        .with_blocks_per_reward_epoch(self.config.network.blocks_per_reward_epoch())
        .with_producer_set(self.producer_set.clone())
        .with_claim_registry(self.claim_registry.clone())
        .with_sync_status(sync_status_fn);

        let server = RpcServer::new(rpc_config, context);
        info!("Starting RPC server on {}", listen_addr);
        server.spawn();

        Ok(())
    }

    /// Main event loop
    async fn run_event_loop(&mut self) -> Result<()> {
        info!("Entering main event loop");

        // Check production opportunity every second
        let production_interval = Duration::from_secs(1);
        let mut production_timer = tokio::time::interval(production_interval);

        // Gossip our producer identity every 10 seconds
        // This ensures nodes that aren't directly connected (e.g., Node 2 -> Node 1 -> Node 3)
        // learn about each other through the GossipSub mesh relay
        let gossip_interval = Duration::from_secs(10);
        let mut gossip_timer = tokio::time::interval(gossip_interval);

        loop {
            // Check shutdown flag
            if *self.shutdown.read().await {
                break;
            }

            // Use select! to handle network events, production timer, and gossip timer
            tokio::select! {
                // Production timer tick
                _ = production_timer.tick() => {
                    // Check for block production opportunity
                    if self.producer_key.is_some() {
                        if let Err(e) = self.try_produce_block().await {
                            warn!("Block production error: {}", e);
                        }
                    }

                    // Run periodic tasks
                    if let Err(e) = self.run_periodic_tasks().await {
                        warn!("Periodic task error: {}", e);
                    }
                }

                // Gossip timer tick - ANTI-ENTROPY: broadcast our full producer view
                _ = gossip_timer.tick() => {
                    // On Devnet/Testnet, periodically broadcast our full known producer list.
                    // This is STATE-BASED gossip (not event-based) - we send our entire view,
                    // and receivers merge with their view using CRDT union semantics.
                    // This guarantees convergence: Union(Local, Remote) = eventual consistency.
                    if self.config.network == Network::Testnet || self.config.network == Network::Devnet {
                        if let Some(ref network) = self.network {
                            // NEW FORMAT: Create/update our producer announcement and broadcast via GSet
                            if let Some(ref key) = self.producer_key {
                                // Increment sequence number
                                let seq = self.announcement_sequence.fetch_add(1, Ordering::SeqCst);

                                // Create new announcement
                                let announcement = ProducerAnnouncement::new(key, self.config.network.id(), seq);

                                // Merge our own announcement into the GSet
                                {
                                    let mut gset = self.producer_gset.write().await;
                                    let _ = gset.merge_one(announcement.clone());
                                }

                                // Store as our current announcement
                                *self.our_announcement.write().await = Some(announcement);
                            }

                            // Export and broadcast all announcements (new format)
                            let announcements = {
                                let gset = self.producer_gset.read().await;
                                gset.export()
                            };

                            if !announcements.is_empty() {
                                debug!(
                                    "Broadcasting {} signed producer announcements",
                                    announcements.len()
                                );
                                let _ = network.broadcast_producer_announcements(announcements).await;
                            }

                            // LEGACY FORMAT: Also broadcast via old format for backwards compatibility
                            let known = self.known_producers.read().await;
                            let mut broadcast_list = known.clone();

                            // Always include ourselves in the broadcast
                            if let Some(ref key) = self.producer_key {
                                let our_pubkey = key.public_key();
                                if !broadcast_list.contains(our_pubkey) {
                                    broadcast_list.push(our_pubkey.clone());
                                }
                            }
                            drop(known);

                            if !broadcast_list.is_empty() {
                                // Sort for deterministic ordering before broadcast
                                broadcast_list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                                // DIAGNOSTIC: Log producer schedule view for convergence debugging
                                // All nodes should show identical lists once anti-entropy converges
                                let hashes: Vec<String> = broadcast_list.iter()
                                    .map(|p| crypto_hash(p.as_bytes()).to_hex()[..8].to_string())
                                    .collect();
                                info!(
                                    "Producer schedule view: {:?} (count={})",
                                    hashes, broadcast_list.len()
                                );

                                debug!("Running Anti-Entropy: Broadcasting {} producers via gossip (legacy)", broadcast_list.len());
                                let _ = network.broadcast_producers(broadcast_list).await;
                            }
                        }
                    }
                }

                // Network event received
                event = async {
                    if let Some(ref mut network) = self.network {
                        network.next_event().await
                    } else {
                        // If no network, just sleep forever
                        std::future::pending::<Option<NetworkEvent>>().await
                    }
                } => {
                    if let Some(event) = event {
                        self.handle_network_event(event).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle network events
    async fn handle_network_event(&mut self, event: NetworkEvent) -> Result<()> {
        match event {
            NetworkEvent::PeerConnected(peer_id) => {
                info!("Peer connected: {}", peer_id);

                // Track when we first connected to a peer (for bootstrap discovery grace period)
                if self.first_peer_connected.is_none() {
                    self.first_peer_connected = Some(Instant::now());
                    info!("First peer connected - starting discovery grace period");
                }

                // Request status from the new peer to learn their chain state
                // Include our producer pubkey so peers can discover us before blocks are exchanged
                let genesis_hash = self.chain_state.read().await.genesis_hash;
                let status_request = if let Some(ref key) = self.producer_key {
                    network::protocols::StatusRequest::with_producer(
                        self.config.network.id(),
                        genesis_hash,
                        key.public_key().clone(),
                    )
                } else {
                    network::protocols::StatusRequest::new(self.config.network.id(), genesis_hash)
                };

                if let Some(ref network) = self.network {
                    if let Err(e) = network.request_status(peer_id, status_request).await {
                        warn!("Failed to request status from peer {}: {}", peer_id, e);
                    } else {
                        debug!("Requested status from peer {}", peer_id);
                    }
                }
            }

            NetworkEvent::PeerDisconnected(peer_id) => {
                info!("Peer disconnected: {}", peer_id);
                self.sync_manager.write().await.remove_peer(&peer_id);
            }

            NetworkEvent::NewBlock(block) => {
                debug!("Received new block: {}", block.hash());
                // Refresh peer timestamps when we see network activity via gossip
                self.sync_manager.write().await.refresh_all_peers();
                self.handle_new_block(block).await?;
            }

            NetworkEvent::NewTransaction(tx) => {
                debug!("Received new transaction: {}", tx.hash());
                self.handle_new_transaction(tx).await?;
            }

            NetworkEvent::PeerStatus { peer_id, status } => {
                debug!(
                    "Peer {} status: height={}, slot={}",
                    peer_id, status.best_height, status.best_slot
                );
                self.sync_manager.write().await.add_peer(
                    peer_id,
                    status.best_height,
                    status.best_hash,
                    status.best_slot,
                );

                // BOOTSTRAP PRODUCER DISCOVERY: If the peer is a producer, add them to
                // known_producers. This allows nodes to discover each other
                // before any blocks are exchanged, solving the chicken-and-egg problem
                // where blocks can't be applied (they look like forks) because producers
                // don't know about each other.
                if let Some(ref producer_pubkey) = status.producer_pubkey {
                    if self.config.network == Network::Testnet
                        || self.config.network == Network::Devnet
                    {
                        let mut known = self.known_producers.write().await;
                        if !known.contains(producer_pubkey) {
                            known.push(producer_pubkey.clone());
                            // Keep sorted for deterministic ordering
                            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                            let pubkey_hash = crypto_hash(producer_pubkey.as_bytes());
                            info!(
                                "Bootstrap producer discovered via status: {} (now {} known)",
                                &pubkey_hash.to_hex()[..16],
                                known.len()
                            );
                            drop(known);
                            // Reset stability timer - new producer discovered
                            self.last_producer_list_change = Some(Instant::now());
                        }
                    }
                }
            }

            NetworkEvent::StatusRequest {
                peer_id,
                channel,
                request,
            } => {
                debug!("Status request from {}", peer_id);

                // BOOTSTRAP PRODUCER DISCOVERY: If the requesting peer is a producer,
                // add them to known_producers (same as we do for PeerStatus)
                if let Some(ref producer_pubkey) = request.producer_pubkey {
                    if self.config.network == Network::Testnet
                        || self.config.network == Network::Devnet
                    {
                        let mut known = self.known_producers.write().await;
                        if !known.contains(producer_pubkey) {
                            known.push(producer_pubkey.clone());
                            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                            let pubkey_hash = crypto_hash(producer_pubkey.as_bytes());
                            info!("Bootstrap producer discovered via status request: {} (now {} known)",
                                  &pubkey_hash.to_hex()[..16],
                                  known.len());
                            drop(known);
                            // Reset stability timer - new producer discovered
                            self.last_producer_list_change = Some(Instant::now());
                        }
                    }
                }

                let state = self.chain_state.read().await;
                let response = if let Some(ref key) = self.producer_key {
                    network::protocols::StatusResponse {
                        version: 1,
                        network_id: self.config.network.id(),
                        genesis_hash: state.genesis_hash,
                        best_height: state.best_height,
                        best_hash: state.best_hash,
                        best_slot: state.best_slot,
                        producer_pubkey: Some(key.public_key().clone()),
                    }
                } else {
                    network::protocols::StatusResponse {
                        version: 1,
                        network_id: self.config.network.id(),
                        genesis_hash: state.genesis_hash,
                        best_height: state.best_height,
                        best_hash: state.best_hash,
                        best_slot: state.best_slot,
                        producer_pubkey: None,
                    }
                };

                if let Some(ref network) = self.network {
                    let _ = network.send_status_response(channel, response).await;
                }
            }

            NetworkEvent::SyncRequest {
                peer_id,
                request,
                channel,
            } => {
                debug!("Sync request from {}: {:?}", peer_id, request);
                self.handle_sync_request(request, channel).await?;
            }

            NetworkEvent::SyncResponse { peer_id, response } => {
                debug!("Sync response from {}", peer_id);
                let blocks = self
                    .sync_manager
                    .write()
                    .await
                    .handle_response(peer_id, response);
                for block in blocks {
                    self.apply_block(block).await?;
                }
            }

            NetworkEvent::NetworkMismatch {
                peer_id,
                our_network_id,
                their_network_id,
            } => {
                warn!(
                    "Disconnected peer {} due to network mismatch: we are on network {}, they are on {}",
                    peer_id, our_network_id, their_network_id
                );
                self.sync_manager.write().await.remove_peer(&peer_id);
            }

            NetworkEvent::GenesisMismatch { peer_id } => {
                warn!(
                    "Disconnected peer {} due to genesis hash mismatch (different chain fork)",
                    peer_id
                );
                self.sync_manager.write().await.remove_peer(&peer_id);
            }

            NetworkEvent::ProducersAnnounced(remote_list) => {
                // LEGACY ANTI-ENTROPY GOSSIP: Merge remote producer list with our local list
                // This is STATE-BASED (not event-based) - we receive the sender's full view
                // and merge using CRDT union semantics: Union(Local, Remote)
                // This guarantees convergence even with packet loss or network partitions.
                if self.config.network == Network::Testnet || self.config.network == Network::Devnet
                {
                    let changed = {
                        let mut known = self.known_producers.write().await;
                        let mut changed = false;

                        // CRDT MERGE: Add any producers we don't already know about
                        for producer in &remote_list {
                            if !known.contains(producer) {
                                known.push(producer.clone());
                                changed = true;
                                let pubkey_hash = crypto_hash(producer.as_bytes());
                                info!(
                                    "Bootstrap producer discovered via ANTI-ENTROPY: {} (now {} known)",
                                    &pubkey_hash.to_hex()[..16],
                                    known.len()
                                );
                            }
                        }

                        // Keep sorted for deterministic round-robin ordering
                        if changed {
                            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                            info!(
                                "Producer set updated via anti-entropy: {} total known producers",
                                known.len()
                            );
                        }
                        changed
                    };

                    // Mark stability timer reset (outside the lock)
                    if changed {
                        self.last_producer_list_change = Some(Instant::now());
                    }
                }
            }

            NetworkEvent::ProducerAnnouncementsReceived(announcements) => {
                // NEW PRODUCER DISCOVERY: Merge signed announcements into the GSet CRDT
                // Each announcement is cryptographically verified before merging
                let merge_result = {
                    let mut gset = self.producer_gset.write().await;
                    gset.merge(announcements)
                };

                // Update adaptive gossip controller with merge result
                let peer_count = self.sync_manager.read().await.peer_count();
                {
                    let mut gossip = self.adaptive_gossip.write().await;
                    gossip.on_gossip_result(&merge_result, peer_count);
                }

                // Log significant changes
                if merge_result.added > 0 {
                    info!(
                        "Producer announcements: added={}, new_producers={}, rejected={}, duplicates={}",
                        merge_result.added, merge_result.new_producers, merge_result.rejected, merge_result.duplicates
                    );
                    // Only reset stability timer when truly NEW producers are discovered
                    // Sequence updates (liveness proofs) should not reset stability
                    if merge_result.new_producers > 0 {
                        self.last_producer_list_change = Some(Instant::now());
                    }

                    // Also sync to legacy known_producers for compatibility
                    let gset = self.producer_gset.read().await;
                    let producers = gset.sorted_producers();
                    let mut known = self.known_producers.write().await;
                    for pubkey in producers {
                        if !known.contains(&pubkey) {
                            known.push(pubkey);
                        }
                    }
                    known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                } else if merge_result.rejected > 0 {
                    debug!(
                        "Producer announcements rejected: {} (invalid signature, stale, or wrong network)",
                        merge_result.rejected
                    );
                }
            }

            NetworkEvent::ProducerDigestReceived { peer_id, digest } => {
                // Peer sent us their bloom filter - compute delta and send missing announcements
                debug!(
                    "Received producer digest from {} ({} elements)",
                    peer_id,
                    digest.element_count()
                );

                // Get delta announcements (producers we know that peer doesn't)
                let delta = {
                    let gset = self.producer_gset.read().await;
                    gset.delta_for_peer(&digest)
                };

                if !delta.is_empty() {
                    debug!("Sending {} producers as delta to {}", delta.len(), peer_id);
                    if let Some(ref network) = self.network {
                        let _ = network.send_producer_delta(peer_id, delta).await;
                    }
                }
            }

            NetworkEvent::NewVote(vote_data) => {
                // Vote received via gossip - forward to updater service
                // The updater service will decode and validate the vote
                debug!("Received vote message ({} bytes)", vote_data.len());
                // TODO: Forward to updater service via vote_tx channel
                // This will be connected when the updater integration is complete
            }
        }

        Ok(())
    }

    /// Handle a new block from the network
    async fn handle_new_block(&mut self, block: Block) -> Result<()> {
        let block_hash = block.hash();

        // Check if we already have this block
        if self.block_store.get_block(&block_hash)?.is_some() {
            debug!("Block {} already known", block_hash);
            return Ok(());
        }

        // Check for equivocation (double signing) - even for forks
        // This is critical for slashing misbehaving producers
        let equivocation_proof = { self.equivocation_detector.write().await.check_block(&block) };
        if let Some(proof) = equivocation_proof {
            // Equivocation detected! Create and submit slash transaction
            self.handle_equivocation(proof).await;
        }

        // Check if block builds on our chain
        let state = self.chain_state.read().await;
        if block.header.prev_hash != state.best_hash {
            // Might be a reorg or we're out of sync
            let current_tip = state.best_hash;
            let current_height = state.best_height;
            drop(state);

            // Cache this block for potential reorg
            {
                let mut cache = self.fork_block_cache.write().await;
                cache.insert(block_hash, block.clone());
                // Keep cache size reasonable (last 100 fork blocks)
                // Evict oldest blocks by slot to keep recent fork candidates
                if cache.len() > 100 {
                    let mut blocks_by_slot: Vec<(Hash, u32)> =
                        cache.iter().map(|(h, b)| (*h, b.header.slot)).collect();
                    blocks_by_slot.sort_by_key(|(_, slot)| *slot);
                    // Remove oldest 50 blocks
                    for (hash, _) in blocks_by_slot.into_iter().take(50) {
                        cache.remove(&hash);
                    }
                }
            }

            debug!(
                "Block {} doesn't build on tip {} (builds on {}), cached for potential reorg",
                block_hash, current_tip, block.header.prev_hash
            );

            // Check for reorg using weight-based fork choice rule
            // Get the producer's effective weight to compare chain weights
            let producer_weight = {
                let producers = self.producer_set.read().await;
                producers
                    .get_by_pubkey(&block.header.producer)
                    .map(|p| p.effective_weight(current_height + 1))
                    .unwrap_or(1) // Default weight 1 for unknown producers (bootstrap)
            };

            let reorg_result = {
                self.sync_manager
                    .write()
                    .await
                    .handle_new_block_weighted(block.clone(), producer_weight)
            };

            if let Some(reorg_result) = reorg_result {
                // Weight-based fork choice: the new chain is heavier
                info!(
                    "Reorg to heavier chain: rolling back {} blocks, applying {} new blocks, weight_delta=+{}",
                    reorg_result.rollback.len(),
                    reorg_result.new_blocks.len(),
                    reorg_result.weight_delta
                );

                // Execute the reorg
                if let Err(e) = self.execute_reorg(reorg_result, block).await {
                    error!("Failed to execute reorg: {}", e);
                }
            } else {
                // Reorg detection failed (parent not in our recent blocks)
                // Check if we're likely on a fork by looking at cache size
                let cache_size = self.fork_block_cache.read().await.len();
                let our_height = self.chain_state.read().await.best_height;

                // If we have many cached blocks from other chains, we're likely on a fork
                let resync_cooldown = Duration::from_secs(120); // 2 minute cooldown
                let can_resync = match self.last_resync_time {
                    Some(last_time) => last_time.elapsed() > resync_cooldown,
                    None => true,
                };

                // Don't resync during discovery grace period
                let past_grace_period = match self.first_peer_connected {
                    Some(connected_time) => connected_time.elapsed() > Duration::from_secs(30),
                    None => false,
                };

                // Fork detection threshold: number of orphan blocks before triggering resync
                // Higher threshold prevents false positives from external peers with different chains
                // At 10s slots, 30 blocks = 5 minutes of chain history
                let fork_threshold = match self.config.network {
                    Network::Mainnet => 60, // 1 hour at 60s slots
                    Network::Testnet => 30, // 5 minutes at 10s slots
                    Network::Devnet => 20,  // ~2 minutes at 5s slots
                };

                // Trigger resync if we have fork_threshold+ blocks from other chains that we can't apply
                // AND we're past the grace period AND we have some chain progress
                // This indicates we're on a fork (devnet/testnet only)
                if cache_size >= fork_threshold
                    && can_resync
                    && past_grace_period
                    && our_height > 0
                    && (self.config.network == Network::Devnet
                        || self.config.network == Network::Testnet)
                {
                    warn!(
                        "Detected fork: {} blocks cached (threshold={}) that don't build on our chain (height {}). Triggering forced resync.",
                        cache_size, fork_threshold, our_height
                    );

                    // Log diagnostic info before resync
                    {
                        let state = self.chain_state.read().await;
                        warn!(
                            "  Our chain tip: hash={}, height={}, slot={}",
                            &state.best_hash.to_string()[..16],
                            state.best_height,
                            state.best_slot
                        );
                    }

                    // Log sample of orphan blocks
                    {
                        let cache = self.fork_block_cache.read().await;
                        let mut orphans: Vec<_> = cache.iter().collect();
                        // Sort by slot for consistent display
                        orphans.sort_by_key(|(_, b)| b.header.slot);

                        warn!(
                            "  Sample orphan blocks (showing up to 5 of {}):",
                            cache.len()
                        );
                        for (hash, block) in orphans.iter().take(5) {
                            let producer_hash = crypto_hash(block.header.producer.as_bytes());
                            let parent_short = &block.header.prev_hash.to_string()[..16];
                            warn!(
                                "    - hash={}, slot={}, parent={}, producer={}",
                                &hash.to_string()[..16],
                                block.header.slot,
                                parent_short,
                                &producer_hash.to_hex()[..16]
                            );
                        }

                        // Check if orphans have a common pattern (same genesis, different chain)
                        if let Some((_, first_block)) = orphans.first() {
                            let first_parent = first_block.header.prev_hash;
                            let same_parent_count = orphans
                                .iter()
                                .filter(|(_, b)| b.header.prev_hash == first_parent)
                                .count();
                            if same_parent_count > 3 {
                                warn!(
                                    "  Note: {} orphans share parent {} - likely external chain",
                                    same_parent_count,
                                    &first_parent.to_string()[..16]
                                );
                            }
                        }
                    }

                    // Force resync: reset to genesis and rebuild from peers
                    self.last_resync_time = Some(Instant::now());
                    if let Err(e) = self.force_resync_from_genesis().await {
                        error!("Failed to force resync: {}", e);
                    }
                } else if cache_size >= 2 {
                    // Try to chain the blocks from cache
                    debug!(
                        "Attempting to apply cached chain: {} blocks in cache",
                        cache_size
                    );

                    if let Err(e) = self.try_apply_cached_chain(block).await {
                        debug!("Could not apply cached chain: {}", e);
                    }
                }
            }
            return Ok(());
        }
        drop(state);

        // Apply the block
        self.apply_block(block).await?;

        Ok(())
    }

    /// Execute a chain reorganization
    ///
    /// This function is atomic: either the full reorg succeeds, or the chain
    /// remains unchanged. We build new state in temporary structures and only
    /// swap them in on success.
    async fn execute_reorg(
        &mut self,
        reorg_result: ReorgResult,
        triggering_block: Block,
    ) -> Result<()> {
        let rollback_count = reorg_result.rollback.len();
        let new_block_count = reorg_result.new_blocks.len();

        info!(
            "Executing reorg: rolling back {} blocks, applying {} new blocks",
            rollback_count, new_block_count
        );

        // Collect all new blocks we need to apply
        let mut new_blocks: Vec<Block> = Vec::new();

        {
            let cache = self.fork_block_cache.read().await;
            for block_hash in &reorg_result.new_blocks {
                if *block_hash == triggering_block.hash() {
                    new_blocks.push(triggering_block.clone());
                } else if let Some(cached_block) = cache.get(block_hash) {
                    new_blocks.push(cached_block.clone());
                } else {
                    warn!(
                        "Cannot execute reorg: missing block {} (need to sync from peers)",
                        block_hash
                    );
                    return Ok(());
                }
            }
        }

        // Sort new blocks by slot number (provides a total order)
        new_blocks.sort_by_key(|b| b.header.slot);

        // Validate the chain: each block must build on the previous
        for i in 1..new_blocks.len() {
            if new_blocks[i].header.prev_hash != new_blocks[i - 1].hash() {
                error!(
                    "Reorg chain is broken: block {} doesn't build on {}",
                    new_blocks[i].hash(),
                    new_blocks[i - 1].hash()
                );
                return Ok(());
            }
        }

        // Get current state
        let current_height = self.chain_state.read().await.best_height;
        let target_height = current_height - rollback_count as u64;

        info!(
            "Rolling back from height {} to {} (common ancestor)",
            current_height, target_height
        );

        // Find the common ancestor block
        let common_ancestor_block = if target_height == 0 {
            None
        } else {
            self.block_store.get_block_by_height(target_height)?
        };

        let genesis_hash = self.chain_state.read().await.genesis_hash;
        let common_ancestor_hash = common_ancestor_block
            .as_ref()
            .map(|b| b.hash())
            .unwrap_or(genesis_hash);
        let common_ancestor_slot = common_ancestor_block
            .as_ref()
            .map(|b| b.header.slot)
            .unwrap_or(0);

        // Build new UTXO set atomically (don't modify existing until success)
        info!("Building new UTXO set up to height {}", target_height);
        let mut new_utxo = UtxoSet::new();

        // Re-apply all blocks from 1 to target_height
        for height in 1..=target_height {
            if let Some(block) = self.block_store.get_block_by_height(height)? {
                for (tx_index, tx) in block.transactions.iter().enumerate() {
                    let is_coinbase = tx_index == 0 && tx.is_coinbase();
                    // For non-coinbase, spend inputs first
                    if !is_coinbase {
                        if let Err(e) = new_utxo.spend_transaction(tx) {
                            error!(
                                "Reorg UTXO rebuild failed at height {}: {} - aborting reorg",
                                height, e
                            );
                            return Err(anyhow::anyhow!("UTXO rebuild failed: {}", e));
                        }
                    }
                    new_utxo.add_transaction(tx, height, is_coinbase);
                }
            }
        }

        // Apply new blocks to the temporary UTXO set first (validation)
        let mut temp_height = target_height;
        for block in &new_blocks {
            temp_height += 1;
            for (tx_index, tx) in block.transactions.iter().enumerate() {
                let is_coinbase = tx_index == 0 && tx.is_coinbase();
                if !is_coinbase {
                    if let Err(e) = new_utxo.spend_transaction(tx) {
                        error!(
                            "Reorg validation failed for block {}: {} - aborting reorg",
                            block.hash(),
                            e
                        );
                        return Err(anyhow::anyhow!("Reorg validation failed: {}", e));
                    }
                }
                new_utxo.add_transaction(tx, temp_height, is_coinbase);
            }
        }

        // All validation passed - now atomically update state
        // Hold both locks to prevent inconsistent reads
        info!("Reorg validated, applying state changes atomically");
        {
            let mut state = self.chain_state.write().await;
            let mut utxo = self.utxo_set.write().await;

            // Reset to common ancestor
            state.best_height = target_height;
            state.best_hash = common_ancestor_hash;
            state.best_slot = common_ancestor_slot;

            // Swap in the new UTXO set (built up to common ancestor only)
            // We'll apply the new blocks via apply_block which updates UTXO properly
            utxo.clear();
            // Re-apply up to common ancestor (we validated this already)
            for height in 1..=target_height {
                if let Some(block) = self.block_store.get_block_by_height(height).ok().flatten() {
                    for (tx_index, tx) in block.transactions.iter().enumerate() {
                        let is_coinbase = tx_index == 0 && tx.is_coinbase();
                        if !is_coinbase {
                            let _ = utxo.spend_transaction(tx);
                        }
                        utxo.add_transaction(tx, height, is_coinbase);
                    }
                }
            }
        }

        // Rebuild producer set to match common ancestor state
        {
            let mut producers = self.producer_set.write().await;
            producers.clear();
            // Re-apply producer registrations from genesis to common ancestor
            for height in 1..=target_height {
                if let Some(block) = self.block_store.get_block_by_height(height)? {
                    for tx in &block.transactions {
                        if tx.tx_type == TxType::Registration {
                            if let Some(reg_data) = tx.registration_data() {
                                // Find the bond output
                                if let Some((bond_index, bond_output)) =
                                    tx.outputs.iter().enumerate().find(|(_, o)| {
                                        o.output_type == doli_core::transaction::OutputType::Bond
                                    })
                                {
                                    let tx_hash = tx.hash();
                                    let era = self.params.height_to_era(height);

                                    let producer_info = storage::ProducerInfo::new(
                                        reg_data.public_key.clone(),
                                        height,
                                        bond_output.amount,
                                        (tx_hash, bond_index as u32),
                                        era,
                                    );

                                    let _ = producers.register(producer_info, height);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Rebuild claim registry to match common ancestor state
        {
            let mut claim_registry = self.claim_registry.write().await;
            // Create fresh registry and re-apply all claims from genesis to common ancestor
            *claim_registry = ClaimRegistry::new();
            for height in 1..=target_height {
                if let Some(block) = self.block_store.get_block_by_height(height)? {
                    for tx in &block.transactions {
                        if tx.tx_type == TxType::ClaimEpochReward {
                            if let Some(claim_data) = tx.claim_epoch_reward_data() {
                                let record = ClaimRecord::new(
                                    tx.hash(),
                                    height,
                                    tx.outputs.first().map(|o| o.amount).unwrap_or(0),
                                    block.header.timestamp,
                                );
                                let _ = claim_registry.mark_claimed(
                                    &claim_data.producer_pubkey,
                                    claim_data.epoch,
                                    record,
                                );
                            }
                        }
                    }
                }
            }
        }

        // Now apply the new blocks through normal path
        info!("Applying {} new blocks from fork", new_blocks.len());
        for block in new_blocks {
            self.apply_block(block).await?;
        }

        // Clear applied blocks from fork cache
        {
            let mut cache = self.fork_block_cache.write().await;
            for hash in &reorg_result.new_blocks {
                cache.remove(hash);
            }
        }

        // Invalidate mempool - transactions may now be invalid
        {
            let mut mempool = self.mempool.write().await;
            let utxo = self.utxo_set.read().await;
            let height = self.chain_state.read().await.best_height;
            mempool.revalidate(&utxo, height);
        }

        info!(
            "Reorg complete: now at height {}",
            self.chain_state.read().await.best_height
        );

        Ok(())
    }

    /// Try to apply a chain of cached blocks when we're behind
    ///
    /// This function attempts to build a chain from cached fork blocks
    /// back to our current tip, then applies them in order.
    async fn try_apply_cached_chain(&mut self, latest_block: Block) -> Result<()> {
        let our_tip = self.chain_state.read().await.best_hash;

        // Build chain backwards from latest_block to our tip
        let mut chain: Vec<Block> = Vec::new();
        let mut current = latest_block.clone();

        // Limit how far back we'll look (prevent infinite loops)
        const MAX_CHAIN_LENGTH: usize = 50;

        for _ in 0..MAX_CHAIN_LENGTH {
            let parent_hash = current.header.prev_hash;

            if parent_hash == our_tip {
                // Found connection to our chain!
                chain.reverse(); // Blocks are in reverse order, flip them
                chain.insert(0, current);

                info!(
                    "Found chain of {} cached blocks connecting to our tip, applying",
                    chain.len()
                );

                // Apply all blocks in order
                for block in chain {
                    // Remove from cache before applying
                    {
                        let mut cache = self.fork_block_cache.write().await;
                        cache.remove(&block.hash());
                    }
                    self.apply_block(block).await?;
                }

                return Ok(());
            }

            // Check if parent is in our block store (not just cache)
            if let Ok(Some(_)) = self.block_store.get_block(&parent_hash) {
                // Parent is in our store but not our tip - this is a fork
                // We can't simply apply these blocks; we'd need to reorg
                debug!(
                    "Parent {} found in store but not at tip - would need reorg",
                    parent_hash
                );
                break;
            }

            // Look for parent in cache
            let cache = self.fork_block_cache.read().await;
            if let Some(parent) = cache.get(&parent_hash) {
                chain.push(current);
                current = parent.clone();
            } else {
                // Parent not in cache - can't build chain
                debug!("Parent {} not in cache, cannot build chain", parent_hash);
                break;
            }
        }

        // Couldn't build complete chain - maybe we need to sync from peers
        // This will be handled by the normal sync process
        anyhow::bail!("Could not build complete chain from cached blocks")
    }

    /// Force resync from genesis (devnet/testnet recovery mechanism)
    ///
    /// This is an aggressive recovery mechanism for when a node gets stuck on a fork.
    /// It resets the chain state to genesis and re-syncs from peers.
    async fn force_resync_from_genesis(&mut self) -> Result<()> {
        warn!("Force resync initiated - resetting to genesis");

        // Get genesis hash before clearing state
        let genesis_hash = self.chain_state.read().await.genesis_hash;

        // Reset chain state to genesis
        {
            let mut state = self.chain_state.write().await;
            state.best_height = 0;
            state.best_hash = genesis_hash;
            state.best_slot = 0;
        }

        // Clear UTXO set
        {
            let mut utxo = self.utxo_set.write().await;
            utxo.clear();
        }

        // Clear fork block cache
        {
            let mut cache = self.fork_block_cache.write().await;
            cache.clear();
        }

        // Reset sync manager
        {
            let mut sync = self.sync_manager.write().await;
            sync.reset_local_state(genesis_hash);
        }

        // Reset production state
        self.last_produced_slot = 0;

        info!("Chain reset to genesis (height 0). Will re-sync from peers.");

        // After resync, we need to request blocks from peers.
        // The sync manager will handle this via the normal block request mechanism.
        // For now, we'll rely on incoming blocks being cached and applied as they connect.

        Ok(())
    }

    /// Apply a block to the chain
    async fn apply_block(&mut self, block: Block) -> Result<()> {
        let block_hash = block.hash();
        let height = self.chain_state.read().await.best_height + 1;

        info!("Applying block {} at height {}", block_hash, height);

        // Store the block
        self.block_store.put_block(&block, height)?;

        // Apply transactions to UTXO set and process special transactions
        {
            let mut utxo = self.utxo_set.write().await;
            let mut producers = self.producer_set.write().await;
            let mut claim_registry = self.claim_registry.write().await;

            for (tx_index, tx) in block.transactions.iter().enumerate() {
                // Apply UTXO changes
                let is_coinbase = tx_index == 0 && tx.is_coinbase();
                let _ = utxo.spend_transaction(tx); // Ignore errors for coinbase
                utxo.add_transaction(tx, height, is_coinbase);

                // Process registration transactions
                if tx.tx_type == TxType::Registration {
                    if let Some(reg_data) = tx.registration_data() {
                        // Find the bond output
                        if let Some((bond_index, bond_output)) =
                            tx.outputs.iter().enumerate().find(|(_, o)| {
                                o.output_type == doli_core::transaction::OutputType::Bond
                            })
                        {
                            let tx_hash = tx.hash();
                            let era = self.params.height_to_era(height);

                            let producer_info = storage::ProducerInfo::new(
                                reg_data.public_key.clone(),
                                height,
                                bond_output.amount,
                                (tx_hash, bond_index as u32),
                                era,
                            );

                            if let Err(e) = producers.register(producer_info, height) {
                                warn!("Failed to register producer: {}", e);
                            } else {
                                info!(
                                    "Registered producer {} at height {}",
                                    crypto_hash(reg_data.public_key.as_bytes()),
                                    height
                                );
                            }
                        }
                    }
                }

                // Process exit transactions
                if tx.tx_type == TxType::Exit {
                    if let Some(exit_data) = tx.exit_data() {
                        if let Err(e) = producers.request_exit(&exit_data.public_key, height) {
                            warn!("Failed to process exit: {}", e);
                        } else {
                            info!(
                                "Producer {} requested exit at height {}",
                                crypto_hash(exit_data.public_key.as_bytes()),
                                height
                            );
                        }
                    }
                }

                // Process slash transactions - equivocation (double signing) penalty
                if tx.tx_type == TxType::SlashProducer {
                    if let Some(slash_data) = tx.slash_data() {
                        match producers.slash_producer(&slash_data.producer_pubkey, height) {
                            Ok(burned_amount) => {
                                warn!(
                                    "SLASHED: Producer {} burned {} DOLI (100% bond) for equivocation at height {}",
                                    crypto_hash(slash_data.producer_pubkey.as_bytes()),
                                    burned_amount / UNITS_PER_COIN, // Convert to DOLI
                                    height
                                );
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to slash producer {}: {}",
                                    crypto_hash(slash_data.producer_pubkey.as_bytes()),
                                    e
                                );
                            }
                        }
                    }
                }

                // Process epoch reward claim transactions
                if tx.tx_type == TxType::ClaimEpochReward {
                    if let Some(claim_data) = tx.claim_epoch_reward_data() {
                        // Create claim record
                        let record = ClaimRecord::new(
                            tx.hash(),
                            height,
                            tx.outputs.first().map(|o| o.amount).unwrap_or(0),
                            block.header.timestamp,
                        );

                        // Mark as claimed in registry
                        if let Err(e) = claim_registry.mark_claimed(
                            &claim_data.producer_pubkey,
                            claim_data.epoch,
                            record,
                        ) {
                            warn!(
                                "Failed to record epoch claim for producer {} epoch {}: {}",
                                crypto_hash(claim_data.producer_pubkey.as_bytes()),
                                claim_data.epoch,
                                e
                            );
                        } else {
                            info!(
                                "Recorded epoch {} claim for producer {} ({} DOLI)",
                                claim_data.epoch,
                                &crypto_hash(claim_data.producer_pubkey.as_bytes()).to_hex()[..16],
                                tx.outputs
                                    .first()
                                    .map(|o| o.amount / UNITS_PER_COIN)
                                    .unwrap_or(0)
                            );
                        }
                    }
                }
            }

            // Process completed unbonding periods
            let completed = producers.process_unbonding(height, UNBONDING_PERIOD);
            for producer in &completed {
                info!(
                    "Producer {} completed unbonding, bond released",
                    crypto_hash(producer.public_key.as_bytes())
                );
            }
        }

        // Update chain state
        {
            let mut state = self.chain_state.write().await;
            state.update(block_hash, height, block.header.slot);

            // For devnet: set genesis_timestamp from first block's timestamp
            // This ensures all nodes agree on genesis time after syncing the genesis block
            if state.genesis_timestamp == 0 && height <= 1 {
                let block_timestamp = block.header.timestamp;
                // Use the block timestamp as genesis time (rounded to slot boundary)
                let new_genesis_time =
                    block_timestamp - (block_timestamp % self.params.slot_duration);
                state.genesis_timestamp = new_genesis_time;

                // Also update params.genesis_time for devnet so slot calculations use synced value
                if self.config.network == Network::Devnet
                    && self.params.genesis_time != new_genesis_time
                {
                    info!(
                        "Devnet genesis time synced from block {}: {} (was {})",
                        height, new_genesis_time, self.params.genesis_time
                    );
                    self.params.genesis_time = new_genesis_time;
                } else {
                    info!(
                        "Genesis timestamp set from block {}: {}",
                        height, state.genesis_timestamp
                    );
                }
            }
        }

        // Remove transactions from mempool
        {
            let mut mempool = self.mempool.write().await;
            mempool.remove_for_block(&block.transactions);
        }

        // Get producer's effective weight for fork choice rule
        let producer_weight = {
            let producers = self.producer_set.read().await;
            producers
                .get_by_pubkey(&block.header.producer)
                .map(|p| p.effective_weight(height))
                .unwrap_or(1) // Default weight 1 for unknown producers (bootstrap)
        };

        // Update sync manager with weight for fork choice rule
        self.sync_manager.write().await.block_applied_with_weight(
            block_hash,
            height,
            block.header.slot,
            producer_weight,
            block.header.prev_hash,
        );

        // Periodic state save for crash resilience
        // This ensures state is persisted even if the node crashes before graceful shutdown
        self.maybe_save_state().await?;

        // Note: Don't broadcast here. Blocks received from the network should not be
        // re-broadcast (they already came from the network). Locally produced blocks
        // should be broadcast explicitly by the caller (produce_block).

        Ok(())
    }

    /// Handle a new transaction from the network
    async fn handle_new_transaction(&self, tx: Transaction) -> Result<()> {
        let tx_hash = tx.hash();

        // Check if we already have this transaction
        {
            let mempool = self.mempool.read().await;
            if mempool.contains(&tx_hash) {
                debug!("Transaction {} already in mempool", tx_hash);
                return Ok(());
            }
        }

        // Add to mempool
        let current_height = self.chain_state.read().await.best_height;
        let result = {
            let utxo = self.utxo_set.read().await;
            let mut mempool = self.mempool.write().await;
            mempool.add_transaction(tx.clone(), &utxo, current_height)
        };

        match result {
            Ok(_) => {
                info!("Added transaction {} to mempool", tx_hash);
                // Broadcast to network
                if let Some(ref network) = self.network {
                    let _ = network.broadcast_transaction(tx).await;
                }
            }
            Err(e) => {
                debug!("Failed to add transaction {} to mempool: {}", tx_hash, e);
            }
        }

        Ok(())
    }

    /// Handle a sync request from a peer
    async fn handle_sync_request(
        &self,
        request: network::protocols::SyncRequest,
        channel: network::ResponseChannel<network::protocols::SyncResponse>,
    ) -> Result<()> {
        use network::protocols::{SyncRequest, SyncResponse};

        let response = match request {
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            } => {
                let mut headers = Vec::new();
                let state = self.chain_state.read().await;
                let genesis_hash = state.genesis_hash;
                let best_height = state.best_height;
                drop(state);

                // Determine starting height
                // If start_hash is genesis, start from height 1
                // Otherwise, find the block with this hash and get its height
                let start_height = if start_hash == genesis_hash {
                    0
                } else {
                    // Find the height of start_hash by iterating (suboptimal but works)
                    let mut found_height = None;
                    for h in 1..=best_height {
                        if let Ok(Some(block)) = self.block_store.get_block_by_height(h) {
                            if block.hash() == start_hash {
                                found_height = Some(h);
                                break;
                            }
                        }
                    }
                    match found_height {
                        Some(h) => h,
                        None => {
                            // Unknown hash, return empty
                            debug!("GetHeaders: unknown start_hash {}", start_hash);
                            return Ok(());
                        }
                    }
                };

                // Return headers from start_height+1 up to max_count
                let end_height = (start_height + max_count as u64).min(best_height);
                for height in (start_height + 1)..=end_height {
                    if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                        headers.push(block.header.clone());
                    } else {
                        break;
                    }
                }

                debug!(
                    "GetHeaders: returning {} headers (heights {}..={})",
                    headers.len(),
                    start_height + 1,
                    end_height
                );
                SyncResponse::Headers(headers)
            }

            SyncRequest::GetBodies { hashes } => {
                let mut bodies = Vec::new();
                for hash in hashes {
                    if let Ok(Some(block)) = self.block_store.get_block(&hash) {
                        bodies.push(block);
                    }
                }
                SyncResponse::Bodies(bodies)
            }

            SyncRequest::GetBlockByHeight { height } => {
                match self.block_store.get_block_by_height(height) {
                    Ok(Some(block)) => SyncResponse::Block(Some(block)),
                    _ => SyncResponse::Block(None),
                }
            }

            SyncRequest::GetBlockByHash { hash } => match self.block_store.get_block(&hash) {
                Ok(Some(block)) => SyncResponse::Block(Some(block)),
                _ => SyncResponse::Block(None),
            },
        };

        if let Some(ref network) = self.network {
            let _ = network.send_sync_response(channel, response).await;
        }

        Ok(())
    }

    /// Try to produce a block if we're an eligible producer
    async fn try_produce_block(&mut self) -> Result<()> {
        let producer_key = match &self.producer_key {
            Some(k) => k,
            None => return Ok(()),
        };

        // VERSION ENFORCEMENT CHECK
        // If an update has been approved and grace period has passed,
        // outdated nodes cannot produce blocks.
        if let Err(blocked) = node_updater::is_production_allowed(&self.config.data_dir) {
            // Log once per minute to avoid spam
            static LAST_WARNING: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let last = LAST_WARNING.load(std::sync::atomic::Ordering::Relaxed);
            if now_secs - last >= 60 {
                LAST_WARNING.store(now_secs, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!("{}", blocked);
            }
            return Ok(());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let current_slot = self.params.timestamp_to_slot(now);

        // Log slot info periodically (every ~60 seconds)
        if current_slot % 60 == 0 && current_slot != self.last_produced_slot {
            info!(
                "Production check: now={}, genesis={}, current_slot={}, last_produced={}",
                now, self.params.genesis_time, current_slot, self.last_produced_slot
            );
        }

        // Don't produce for the same slot twice
        if current_slot <= self.last_produced_slot {
            return Ok(());
        }

        // Get chain state
        let state = self.chain_state.read().await;
        let prev_hash = state.best_hash;
        let prev_slot = state.best_slot;
        let height = state.best_height + 1;
        drop(state);

        // Can't produce if slot hasn't advanced
        if current_slot <= prev_slot {
            debug!(
                "Slot not advanced: current_slot={} <= prev_slot={}",
                current_slot, prev_slot
            );
            return Ok(());
        }

        // Get active producers with their effective weights for weighted selection (Option C)
        // This enables anti-grinding protection: top N by weight are eligible (deterministic),
        // then hash selects among them (limited grinding impact)
        let producers = self.producer_set.read().await;
        let active_with_weights: Vec<(PublicKey, u64)> = producers
            .active_producers()
            .iter()
            .map(|p| (p.public_key.clone(), p.effective_weight(height)))
            .collect();
        drop(producers);

        // If no active producers and this is testnet/devnet, allow bootstrap production
        let our_pubkey = producer_key.public_key().clone();
        let (eligible, our_bootstrap_rank) = if active_with_weights.is_empty() {
            match self.config.network {
                Network::Testnet | Network::Devnet => {
                    // Bootstrap mode: deterministic rank-based leader election
                    // Each producer computes their rank based on slot + their public key.
                    // We deliberately DO NOT use prev_hash here because nodes may be on
                    // different chains initially. Using only slot ensures all nodes
                    // compute the same election result regardless of chain state.

                    // PRODUCER LIST STABILITY CHECK: Don't produce until the producer list
                    // has been stable for N seconds. This ensures anti-entropy has converged
                    // and all nodes have discovered all producers before production begins.
                    // Without this, a node might start producing after discovering only 2 of 3
                    // producers, leading to incorrect round-robin and chain forks.
                    //
                    // The stability window must be longer than the gossip interval (10 seconds)
                    // to ensure at least one full anti-entropy round has passed without changes.
                    // Use shorter window for devnet to enable faster testing.
                    let producer_list_stability_secs: u64 =
                        if self.config.network == Network::Devnet {
                            3 // Fast for devnet testing
                        } else {
                            15 // Production-like for testnet
                        };
                    if let Some(last_change) = self.last_producer_list_change {
                        let elapsed = last_change.elapsed();
                        if elapsed.as_secs() < producer_list_stability_secs {
                            debug!(
                                "Producer list changed {:?} ago, waiting for stability ({} secs required)...",
                                elapsed, producer_list_stability_secs
                            );
                            return Ok(());
                        }
                    }

                    // SYNC-BEFORE-PRODUCE: Check if we should defer to syncing
                    // This replaces the old time-based warmup with a state-based check.
                    //
                    // Rules:
                    // 1. If we have NO bootstrap nodes configured -> we're the seed node, produce immediately
                    // 2. If we have bootstrap nodes but no peer status yet -> wait for connection
                    // 3. If our chain is significantly behind peers -> wait for sync
                    // 4. If we're synced (within 2 slots of best peer) -> produce
                    //
                    // This scales to thousands of nodes because:
                    // - No arbitrary time delays
                    // - Nodes naturally wait for sync before producing
                    // - Seed nodes (no bootstrap config) can start immediately
                    // - Joining nodes sync first, then produce
                    let has_bootstrap_nodes = !self.config.bootstrap_nodes.is_empty();
                    let sync_state = self.sync_manager.read().await;
                    let peer_count = sync_state.peer_count();
                    let best_peer_height = sync_state.best_peer_height();
                    let best_peer_slot = sync_state.best_peer_slot();
                    drop(sync_state);

                    if has_bootstrap_nodes && peer_count == 0 {
                        // We have bootstrap nodes configured but haven't received their status yet
                        // Wait for connection before producing to avoid chain splits
                        debug!(
                            "Bootstrap sync: waiting for peer status (bootstrap nodes configured but no peers yet)"
                        );
                        return Ok(());
                    }

                    if peer_count > 0 {
                        // We have peers - check if we're synced
                        let our_height = height.saturating_sub(1); // height is next block
                        let our_slot = prev_slot;

                        // Consider synced if within 2 slots of best peer
                        // This allows for natural network propagation delays
                        let slot_diff = best_peer_slot.saturating_sub(our_slot);
                        let height_diff = best_peer_height.saturating_sub(our_height);

                        if slot_diff > 2 || height_diff > 2 {
                            debug!(
                                "Bootstrap sync: deferring production (our slot={}, peer slot={}, diff={})",
                                our_slot, best_peer_slot, slot_diff
                            );
                            return Ok(());
                        }

                        // POST-RESYNC GRACE PERIOD: After a forced resync, wait before producing
                        // This handles the case where all nodes reset simultaneously - they would
                        // all see each other at height 0 and think they're "synced". By waiting,
                        // we give sync a chance to actually fetch blocks from peers.
                        // Use shorter grace for devnet testing.
                        let post_resync_grace_secs: u64 = if self.config.network == Network::Devnet
                        {
                            5 // Fast for devnet testing
                        } else {
                            30 // Production-like for testnet
                        };
                        if let Some(last_resync) = self.last_resync_time {
                            if last_resync.elapsed().as_secs() < post_resync_grace_secs {
                                debug!(
                                    "Post-resync grace: waiting {}s before producing (peers may have more blocks)",
                                    post_resync_grace_secs - last_resync.elapsed().as_secs()
                                );
                                return Ok(());
                            }
                        }
                    }
                    // DISCOVERY GRACE PERIOD for seed nodes ONLY
                    // If we're the seed node (no bootstrap config) and have peers but only know ourselves,
                    // wait for discovery. This prevents the seed node from monopolizing production
                    // before learning about other producers.
                    //
                    // Nodes WITH bootstrap config should produce immediately after syncing,
                    // so they can be discovered by the seed node.
                    let known = self.known_producers.read().await;
                    let known_count = known.len();
                    drop(known);

                    // Grace period only applies to seed nodes (no bootstrap config)
                    let is_seed_node = !has_bootstrap_nodes;
                    // Use shorter grace period for devnet to enable faster testing
                    let discovery_grace_secs: u64 = if self.config.network == Network::Devnet {
                        5 // Fast for devnet testing
                    } else {
                        30 // Production-like for testnet
                    };
                    let grace_period_active =
                        if let Some(first_peer_time) = self.first_peer_connected {
                            first_peer_time.elapsed().as_secs() < discovery_grace_secs
                        } else {
                            false
                        };

                    // Seed nodes wait during grace period if they only know themselves
                    if is_seed_node && peer_count > 0 && known_count <= 1 && grace_period_active {
                        debug!(
                            "Seed node discovery: waiting for producer discovery (peers={}, known={}, grace_remaining={}s)",
                            peer_count,
                            known_count,
                            discovery_grace_secs - self.first_peer_connected.map(|t| t.elapsed().as_secs()).unwrap_or(0)
                        );
                        return Ok(());
                    }

                    // LATE JOINER GUARD: Prevent production while isolated from producer discovery.
                    //
                    // Problem: A joining node connects to peers, passes sync checks (maybe everyone
                    // is at height 0), but hasn't received the producer list via anti-entropy yet.
                    // If it produces with only itself in the list, it uses wrong round-robin order.
                    //
                    // Solution: If we're connected to peers but only know about ourselves, wait.
                    // The anti-entropy gossip runs every 10 seconds, so we should learn about
                    // other producers quickly if they exist.
                    //
                    // Exception: If we've waited past the grace period and still only know ourselves,
                    // we're probably the only producer (or others haven't started yet), so proceed.
                    if !is_seed_node && peer_count > 0 && known_count <= 1 && grace_period_active {
                        debug!(
                            "Late joiner guard: waiting for anti-entropy producer discovery (peers={}, known={}, grace_remaining={}s)",
                            peer_count,
                            known_count,
                            discovery_grace_secs - self.first_peer_connected.map(|t| t.elapsed().as_secs()).unwrap_or(0)
                        );
                        return Ok(());
                    }

                    // EQUITABLE BOOTSTRAP MODE: Round-robin based on known peers
                    //
                    // For devnet/testnet to mimic mainnet's equitable distribution,
                    // we need fair rotation among all bootstrap producers. This works by:
                    // 1. Building a sorted list of all known producer pubkeys (us + peers)
                    // 2. Using slot % num_producers to pick the leader for this slot
                    // 3. Only the designated leader produces
                    //
                    // This ensures truly equitable block production regardless of
                    // network latency or hash luck.

                    // Build sorted list of known producers (us + peers we've seen produce)
                    // Uses the persistent known_producers list that survives epoch resets
                    let known = self.known_producers.read().await;
                    let mut known_producers: Vec<PublicKey> = known.clone();
                    drop(known);

                    // Always include ourselves
                    if !known_producers.iter().any(|p| p == &our_pubkey) {
                        known_producers.push(our_pubkey.clone());
                        // Sort for deterministic ordering (all nodes compute same order)
                        known_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                    }

                    let num_producers = known_producers.len();
                    let leader_index = (current_slot as usize) % num_producers;
                    let designated_leader = &known_producers[leader_index];

                    let is_our_turn = designated_leader == &our_pubkey;

                    debug!(
                        "Bootstrap round-robin: slot={}, {} known producers, leader_index={}, our_turn={}",
                        current_slot, num_producers, leader_index, is_our_turn
                    );

                    if !is_our_turn {
                        // Not our slot - don't produce
                        return Ok(());
                    }

                    // It's our turn - produce immediately (score 0)
                    (vec![our_pubkey.clone()], Some(0))
                }
                Network::Mainnet => {
                    // Mainnet requires registered producers
                    return Ok(());
                }
            }
        } else {
            // EPOCH LOOKAHEAD: Deterministic round-robin selection (anti-grinding)
            //
            // Selection is based ONLY on:
            // 1. Slot number (known in advance)
            // 2. Sorted producer list with bond weights (fixed at epoch start)
            //
            // NO dependency on prev_hash = NO grinding possible.
            // This is the core insight: "Proof of Longevity" over time,
            // not "Proof of Delay" via long VDF.
            //
            // Algorithm: slot % total_bonds -> deterministic ticket assignment
            let eligible = select_producer_for_slot(current_slot, &active_with_weights);
            (eligible, None)
        };

        if eligible.is_empty() {
            return Ok(());
        }

        // Calculate slot offset to determine eligibility window
        let slot_start = self.params.slot_to_timestamp(current_slot);
        let slot_offset = now.saturating_sub(slot_start);

        // Check if we're eligible at this time
        // For bootstrap mode, use continuous time-based scheduling
        // For normal mode, use the standard eligibility check
        let is_eligible = if let Some(score) = our_bootstrap_rank {
            // Bootstrap mode: continuous time-based scheduling
            // The score (0-255) determines when we should produce within the slot.
            // We can produce when the current time offset exceeds our target offset.
            let slot_duration = self.params.slot_duration;
            let max_offset_percent = 80; // Leave 20% for propagation
            let target_offset_percent = (score as u64 * max_offset_percent) / 255;
            let target_offset_secs = (slot_duration * target_offset_percent) / 100;

            // We're eligible if current offset >= our target offset
            slot_offset >= target_offset_secs
        } else {
            // Normal mode: use standard eligibility check
            consensus::is_producer_eligible(&our_pubkey, &eligible, slot_offset)
        };

        if !is_eligible {
            return Ok(());
        }

        // We're eligible - produce a block!
        info!(
            "Producing block for slot {} at height {} (offset {}s)",
            current_slot, height, slot_offset
        );

        // SIGNED SLOTS PROTECTION: Check if we already signed this slot
        // This prevents double-signing if we restart quickly
        if let Some(ref signed_slots) = self.signed_slots_db {
            if let Err(e) = signed_slots.check_and_mark(current_slot as u64) {
                error!("SLASHING PROTECTION: {}", e);
                return Ok(());
            }
        }

        // Build the block
        let mut builder = BlockBuilder::new(prev_hash, prev_slot, our_pubkey.clone())
            .with_params(self.params.clone());

        // Add transactions from mempool
        // Note: No coinbase transaction - rewards are claimed via ClaimEpochReward
        // transactions using the new weighted presence reward system.
        let mempool_txs: Vec<Transaction> = {
            let mempool = self.mempool.read().await;
            mempool.select_for_block(100) // Up to 100 tx per block
        };
        for tx in &mempool_txs {
            builder.add_transaction(tx.clone());
        }

        // Build block header (without VDF - we'll compute it)
        let header = match builder.build(now) {
            Some(h) => h,
            None => {
                warn!("Failed to build block - slot monotonicity violation");
                return Ok(());
            }
        };

        // Compute VDF using hash-chain with dynamically calibrated iterations
        // The VDF serves as anti-grinding protection, not timing enforcement
        // For devnet, VDF is disabled to enable rapid development
        let (vdf_output, vdf_proof) = if self.config.network.vdf_enabled() {
            // Construct VDF input from block context: prev_hash || tx_root || slot || producer_key
            let vdf_input = construct_vdf_input(
                &prev_hash,
                &header.merkle_root,
                header.slot,
                &header.producer,
            );

            // Get network-specific fixed iterations (must match validation)
            let iterations = self.config.network.heartbeat_vdf_iterations();
            info!(
                "Computing hash-chain VDF with {} iterations (network={:?})...",
                iterations, self.config.network
            );

            // Use hash-chain VDF with calibrated iterations
            let vdf_input_clone = vdf_input;
            let vdf_start = std::time::Instant::now();
            let output_bytes =
                tokio::task::spawn_blocking(move || hash_chain_vdf(&vdf_input_clone, iterations))
                    .await
                    .map_err(|e| anyhow::anyhow!("VDF task failed: {}", e))?;
            let vdf_duration = vdf_start.elapsed();

            // Record timing for calibration
            self.vdf_calibrator
                .write()
                .await
                .record_timing(iterations, vdf_duration);

            info!("VDF computed in {:?} (target: ~700ms)", vdf_duration);
            (
                VdfOutput {
                    value: output_bytes.to_vec(),
                },
                VdfProof::empty(), // Hash-chain VDF is self-verifying
            )
        } else {
            // VDF disabled for this network - use placeholder values
            info!(
                "VDF disabled for {:?} network, using placeholder",
                self.config.network
            );
            (
                VdfOutput {
                    value: prev_hash.as_bytes().to_vec(),
                },
                VdfProof::empty(),
            )
        };

        // Create final block header with VDF
        let final_header = BlockHeader {
            version: header.version,
            prev_hash: header.prev_hash,
            merkle_root: header.merkle_root,
            timestamp: header.timestamp,
            slot: header.slot,
            producer: header.producer,
            vdf_output,
            vdf_proof,
        };

        // Collect transactions from mempool
        // Note: No automatic coinbase or epoch reward transactions - the new weighted
        // presence reward system uses on-demand ClaimEpochReward transactions that
        // producers submit when they want to claim their accumulated rewards.
        let transactions = mempool_txs;

        // Build presence commitment from collected heartbeats
        //
        // The presence commitment records which producers submitted valid heartbeat
        // proofs during this slot. This data is used for weighted presence rewards.
        //
        // Currently, heartbeat gossip is not yet implemented, so we only mark the
        // block producer as present (they are definitionally present to produce).
        // Full heartbeat collection will be added when network gossip is integrated.
        let presence = self
            .build_presence_commitment(&our_pubkey, current_slot, &prev_hash)
            .await;

        let block = Block {
            header: final_header,
            transactions,
            presence: Some(presence),
        };

        let block_hash = block.hash();
        info!("Block {} produced at height {}", block_hash, height);

        // Apply the block locally
        self.apply_block(block.clone()).await?;

        // Mark that we produced for this slot
        self.last_produced_slot = current_slot;

        // Broadcast the block to the network
        // This is only done for blocks we produce ourselves - received blocks
        // are already on the network and don't need to be re-broadcast.
        if let Some(ref network) = self.network {
            let _ = network.broadcast_block(block).await;
        }

        Ok(())
    }

    /// Build presence commitment for a block
    ///
    /// Collects valid heartbeats from the heartbeat pool and builds a presence
    /// commitment that records which producers were present during this slot.
    ///
    /// Currently, this marks only the block producer as present since full
    /// heartbeat gossip integration is pending. When heartbeat gossip is added,
    /// this will include all producers with valid witnessed heartbeats.
    async fn build_presence_commitment(
        &mut self,
        our_pubkey: &PublicKey,
        current_slot: u32,
        prev_hash: &Hash,
    ) -> doli_core::presence::PresenceCommitment {
        // Get active producers and their bond weights
        let (active_producers, bond_weights) = {
            let producers = self.producer_set.read().await;
            let active: Vec<PublicKey> = producers
                .active_producers()
                .iter()
                .map(|p| p.public_key.clone())
                .collect();

            let weights: std::collections::HashMap<PublicKey, u64> = producers
                .active_producers()
                .iter()
                .map(|p| (p.public_key.clone(), p.bond_amount))
                .collect();

            (active, weights)
        };

        // Sort producers by pubkey for deterministic ordering
        let mut sorted_producers = active_producers.clone();
        sorted_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Reset heartbeat pool for current slot
        self.heartbeat_pool.reset_for_slot(
            current_slot,
            *prev_hash,
            sorted_producers.clone(),
            bond_weights.clone(),
        );

        // Currently, heartbeat collection from the network is not implemented.
        // For now, we mark the block producer as present (they must be present to produce).
        // This ensures presence tracking works correctly even before heartbeat gossip is added.
        //
        // Future milestone: Add heartbeat gossip to network layer and collect heartbeats
        // from all producers during the slot.

        // Find producer index in sorted list
        let producer_index = sorted_producers.iter().position(|p| p == our_pubkey);

        if let Some(idx) = producer_index {
            // Mark block producer as present with their bond weight
            let weight = bond_weights.get(our_pubkey).copied().unwrap_or(0);

            if weight > 0 {
                // Build presence with just the block producer
                // The merkle root is zero since we don't have actual heartbeat data yet
                let present_indices = vec![idx];
                let weights = vec![weight];

                debug!(
                    "Building presence: producer at index {} with weight {} (slot {})",
                    idx, weight, current_slot
                );

                return doli_core::presence::PresenceCommitment::new(
                    sorted_producers.len(),
                    &present_indices,
                    weights,
                    Hash::ZERO, // Placeholder merkle root until heartbeat gossip is added
                );
            }
        }

        // Fallback: If producer not found or has no weight, return empty presence
        // This shouldn't happen in normal operation
        debug!("Building empty presence commitment (producer not in active set or no weight)");
        doli_core::presence::PresenceCommitment::empty()
    }

    /// Handle a detected equivocation (double signing)
    ///
    /// Creates and submits a slash transaction to the mempool when a producer
    /// is caught creating two different blocks for the same slot.
    async fn handle_equivocation(&mut self, proof: EquivocationProof) {
        // Log the equivocation with all details
        warn!(
            "SLASHING: Producer {} created two blocks for slot {}: {} and {}",
            crypto_hash(proof.producer.as_bytes()),
            proof.slot,
            proof.block_header_1.hash(),
            proof.block_header_2.hash()
        );

        // Check if the producer is actually registered (to avoid spam for unknown producers)
        let is_registered = {
            let producers = self.producer_set.read().await;
            producers.get_by_pubkey(&proof.producer).is_some()
        };

        if !is_registered {
            info!(
                "Equivocation by unregistered producer {} - not submitting slash tx",
                crypto_hash(proof.producer.as_bytes())
            );
            return;
        }

        // Create slash transaction using our producer key as reporter
        // If we don't have a producer key, we can't sign the slash tx
        let slash_tx = if let Some(ref reporter_key) = self.producer_key {
            proof.to_slash_transaction(reporter_key)
        } else {
            // Generate ephemeral keypair for reporting (anyone can report)
            let reporter_key = KeyPair::generate();
            proof.to_slash_transaction(&reporter_key)
        };

        let slash_tx_hash = slash_tx.hash();

        // Add to mempool for inclusion in next block
        // Use add_system_transaction since slash txs have no inputs/outputs
        let current_height = self.chain_state.read().await.best_height;
        let add_result = {
            let mut mempool = self.mempool.write().await;
            mempool.add_system_transaction(slash_tx.clone(), current_height)
        };

        match add_result {
            Ok(_hash) => {
                info!(
                    "Slash transaction {} submitted to mempool for producer {}",
                    slash_tx_hash,
                    crypto_hash(proof.producer.as_bytes())
                );

                // Broadcast the slash tx to the network
                if let Some(ref network) = self.network {
                    if let Err(e) = network.broadcast_transaction(slash_tx).await {
                        warn!("Failed to broadcast slash transaction: {}", e);
                    }
                }
            }
            Err(rpc::MempoolError::AlreadyExists) => {
                debug!("Slash transaction {} already in mempool", slash_tx_hash);
            }
            Err(e) => {
                warn!(
                    "Failed to add slash transaction {} to mempool: {}",
                    slash_tx_hash, e
                );
            }
        }
    }

    /// Run periodic tasks
    async fn run_periodic_tasks(&mut self) -> Result<()> {
        // Clean up sync manager
        self.sync_manager.write().await.cleanup();

        // Expire old mempool transactions
        self.mempool.write().await.expire_old();

        // Check if we need to request sync
        if let Some((peer_id, request)) = self.sync_manager.write().await.next_request() {
            if let Some(ref network) = self.network {
                let _ = network.request_sync(peer_id, request).await;
            }
        }

        Ok(())
    }

    /// Get current chain height
    pub async fn height(&self) -> u64 {
        self.chain_state.read().await.best_height
    }

    /// Get current best hash
    pub async fn best_hash(&self) -> Hash {
        self.chain_state.read().await.best_hash
    }

    /// Get sync state
    pub async fn sync_state(&self) -> network::SyncState {
        self.sync_manager.read().await.state().clone()
    }

    /// Get peer count
    pub async fn peer_count(&self) -> usize {
        if let Some(ref network) = self.network {
            network.peer_count().await
        } else {
            0
        }
    }

    /// Get mempool size
    pub async fn mempool_size(&self) -> usize {
        self.mempool.read().await.len()
    }

    /// Save state periodically (every STATE_SAVE_INTERVAL blocks)
    ///
    /// This provides crash resilience by persisting state to disk without
    /// waiting for graceful shutdown. Called after applying each block.
    async fn maybe_save_state(&mut self) -> Result<()> {
        self.blocks_since_save += 1;

        if self.blocks_since_save >= STATE_SAVE_INTERVAL {
            self.save_state().await?;
            self.blocks_since_save = 0;
        }

        Ok(())
    }

    /// Save all node state to disk
    async fn save_state(&self) -> Result<()> {
        debug!("Saving node state to disk...");

        // Save chain state
        let state_path = self.config.data_dir.join("chain_state.bin");
        self.chain_state.read().await.save(&state_path)?;

        // Save UTXO set
        let utxo_path = self.config.data_dir.join("utxo");
        self.utxo_set.read().await.save(&utxo_path)?;

        // Save producer set
        let producers_path = self.config.data_dir.join("producers.bin");
        self.producer_set.read().await.save(&producers_path)?;

        // Save claim registry
        let claims_path = self.config.data_dir.join("claims.bin");
        self.claim_registry.read().await.save(&claims_path)?;

        debug!("Node state saved");
        Ok(())
    }

    /// Shutdown the node
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down node...");

        // Set shutdown flag
        *self.shutdown.write().await = true;

        // Final state save
        self.save_state().await?;

        info!("Node shutdown complete");
        Ok(())
    }
}

// Note: Old epoch reward tests removed as part of Milestone 10
// (Remove Old Reward System). The new weighted presence reward
// system uses ClaimEpochReward transactions with validation in
// crates/core/src/validation.rs and tests in crates/core/src/rewards.rs.
