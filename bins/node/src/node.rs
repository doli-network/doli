//! Node implementation
//!
//! Integrates all DOLI components: storage, networking, RPC, mempool, and sync.
// v0.2.1-test: upgrade pipeline validation

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crypto::hash::{hash as crypto_hash, hash_with_domain};
use crypto::{Hash, KeyPair, PublicKey, ADDRESS_DOMAIN};
use doli_core::block::BlockBuilder;
use doli_core::consensus::{
    self, compute_tier1_set, construct_vdf_input, producer_tier, reward_epoch, ConsensusParams,
    DELEGATE_REWARD_PCT, SLOTS_PER_EPOCH, STAKER_REWARD_PCT, UNBONDING_PERIOD,
};
use doli_core::rewards::WeightedRewardCalculator;
use doli_core::tpop::calibration::VdfCalibrator;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::tpop::heartbeat::verify_hash_chain_vdf;
use doli_core::transaction::{RegistrationData, TxType};
use doli_core::types::UNITS_PER_COIN;
use doli_core::validation;
// ValidationMode is ready for use once merkle root computation is fixed
#[allow(unused_imports)]
use doli_core::validation::ValidationMode;
use doli_core::{
    AdaptiveGossip, Attestation, Block, BlockHeader, Network, ProducerAnnouncement, ProducerGSet,
    Transaction,
};
use doli_core::{DeterministicScheduler, ScheduledProducer};
use network::{
    EquivocationDetector, EquivocationProof, NetworkCommand, NetworkConfig, NetworkEvent,
    NetworkService, PeerId, ProductionAuthorization, ReorgResult, SyncConfig, SyncManager,
};
use rpc::{Mempool, MempoolPolicy, RpcContext, RpcServer, RpcServerConfig, SyncStatus};
use storage::{BlockStore, ChainState, ProducerSet, UtxoSet};
use updater::is_using_placeholder_keys;

use crate::updater as node_updater;
use vdf::{VdfOutput, VdfProof};

use crate::config::NodeConfig;
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
    _last_production_check: Instant,
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
    /// Consecutive slots where production was blocked due to fork detection
    /// (AheadOfPeers, SyncFailures, ChainMismatch). Triggers auto-resync when threshold exceeded.
    consecutive_fork_blocks: u32,
    /// Number of shallow fork rollbacks performed since last successful sync.
    /// Capped at MAX_SHALLOW_ROLLBACKS to prevent rolling back the entire chain.
    shallow_rollback_count: u32,
    /// Cached DeterministicScheduler (epoch, producer_count, total_bonds, scheduler)
    /// Rebuilt when epoch changes OR active producer set changes (new registrations, exits, slashing).
    cached_scheduler: Option<(u64, usize, u64, DeterministicScheduler)>,
    /// Our computed tier (1, 2, or 3). Recomputed at each epoch boundary.
    our_tier: u8,
    /// Last epoch for which we computed our tier (to detect epoch boundaries).
    last_tier_epoch: Option<u64>,
    /// Channel to forward gossip votes to the UpdateService
    vote_tx: Option<tokio::sync::mpsc::Sender<node_updater::VoteMessage>>,
    /// Shared pending update state from UpdateService (for RPC to read live)
    pending_update: Option<Arc<RwLock<Option<node_updater::PendingUpdate>>>>,
    /// Last time we attempted to redial bootstrap nodes (rate limiter)
    last_peer_redial: Option<Instant>,
    /// Last height at which each producer produced a block (for liveness filter).
    /// Populated from chain data in apply_block(), rebuilt from block_store on startup.
    /// Used by bootstrap scheduling to exclude stale producers from primary rotation.
    producer_liveness: HashMap<PublicKey, u64>,
    /// Height at which snap sync last completed. Used to determine validation mode:
    /// blocks near the tip (within 1 epoch / 360 blocks) get full VDF verification,
    /// older blocks get light validation (VDF skipped — already trusted via state root quorum).
    snap_sync_height: Option<u64>,
    /// Cached genesis VDF proof output (computed in background at startup during genesis).
    /// Used to create a zero-bond Registration TX that proves VDF work on-chain.
    genesis_vdf_output: Option<[u8; 32]>,
    /// Whether the genesis VDF Registration TX has been included in a produced block.
    genesis_vdf_submitted: bool,
    /// Cached state root, updated atomically after each block application.
    /// Avoids race conditions when GetStateRoot reads during apply_block.
    /// Tuple: (state_root, block_hash, block_height)
    cached_state_root: Arc<RwLock<Option<(Hash, Hash, u64)>>>,
}

/// How often to save state (every N blocks applied)
const STATE_SAVE_INTERVAL: u64 = 1;

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

        // Apply chainspec overrides (authoritative for non-mainnet networks)
        if let Some(ref spec) = config.chainspec {
            params.apply_chainspec(spec);
            info!(
                "Applied chainspec overrides: slot_duration={}s, bond={}, reward={}",
                params.slot_duration, params.initial_bond, params.initial_reward
            );
        }

        // Open storage
        let blocks_path = config.data_dir.join("blocks");
        let block_store = Arc::new(BlockStore::open(&blocks_path)?);

        // Load or create UTXO set
        // Priority: RocksDB dir > legacy utxo.bin (auto-migrate) > empty
        let utxo_rocks_path = config.data_dir.join("utxo_rocks");
        let utxo_path = config.data_dir.join("utxo.bin");
        let utxo_set = if utxo_rocks_path.exists() {
            // RocksDB backend already exists — open it
            info!("[UTXO] Opening RocksDB backend at {:?}", utxo_rocks_path);
            UtxoSet::open_rocksdb(&utxo_rocks_path)?
        } else if utxo_path.exists() {
            // Legacy utxo.bin exists — migrate to RocksDB
            info!("[UTXO] Migrating legacy utxo.bin → RocksDB...");
            let legacy = storage::InMemoryUtxoStore::load(&utxo_path)?;
            let rocks = storage::RocksDbUtxoStore::open(&utxo_rocks_path)?;
            rocks.import_from(legacy.iter());
            info!(
                "[UTXO] Migration complete: {} entries. Backing up utxo.bin",
                rocks.len()
            );
            // Rename old file so we don't re-migrate
            let backup_path = config.data_dir.join("utxo.bin.backup");
            std::fs::rename(&utxo_path, &backup_path)?;
            UtxoSet::RocksDb(rocks)
        } else {
            // Fresh node — create RocksDB backend
            info!(
                "[UTXO] Creating new RocksDB backend at {:?}",
                utxo_rocks_path
            );
            UtxoSet::open_rocksdb(&utxo_rocks_path)?
        };
        let utxo_set = Arc::new(RwLock::new(utxo_set));

        // Load or create chain state
        let state_path = config.data_dir.join("chain_state.bin");
        let mut chain_state = if state_path.exists() {
            ChainState::load(&state_path)?
        } else {
            let genesis_hash = crypto::hash::hash(b"DOLI Genesis");
            ChainState::new(genesis_hash)
        };
        let genesis_hash = chain_state.genesis_hash;

        // Verify chain state consistency with block store
        if chain_state.best_height > 0 {
            match block_store.get_block(&chain_state.best_hash) {
                Ok(Some(_tip_block)) => {
                    // Tip hash exists in store — chain state is consistent
                }
                Ok(None) => {
                    if chain_state.is_snap_synced() {
                        // Block store is intentionally empty after snap sync —
                        // this is NOT corruption. Re-seed the canonical index so
                        // set_canonical_chain can exit cleanly on the first
                        // post-snap-sync block, then proceed normally.
                        info!(
                            "Chain state from snap sync at height {} — block store empty by design, re-seeding index.",
                            chain_state.best_height
                        );
                        if let Err(e) = block_store
                            .seed_canonical_index(chain_state.best_hash, chain_state.best_height)
                        {
                            warn!("Failed to re-seed canonical index after snap sync: {}", e);
                        }
                    } else {
                        warn!(
                            "Chain state tip {} at height {} not found in block store. Recovering...",
                            chain_state.best_hash, chain_state.best_height
                        );
                        // Walk backwards through height index to find highest valid block
                        let mut recovered = false;
                        for h in (1..chain_state.best_height).rev() {
                            if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                                info!(
                                    "Recovered chain state to height {} (hash {})",
                                    h,
                                    block.hash()
                                );
                                chain_state.best_hash = block.hash();
                                chain_state.best_height = h;
                                chain_state.best_slot = block.header.slot;
                                recovered = true;
                                break;
                            }
                        }
                        if !recovered {
                            // Block store is empty but chain state has valid data.
                            // This can happen after crash recovery or force-resync when
                            // state files survived but block history was lost.
                            // Treat as snap-synced state: preserve balances/producers,
                            // blocks will rebuild from peers.
                            warn!(
                                "No blocks in store but chain state at height {}. \
                                 Marking as snap-synced to preserve state.",
                                chain_state.best_height
                            );
                            chain_state.mark_snap_synced(chain_state.best_height);
                            if let Err(e) = block_store.seed_canonical_index(
                                chain_state.best_hash,
                                chain_state.best_height,
                            ) {
                                warn!("Failed to seed canonical index: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Block store error during integrity check: {}. Resetting to genesis.",
                        e
                    );
                    chain_state = ChainState::new(genesis_hash);
                }
            }
        } else {
            // Chain state at genesis — check if block store has blocks (e.g., chain_state.bin
            // was not saved before shutdown). Recover to highest stored block.
            let mut recovered_height = 0u64;
            for h in 1..=1000 {
                if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                    chain_state.best_hash = block.hash();
                    chain_state.best_height = h;
                    chain_state.best_slot = block.header.slot;
                    recovered_height = h;
                } else {
                    break;
                }
            }
            if recovered_height > 0 {
                warn!(
                    "Recovered chain state from block store: height {} (chain_state.bin was missing/stale)",
                    recovered_height
                );
            }
        }

        // Apply slot_duration from chainspec if available (consensus-critical)
        // This ensures all nodes compute the same slot numbers regardless of local .env
        if let Some(slot_duration) = config.slot_duration_override {
            params.slot_duration = slot_duration;
            info!("Slot duration from chainspec: {}s", slot_duration);
        }

        // For devnet, handle genesis_time from multiple sources (in priority order):
        // 1. Chainspec override (config.genesis_time_override) - ensures all nodes use same time
        // 2. Stored state (chain_state.genesis_timestamp) - for rejoining existing network
        // 3. Dynamic (current time) - for new isolated networks
        if config.network == Network::Devnet && params.genesis_time == 0 {
            use std::time::{SystemTime, UNIX_EPOCH};
            if let Some(override_time) = config.genesis_time_override {
                // Use chainspec genesis time for coordinated startup
                params.genesis_time = override_time;
                info!(
                    "Devnet genesis time from chainspec: {}",
                    params.genesis_time
                );
            } else if chain_state.genesis_timestamp != 0 {
                // Use stored genesis timestamp from previous run
                params.genesis_time = chain_state.genesis_timestamp;
                info!(
                    "Devnet genesis time loaded from state: {}",
                    params.genesis_time
                );
            } else {
                // New devnet without chainspec - set genesis time to current timestamp (rounded to slot)
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                params.genesis_time = now - (now % params.slot_duration);
                info!("Devnet genesis time initialized: {}", params.genesis_time);
            }
        }

        // Rebuild producer liveness map from recent blocks in block_store.
        // Scans the last LIVENESS_WINDOW_MIN blocks to determine which producers
        // have been active recently. This is deterministic (same chain = same map).
        let producer_liveness = {
            let mut liveness: HashMap<PublicKey, u64> = HashMap::new();
            let tip = chain_state.best_height;
            let window = consensus::LIVENESS_WINDOW_MIN;
            let start = tip.saturating_sub(window).max(1);
            for h in start..=tip {
                if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                    liveness.insert(block.header.producer, h);
                }
            }
            if !liveness.is_empty() {
                info!(
                    "Rebuilt producer liveness from blocks {}-{}: {} producers tracked",
                    start,
                    tip,
                    liveness.len()
                );
            }
            liveness
        };

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
                        ProducerSet::with_genesis_producers(
                            genesis_producers,
                            config.network.bond_unit(),
                        )
                    } else {
                        ProducerSet::new()
                    }
                } else {
                    ProducerSet::new()
                }
            };
            Arc::new(RwLock::new(set))
        };

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

        // Create sync manager with default settings (2 slots/heights tolerance).
        // All networks use the same tolerance — recovery from forks is handled by
        // auto-resync (triggered by sync failure threshold), not by loose tolerances.
        let sync_config = SyncConfig::default();
        let sync_manager = Arc::new(RwLock::new(SyncManager::new(sync_config, genesis_hash)));

        // Configure bootstrap grace period from network params
        // This is the wait time at genesis before allowing production (chain evidence collection)
        {
            let mut sm = sync_manager.write().await;
            sm.set_bootstrap_grace_period_secs(params.bootstrap_grace_period_secs);

            // Configure min peers for production based on network and genesis phase.
            // During genesis, allow single-peer production since the network is
            // still bootstrapping with very few nodes. After the first epoch boundary,
            // recompute_tier() overrides with tier-aware minimums (Tier1=10, Tier2=5, Tier3=2).
            let in_genesis_at_start = {
                let state = chain_state.read().await;
                config.network.is_in_genesis(state.best_height + 1)
            };
            let min_peers = match config.network {
                Network::Devnet => 1,
                _ if in_genesis_at_start => 1,
                Network::Testnet | Network::Mainnet => 2,
            };
            sm.set_min_peers_for_production(min_peers);

            // Configure gossip timeout based on slot duration (P0 #3)
            // 18 slots = ~3 minutes on Mainnet (10s slots)
            // Scales down for Devnet/Testnet with faster slots
            let gossip_timeout = 18 * params.slot_duration;
            sm.set_gossip_activity_timeout_secs(gossip_timeout);

            // Initialize sync manager with current chain state (critical for restart correctness).
            // Without this, SyncManager starts at genesis and re-downloads the entire chain,
            // causing height double-counting (ISSUE-5).
            {
                let state = chain_state.read().await;
                if state.best_height > 0 {
                    sm.update_local_tip(state.best_height, state.best_hash, state.best_slot);
                    info!(
                        "Sync manager initialized at height {} (hash {})",
                        state.best_height,
                        &state.best_hash.to_string()[..16]
                    );
                }
            }
        }

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
            _last_production_check: Instant::now(),
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
            consecutive_fork_blocks: 0,
            shallow_rollback_count: 0,
            cached_scheduler: None,
            our_tier: 0, // Computed on first block application
            last_tier_epoch: None,
            vote_tx: None,
            pending_update: None,
            last_peer_redial: None,
            producer_liveness,
            snap_sync_height: None,
            genesis_vdf_output: None,
            genesis_vdf_submitted: false,
            cached_state_root: Arc::new(RwLock::new(None)),
        })
    }

    /// Set the vote forwarding channel (connects gossip votes to UpdateService)
    pub fn set_vote_tx(&mut self, tx: tokio::sync::mpsc::Sender<node_updater::VoteMessage>) {
        self.vote_tx = Some(tx);
    }

    /// Set the shared pending update state (connects UpdateService to RPC)
    pub fn set_pending_update(
        &mut self,
        pending: Arc<RwLock<Option<node_updater::PendingUpdate>>>,
    ) {
        self.pending_update = Some(pending);
    }

    /// Run the node
    pub async fn run(&mut self) -> Result<()> {
        info!("Node starting...");

        // Check for placeholder maintainer keys on mainnet
        if self.config.network == Network::Mainnet && is_using_placeholder_keys() {
            error!("CRITICAL: Placeholder maintainer keys detected!");
            error!("This node is NOT suitable for mainnet operation.");
            error!("Replace BOOTSTRAP_MAINTAINER_KEYS in doli-updater/src/lib.rs with real Ed25519 keys.");
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
            let genesis_active = {
                let state = self.chain_state.read().await;
                self.config.network.is_in_genesis(state.best_height + 1)
            };
            if self.config.network == Network::Testnet
                || self.config.network == Network::Devnet
                || genesis_active
            {
                let our_pubkey = *key.public_key();

                // Read our stored sequence from the persisted GSet so we resume
                // from where we left off. Without this, a restart resets to 0 and
                // every peer silently rejects our announcements as Duplicate until
                // our counter exceeds their stored sequence — causing us to vanish
                // from their active_producers() for potentially hours.
                let stored_seq = {
                    let gset = self.producer_gset.read().await;
                    gset.sequence_for(&our_pubkey)
                };
                let start_seq = stored_seq + 1;
                self.announcement_sequence
                    .store(start_seq, std::sync::atomic::Ordering::SeqCst);

                let announcement =
                    ProducerAnnouncement::new(key, self.config.network.id(), start_seq);

                // Add to GSet (new format)
                {
                    let mut gset = self.producer_gset.write().await;
                    let _ = gset.merge_one(announcement.clone());
                }
                *self.our_announcement.write().await = Some(announcement);

                // Also add to legacy known_producers for compatibility during transition
                let mut known = self.known_producers.write().await;
                if !known.contains(&our_pubkey) {
                    known.push(our_pubkey);
                    known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                    let pubkey_hash = crypto_hash(our_pubkey.as_bytes());
                    info!(
                        "Registered self as bootstrap producer: {} (now {} known)",
                        &pubkey_hash.to_hex()[..16],
                        known.len()
                    );
                    self.last_producer_list_change = Some(Instant::now());
                }
                drop(known);

                // IMMEDIATE ANTI-ENTROPY: Broadcast our announcement via new format
                if let Some(ref network) = self.network {
                    let announcements = {
                        let gset = self.producer_gset.read().await;
                        gset.export()
                    };
                    if !announcements.is_empty() {
                        info!(
                            "Broadcasting initial producer announcements ({} producers)",
                            announcements.len()
                        );
                        let _ = network
                            .broadcast_producer_announcements(announcements)
                            .await;
                    }
                }
            }
        }

        // Compute genesis VDF proof in background during genesis phase.
        // This takes ~30s for 5M iterations. The proof will be embedded in a
        // Registration TX in the next block this producer creates.
        if let Some(ref key) = self.producer_key {
            let genesis_active = {
                let state = self.chain_state.read().await;
                self.config.network.is_in_genesis(state.best_height + 1)
            };
            if genesis_active {
                let our_pubkey = *key.public_key();
                let iterations = self.config.network.vdf_register_iterations();
                let vdf_input = vdf::registration_input(&our_pubkey, 0);
                info!(
                    "Computing genesis registration VDF proof ({} iterations)...",
                    iterations
                );
                let output =
                    tokio::task::spawn_blocking(move || hash_chain_vdf(&vdf_input, iterations))
                        .await?;
                self.genesis_vdf_output = Some(output);
                info!(
                    "Genesis VDF proof computed for {}",
                    hex::encode(&our_pubkey.as_bytes()[..8])
                );
            }
        }

        // NOTE: Do NOT call recompute_tier() here. At startup the on-chain ProducerSet
        // is incomplete (not synced yet). producer_tier() would default to Tier 3
        // (header-only), causing reconfigure_topics_for_tier(3) to unsubscribe from
        // BLOCKS_TOPIC — the node would stop receiving blocks and get stuck.
        // Tier computation runs safely at epoch boundaries (after sync completes).

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
        network_config.node_key_path = Some(self.config.data_dir.join("node_key"));
        network_config.peer_cache_path = Some(self.config.data_dir.join("peers.cache"));

        // Gossip mesh params from NetworkParams (env vars / .env / chainspec / defaults)
        let net_params = self.config.network.params();
        network_config.mesh_n = net_params.mesh_n;
        network_config.mesh_n_low = net_params.mesh_n_low;
        network_config.mesh_n_high = net_params.mesh_n_high;
        network_config.gossip_lazy = net_params.gossip_lazy;

        // NAT traversal: enable relay server if configured (for public/bootstrap nodes)
        if self.config.relay_server {
            network_config.nat_config = network::NatConfig::relay_server();
        }

        // External address: advertise a specific public address to peers
        if let Some(ref addr_str) = self.config.external_address {
            match addr_str.parse::<network::Multiaddr>() {
                Ok(addr) => {
                    network_config.external_address = Some(addr);
                }
                Err(e) => {
                    warn!("Invalid --external-address '{}': {}", addr_str, e);
                }
            }
        }

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
        let mut context = RpcContext::new_for_network(
            self.chain_state.clone(),
            self.block_store.clone(),
            self.utxo_set.clone(),
            self.mempool.clone(),
            self.params.clone(),
            self.config.network,
        )
        .with_blocks_per_reward_epoch(self.config.network.blocks_per_reward_epoch())
        .with_coinbase_maturity(self.config.network.coinbase_maturity())
        .with_bond_unit(self.config.network.bond_unit())
        .with_producer_set(self.producer_set.clone())
        .with_sync_status(sync_status_fn);

        // Wire up transaction broadcast so RPC-submitted txs are gossiped to peers
        if let Some(ref network) = self.network {
            let cmd_tx = network.command_sender();
            context = context.with_broadcast(move |tx: Transaction| {
                let cmd_tx = cmd_tx.clone();
                tokio::spawn(async move {
                    let _ = cmd_tx.send(NetworkCommand::BroadcastTransaction(tx)).await;
                });
            });
        }

        // Wire up vote broadcast so RPC-submitted votes are gossiped to peers
        if let Some(ref network) = self.network {
            let cmd_tx = network.command_sender();
            context = context.with_broadcast_vote(move |vote_data: Vec<u8>| {
                let cmd_tx = cmd_tx.clone();
                tokio::spawn(async move {
                    let _ = cmd_tx.send(NetworkCommand::BroadcastVote(vote_data)).await;
                });
            });
        }

        // Wire up update status callback to read live state from UpdateService
        if let Some(ref pending) = self.pending_update {
            let pending = pending.clone();
            let producer_set = self.producer_set.clone();
            context = context.with_update_status(move || {
                let pending_guard = pending.try_read();
                match pending_guard {
                    Ok(guard) => match guard.as_ref() {
                        Some(p) => {
                            let total_producers = producer_set
                                .try_read()
                                .map(|set| set.active_count())
                                .unwrap_or(0);
                            let veto_active =
                                !p.approved && !node_updater::veto_period_ended(&p.release);
                            serde_json::json!({
                                "pending_update": {
                                    "version": p.release.version,
                                    "published_at": p.release.published_at,
                                    "changelog": p.release.changelog,
                                    "approved": p.approved,
                                    "days_remaining": p.days_remaining(),
                                    "hours_remaining": p.hours_remaining(),
                                },
                                "veto_period_active": veto_active,
                                "veto_count": p.vote_tracker.veto_count(),
                                "veto_percent": p.vote_tracker.veto_percent(total_producers) as f64,
                                "enforcement": p.enforcement.as_ref().map(|e| serde_json::json!({
                                    "active": e.active,
                                    "min_version": e.min_version,
                                })),
                            })
                        }
                        None => serde_json::json!({
                            "pending_update": null,
                            "veto_period_active": false,
                            "veto_count": 0,
                            "veto_percent": 0.0
                        }),
                    },
                    Err(_) => serde_json::json!({
                        "pending_update": null,
                        "veto_period_active": false,
                        "veto_count": 0,
                        "veto_percent": 0.0
                    }),
                }
            });
        }

        let server = RpcServer::new(rpc_config, context);
        info!("Starting RPC server on {}", listen_addr);
        server.spawn();

        Ok(())
    }

    /// Recompute our tier classification from the active producer set.
    /// Called once per epoch boundary (when `height / SLOTS_PER_EPOCH` changes).
    async fn recompute_tier(&mut self, height: u64) {
        let current_epoch = height / SLOTS_PER_EPOCH as u64;
        if self.last_tier_epoch == Some(current_epoch) {
            return; // Already computed for this epoch
        }

        let our_pubkey = match &self.producer_key {
            Some(kp) => *kp.public_key(),
            None => {
                self.our_tier = 0; // Non-producer
                self.last_tier_epoch = Some(current_epoch);
                return;
            }
        };

        let producers = self.producer_set.read().await;
        let active = producers.active_producers_at_height(height);
        let producers_with_weights: Vec<(PublicKey, u64)> = active
            .iter()
            .map(|p| (p.public_key, p.selection_weight()))
            .collect();
        drop(producers);

        let tier1_set = compute_tier1_set(&producers_with_weights);
        // Build sorted-by-weight list for producer_tier()
        let mut all_sorted = producers_with_weights.clone();
        all_sorted.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
        });
        let all_sorted_pks: Vec<PublicKey> = all_sorted.into_iter().map(|(pk, _)| pk).collect();

        let new_tier = producer_tier(&our_pubkey, &tier1_set, &all_sorted_pks);

        if self.our_tier != new_tier {
            info!(
                "Tier classification changed: {} -> {} (epoch {})",
                self.our_tier, new_tier, current_epoch
            );
            let mut sync = self.sync_manager.write().await;
            sync.set_tier(new_tier, producers_with_weights.len());
            // During genesis, keep min_peers=1 to allow bootstrapping with fewer nodes
            if self.config.network.is_in_genesis(height) {
                sync.set_min_peers_for_production(1);
            }
            drop(sync);

            // Reconfigure gossipsub topic subscriptions for the new tier
            // Tier 2 nodes get a deterministic region assignment
            let region = if new_tier == 2 {
                Some(doli_core::consensus::producer_region(&our_pubkey))
            } else {
                None
            };
            if let Some(ref network) = self.network {
                let _ = network.reconfigure_tier(new_tier, region).await;
            }
        }

        self.our_tier = new_tier;
        self.last_tier_epoch = Some(current_epoch);
    }

    /// Create an attestation for a block and broadcast it to the network.
    async fn create_and_broadcast_attestation(&self, block_hash: Hash, slot: u32, height: u64) {
        let (private_key, public_key, weight) = match &self.producer_key {
            Some(kp) => {
                let pk = *kp.public_key();
                let producers = self.producer_set.read().await;
                let w = producers
                    .get_by_pubkey(&pk)
                    .map(|p| p.selection_weight())
                    .unwrap_or(0);
                (kp.private_key().clone(), pk, w)
            }
            None => return, // Non-producer can't attest
        };

        if weight == 0 {
            return; // Not active, skip attestation
        }

        let attestation =
            Attestation::new(block_hash, slot, height, weight, &private_key, public_key);

        // Add our own weight to finality tracker
        {
            let mut sync = self.sync_manager.write().await;
            sync.add_attestation_weight(&block_hash, weight);
        }

        // Broadcast to network
        if let Some(ref network) = self.network {
            let _ = network.broadcast_attestation(attestation.to_bytes()).await;
        }
    }

    /// Main event loop
    async fn run_event_loop(&mut self) -> Result<()> {
        info!("Entering main event loop");

        // Check production opportunity - faster for devnet to catch the 700ms heartbeat window
        let production_interval = if self.config.network == Network::Devnet {
            Duration::from_millis(200) // 5 checks per second for devnet
        } else {
            Duration::from_secs(1)
        };
        let mut production_timer = tokio::time::interval(production_interval);

        // Gossip our producer identity using adaptive intervals
        // This ensures nodes that aren't directly connected (e.g., Node 2 -> Node 1 -> Node 3)
        // learn about each other through the GossipSub mesh relay
        // Phase 1: Use AdaptiveGossip for dynamic interval adjustment
        let mut current_gossip_interval = self.adaptive_gossip.read().await.interval();
        let mut gossip_timer = tokio::time::interval(current_gossip_interval);

        loop {
            // Check shutdown flag
            if *self.shutdown.read().await {
                break;
            }

            // Use select! to handle network events, production timer, and gossip timer
            // BIASED SELECT: Network events have priority over production
            //
            // This is critical for preventing propagation race forks. Without bias,
            // tokio::select! can choose the production branch even when a NewBlock
            // event is ready in the network queue. This causes nodes to produce
            // blocks on stale chain tips, creating forks.
            //
            // With biased select, we always process pending network events first,
            // ensuring the chain tip is up-to-date before attempting production.
            tokio::select! {
                biased;

                // Network event received (HIGHEST PRIORITY)
                // Must be first to ensure blocks are processed before production
                event = async {
                    if let Some(ref mut network) = self.network {
                        network.next_event().await
                    } else {
                        std::future::pending::<Option<NetworkEvent>>().await
                    }
                } => {
                    if let Some(event) = event {
                        self.handle_network_event(event).await?;
                    }
                }

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

                // Gossip timer tick - ANTI-ENTROPY: broadcast producer view
                // Phase 1: Uses adaptive intervals based on network activity
                // Phase 2: Uses delta sync (bloom filter) for large networks
                _ = gossip_timer.tick() => {
                    let genesis_active = {
                        let state = self.chain_state.read().await;
                        self.config.network.is_in_genesis(state.best_height + 1)
                    };
                    if self.config.network == Network::Testnet || self.config.network == Network::Devnet || genesis_active {
                        if let Some(ref network) = self.network {
                            // Update our producer announcement if we're a producer
                            if let Some(ref key) = self.producer_key {
                                let seq = self.announcement_sequence.fetch_add(1, Ordering::SeqCst);
                                let announcement = ProducerAnnouncement::new(key, self.config.network.id(), seq);

                                {
                                    let mut gset = self.producer_gset.write().await;
                                    let _ = gset.merge_one(announcement.clone());

                                    // Purge ghost producers: entries older than 4 hours
                                    // (2x the active_producers liveness window of 7200s).
                                    // Without this, nodes that announced once and disappeared
                                    // persist forever, inflating the slot % n denominator.
                                    let purged = gset.purge_stale(14400);
                                    if purged > 0 {
                                        info!("GSet: purged {} ghost producer(s)", purged);
                                        if let Err(e) = gset.persist_to_disk() {
                                            warn!("Failed to persist GSet after purge: {}", e);
                                        }
                                    }
                                }

                                *self.our_announcement.write().await = Some(announcement);
                            }

                            // Get adaptive gossip settings and producer count
                            let (use_delta, producer_count) = {
                                let adaptive = self.adaptive_gossip.read().await;
                                let gset = self.producer_gset.read().await;
                                (adaptive.use_delta_sync(), gset.len())
                            };

                            // Phase 2: Choose sync strategy based on network size
                            // Delta sync is more efficient for larger networks (>50 producers)
                            if use_delta && producer_count > 50 {
                                // DELTA SYNC: Send bloom filter, peers respond with missing announcements
                                let bloom = {
                                    let gset = self.producer_gset.read().await;
                                    gset.to_bloom_filter()
                                };
                                debug!(
                                    "Delta sync: broadcasting bloom filter ({} bytes) for {} producers",
                                    bloom.size_bytes(),
                                    producer_count
                                );
                                let _ = network.broadcast_producer_digest(bloom).await;
                            } else {
                                // FULL SYNC: Send all announcements (better for small networks)
                                let announcements = {
                                    let gset = self.producer_gset.read().await;
                                    gset.export()
                                };

                                if !announcements.is_empty() {
                                    debug!(
                                        "Full sync: broadcasting {} producer announcements",
                                        announcements.len()
                                    );
                                    let _ = network.broadcast_producer_announcements(announcements).await;
                                }
                            }

                            // Log producer schedule and detect divergence between GSet and known_producers
                            let gset_list = {
                                let gset = self.producer_gset.read().await;
                                gset.sorted_producers()
                            };
                            let known_list = self.known_producers.read().await.clone();
                            if !gset_list.is_empty() || !known_list.is_empty() {
                                let gset_hashes: Vec<String> = gset_list.iter()
                                    .map(|p| crypto_hash(p.as_bytes()).to_hex()[..8].to_string())
                                    .collect();
                                let known_hashes: Vec<String> = known_list.iter()
                                    .map(|p| crypto_hash(p.as_bytes()).to_hex()[..8].to_string())
                                    .collect();
                                if gset_hashes != known_hashes {
                                    warn!(
                                        "Producer schedule DIVERGENCE: gset={:?} (count={}) vs known={:?} (count={})",
                                        gset_hashes, gset_list.len(), known_hashes, known_list.len()
                                    );
                                } else {
                                    info!(
                                        "Producer schedule view: {:?} (count={}, source=gset)",
                                        gset_hashes, gset_list.len()
                                    );
                                }
                            }

                            // Phase 1: Update gossip interval if adaptive gossip changed it
                            let new_interval = self.adaptive_gossip.read().await.interval();
                            if new_interval != current_gossip_interval {
                                debug!(
                                    "Adaptive gossip: interval changed {:?} -> {:?}",
                                    current_gossip_interval, new_interval
                                );
                                current_gossip_interval = new_interval;
                                gossip_timer = tokio::time::interval(current_gossip_interval);
                            }
                        }
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

                // Enable bootstrap gate in SyncManager - production will be blocked
                // until we receive at least one peer status response
                self.sync_manager.write().await.set_peer_connected();

                // Request status from the new peer to learn their chain state
                // Include our producer pubkey so peers can discover us before blocks are exchanged
                let genesis_hash = self.chain_state.read().await.genesis_hash;
                let status_request = if let Some(ref key) = self.producer_key {
                    network::protocols::StatusRequest::with_producer(
                        self.config.network.id(),
                        genesis_hash,
                        *key.public_key(),
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

                // Reconnect to bootstrap nodes if we lost all peers
                let peer_count = self.sync_manager.read().await.peer_count();
                if peer_count == 0 && !self.config.bootstrap_nodes.is_empty() {
                    info!("Lost all peers — scheduling reconnect to bootstrap nodes");
                    if let Some(ref network) = self.network {
                        for addr in &self.config.bootstrap_nodes {
                            if let Err(e) = network.connect(addr).await {
                                warn!("Failed to reconnect to bootstrap {}: {}", addr, e);
                            }
                        }
                    }
                }
            }

            NetworkEvent::NewBlock(block, source_peer) => {
                debug!("Received new block: {} from {}", block.hash(), source_peer);
                // Update network tip slot from gossip - this tells us what slot the network has reached
                // even if we don't know which specific peer sent the block.
                // This is critical for the "behind peers" production safety check.
                //
                // Note: Height is updated when blocks are successfully applied (in apply_block).
                {
                    let mut sync = self.sync_manager.write().await;
                    sync.refresh_all_peers();
                    sync.update_network_tip_slot(block.header.slot);
                    sync.note_block_received_via_gossip();
                }
                self.handle_new_block(block, source_peer).await?;
            }

            NetworkEvent::NewHeader(header) => {
                debug!(
                    "Header pre-announcement: slot={} producer={:.16} hash={:.16}",
                    header.slot,
                    hex::encode(header.producer.as_bytes()),
                    hex::encode(header.hash().as_bytes()),
                );
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
                {
                    let mut sync = self.sync_manager.write().await;
                    sync.add_peer(
                        peer_id,
                        status.best_height,
                        status.best_hash,
                        status.best_slot,
                    );
                    // CRITICAL: Notify SyncManager that we received a valid peer status.
                    // This satisfies the bootstrap gate and allows production to proceed.
                    sync.note_peer_status_received();
                }

                // BOOTSTRAP PRODUCER DISCOVERY: If the peer is a producer, add them to
                // known_producers. This allows nodes to discover each other
                // before any blocks are exchanged, solving the chicken-and-egg problem
                // where blocks can't be applied (they look like forks) because producers
                // don't know about each other.
                if let Some(ref producer_pubkey) = status.producer_pubkey {
                    let genesis_active = {
                        let state = self.chain_state.read().await;
                        self.config.network.is_in_genesis(state.best_height + 1)
                    };
                    if self.config.network == Network::Testnet
                        || self.config.network == Network::Devnet
                        || genesis_active
                    {
                        let mut known = self.known_producers.write().await;
                        if !known.contains(producer_pubkey) {
                            known.push(*producer_pubkey);
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
                    let genesis_active = {
                        let state = self.chain_state.read().await;
                        self.config.network.is_in_genesis(state.best_height + 1)
                    };
                    if self.config.network == Network::Testnet
                        || self.config.network == Network::Devnet
                        || genesis_active
                    {
                        let mut known = self.known_producers.write().await;
                        if !known.contains(producer_pubkey) {
                            known.push(*producer_pubkey);
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
                        producer_pubkey: Some(*key.public_key()),
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
                // P1 #5: Note that this peer is sending data (active), not just reachable
                self.sync_manager
                    .write()
                    .await
                    .note_block_received_from_peer(peer_id);

                let blocks = self
                    .sync_manager
                    .write()
                    .await
                    .handle_response(peer_id, response);
                for block in blocks {
                    // Route through handle_new_block for orphan/fork detection.
                    // For normal sync blocks that build on tip, this falls through
                    // to apply_block unchanged. For orphan blocks (e.g., peer's tip
                    // when we're on a fork), they get cached and trigger fork recovery.
                    self.handle_new_block(block, peer_id).await?;
                }

                // Check if snap sync produced a ready snapshot
                let snap = self.sync_manager.write().await.take_snap_snapshot();
                if let Some(snapshot) = snap {
                    self.apply_snap_snapshot(snapshot).await?;
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
                let genesis_active = {
                    let state = self.chain_state.read().await;
                    self.config.network.is_in_genesis(state.best_height + 1)
                };
                if self.config.network == Network::Testnet
                    || self.config.network == Network::Devnet
                    || genesis_active
                {
                    let changed = {
                        let mut known = self.known_producers.write().await;
                        let mut changed = false;

                        // CRDT MERGE: Add any producers we don't already know about
                        for producer in &remote_list {
                            if !known.contains(producer) {
                                known.push(*producer);
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
                debug!("Received vote message ({} bytes)", vote_data.len());
                if let Some(ref vote_tx) = self.vote_tx {
                    match serde_json::from_slice::<node_updater::VoteMessage>(&vote_data) {
                        Ok(vote_msg) => {
                            info!(
                                "Vote received via gossip: {} vote for v{} from {}",
                                if vote_msg.vote == node_updater::Vote::Veto {
                                    "VETO"
                                } else {
                                    "APPROVE"
                                },
                                vote_msg.version,
                                &vote_msg.producer_id[..16.min(vote_msg.producer_id.len())]
                            );
                            let _ = vote_tx.try_send(vote_msg);
                        }
                        Err(e) => {
                            debug!("Failed to decode vote message: {}", e);
                        }
                    }
                }
            }

            // NOTE: Heartbeats removed in deterministic scheduler model
            // Rewards go 100% to block producer via coinbase
            NetworkEvent::NewHeartbeat(_) => {
                // Ignored - deterministic scheduler model doesn't use heartbeats
            }
            NetworkEvent::NewAttestation(data) => {
                // Decode and apply attestation for finality gadget
                if let Some(attestation) = doli_core::Attestation::from_bytes(&data) {
                    if attestation.verify().is_ok() {
                        let mut sync = self.sync_manager.write().await;
                        sync.add_attestation_weight(
                            &attestation.block_hash,
                            attestation.attester_weight,
                        );
                    } else {
                        debug!("Received invalid attestation signature");
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle a new block from the network
    async fn handle_new_block(&mut self, block: Block, source_peer: PeerId) -> Result<()> {
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
                // Reorg detection failed (parent not in our recent blocks).
                // Try active fork recovery: walk backward through parent chain
                // from this orphan block until we connect to our chain.
                // Use source_peer (who gossiped this block) — they have the fork chain.
                let can_start = self.sync_manager.read().await.can_start_fork_recovery();
                if can_start {
                    let started = self
                        .sync_manager
                        .write()
                        .await
                        .start_fork_recovery(block.clone(), source_peer);
                    if started {
                        info!(
                            "Fork recovery started: walking parents from block {} (asking source peer {})",
                            block_hash, source_peer
                        );
                        return Ok(());
                    }
                }

                // Fallback: Check if we're likely on a fork by looking at RECENT orphan blocks.
                // Only blocks near our current slot count as fork evidence.
                // Old blocks from syncing peers are NOT fork evidence.
                let our_slot = self.chain_state.read().await.best_slot;
                let our_height = self.chain_state.read().await.best_height;
                let cache_size = {
                    let cache = self.fork_block_cache.read().await;
                    let slot_window = 30u32; // only count blocks within last 30 slots (~5 min)
                    let min_slot = our_slot.saturating_sub(slot_window);
                    cache.values().filter(|b| b.header.slot >= min_slot).count()
                };

                // Many orphan blocks indicate we're on a minority fork.
                // Stale chain detector + fork recovery handle this — no genesis resync.
                if cache_size >= 10 && cache_size % 10 == 0 {
                    warn!(
                        "Fork detected: {} orphan blocks don't build on our chain (height {}). Relying on fork recovery + stale chain sync.",
                        cache_size, our_height
                    );
                }

                // Try fork recovery for the orphan block
                self.try_trigger_fork_recovery().await;

                if cache_size >= 2 {
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

        // Validate producer eligibility before applying
        if let Err(e) = self.check_producer_eligibility(&block).await {
            warn!("Rejected gossip block at slot {}: {}", block.header.slot, e);
            return Ok(());
        }

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
            let block = self
                .block_store
                .get_block_by_height(height)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Reorg UTXO rebuild: missing block at height {} (store corrupted)",
                        height
                    )
                })?;
            for (tx_index, tx) in block.transactions.iter().enumerate() {
                let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                // For non-reward-minting txs, spend inputs first
                if !is_reward_tx {
                    if let Err(e) = new_utxo.spend_transaction(tx) {
                        error!(
                            "Reorg UTXO rebuild failed at height {}: {} - aborting reorg",
                            height, e
                        );
                        return Err(anyhow::anyhow!("UTXO rebuild failed: {}", e));
                    }
                }
                new_utxo.add_transaction(tx, height, is_reward_tx);
            }
        }

        // Apply new blocks to the temporary UTXO set first (validation)
        let mut temp_height = target_height;
        for block in &new_blocks {
            temp_height += 1;
            for (tx_index, tx) in block.transactions.iter().enumerate() {
                let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                if !is_reward_tx {
                    if let Err(e) = new_utxo.spend_transaction(tx) {
                        error!(
                            "Reorg validation failed for block {}: {} - aborting reorg",
                            block.hash(),
                            e
                        );
                        return Err(anyhow::anyhow!("Reorg validation failed: {}", e));
                    }
                }
                new_utxo.add_transaction(tx, temp_height, is_reward_tx);
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
                        let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                        if !is_reward_tx {
                            let _ = utxo.spend_transaction(tx);
                        }
                        utxo.add_transaction(tx, height, is_reward_tx);
                    }
                }
            }
        }

        // Rebuild producer set to match common ancestor state (all TX types)
        {
            let mut producers = self.producer_set.write().await;
            self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
        }

        // Now apply the new blocks through normal path
        // Note: we skip check_producer_eligibility here because the fork blocks were
        // validated when originally produced, and re-validating against rolled-back
        // state uses the wrong producer set (common ancestor, not fork chain).
        info!("Applying {} new blocks from fork", new_blocks.len());
        for block in new_blocks {
            if let Err(e) = self.apply_block(block).await {
                error!(
                    "Reorg apply_block failed: {} — state is at common ancestor + applied blocks, sync will catch up",
                    e
                );
                // State is consistent (common ancestor + whatever blocks succeeded).
                // Don't propagate error — let normal sync fill the gap.
                return Ok(());
            }
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

    /// Execute a fork sync reorg: the binary search found the common ancestor,
    /// canonical blocks have been downloaded. Build a ReorgResult and call execute_reorg.
    async fn execute_fork_sync_reorg(
        &mut self,
        result: network::sync::ForkSyncResult,
    ) -> Result<()> {
        let current_height = self.chain_state.read().await.best_height;
        let rollback_depth = current_height.saturating_sub(result.ancestor_height);

        info!(
            "Fork sync reorg: rolling back {} blocks (height {} → {}), \
             applying {} canonical blocks",
            rollback_depth,
            current_height,
            result.ancestor_height,
            result.canonical_blocks.len()
        );

        if result.canonical_blocks.is_empty() {
            warn!("Fork sync: no canonical blocks to apply — aborting reorg");
            // Still reset sync so we can try again
            let mut sync = self.sync_manager.write().await;
            sync.reset_sync_for_rollback();
            return Ok(());
        }

        // Build rollback list: our blocks from current_height down to ancestor+1
        let mut rollback = Vec::new();
        for height in (result.ancestor_height + 1..=current_height).rev() {
            if let Some(hash) = self.block_store.get_hash_by_height(height)? {
                rollback.push(hash);
            }
        }

        // Insert canonical blocks into fork_block_cache so execute_reorg can find them
        let new_block_hashes: Vec<crypto::Hash> =
            result.canonical_blocks.iter().map(|b| b.hash()).collect();
        {
            let mut cache = self.fork_block_cache.write().await;
            for block in &result.canonical_blocks {
                cache.insert(block.hash(), block.clone());
            }
        }

        // Use the last canonical block as the triggering block
        let trigger_block = result.canonical_blocks.last().unwrap().clone();

        let reorg_result = network::sync::ReorgResult {
            rollback,
            common_ancestor: result.ancestor_hash,
            new_blocks: new_block_hashes,
            weight_delta: 1, // Positive = new chain is preferred (peers chose it)
        };

        // Execute the reorg using the existing atomic reorg path
        self.execute_reorg(reorg_result, trigger_block).await?;

        // Reset sync state so normal sync can resume from the new tip
        {
            let mut sync = self.sync_manager.write().await;
            sync.reset_sync_for_rollback();
            let (height, hash, slot) = {
                let state = self.chain_state.read().await;
                (state.best_height, state.best_hash, state.best_slot)
            };
            sync.update_local_tip(height, hash, slot);
        }

        // Reset shallow rollback counter since we successfully recovered
        self.shallow_rollback_count = 0;
        self.consecutive_fork_blocks = 0;

        info!(
            "Fork sync reorg complete: now at height {}",
            self.chain_state.read().await.best_height
        );

        Ok(())
    }

    /// Handle a completed fork recovery — evaluate the fork chain and reorg if heavier.
    ///
    /// Called when the parent chain walk connects to a block in our block_store.
    /// Records weights, moves blocks to fork_block_cache, plans reorg, executes if heavier.
    async fn handle_completed_fork_recovery(
        &mut self,
        recovery: network::sync::CompletedRecovery,
    ) -> Result<()> {
        let fork_len = recovery.blocks.len();
        info!(
            "Fork recovery complete: {} blocks connected at {}",
            fork_len,
            &recovery.connection_point.to_string()[..16]
        );

        let current_height = self.chain_state.read().await.best_height;
        let current_tip = self.chain_state.read().await.best_hash;

        // 1. Record fork blocks in reorg_handler with weights (forward order for correct accumulation)
        let mut last_block_weight = 1u64;
        {
            let producers = self.producer_set.read().await;
            let mut sync = self.sync_manager.write().await;
            for block in &recovery.blocks {
                let weight = producers
                    .get_by_pubkey(&block.header.producer)
                    .map(|p| p.effective_weight(current_height + 1))
                    .unwrap_or(1);
                sync.record_fork_block_weight(block.hash(), block.header.prev_hash, weight);
                last_block_weight = weight;
            }
        }

        // 2. Move fork blocks to fork_block_cache (execute_reorg reads from here)
        {
            let mut cache = self.fork_block_cache.write().await;
            for block in &recovery.blocks {
                cache.insert(block.hash(), block.clone());
            }
        }

        // 3. Try simple reorg first (works for single-block forks within recent_blocks)
        let fork_tip = recovery.blocks.last().unwrap();
        let simple_reorg = {
            let sync = self.sync_manager.read().await;
            sync.reorg_handler()
                .check_reorg_weighted(fork_tip, current_tip, last_block_weight)
        };

        if let Some(result) = simple_reorg {
            if result.weight_delta > 0 {
                info!(
                    "Fork is heavier (+{}) via simple reorg — executing: rollback={}, new={}",
                    result.weight_delta,
                    result.rollback.len(),
                    result.new_blocks.len()
                );
                let trigger = fork_tip.clone();
                self.execute_reorg(result, trigger).await?;
            } else {
                info!(
                    "Fork is lighter ({}) — keeping current chain",
                    result.weight_delta
                );
            }
            return Ok(());
        }

        // 4. Fall back to plan_reorg for deeper forks
        let fork_tip_hash = fork_tip.hash();
        let reorg_result = {
            let sync = self.sync_manager.read().await;
            let store = &self.block_store;
            sync.reorg_handler()
                .plan_reorg(current_tip, fork_tip_hash, |hash| {
                    store.get_header(hash).ok().flatten().map(|h| h.prev_hash)
                })
        };

        // 5. Execute reorg if fork is heavier
        match reorg_result {
            Some(result) if result.weight_delta > 0 => {
                info!(
                    "Fork is heavier (+{}) — executing reorg: rollback={}, new={}",
                    result.weight_delta,
                    result.rollback.len(),
                    result.new_blocks.len()
                );
                let trigger = recovery.blocks.last().unwrap().clone();
                self.execute_reorg(result, trigger).await?;
            }
            Some(result) => {
                info!(
                    "Fork is lighter ({}) — keeping current chain",
                    result.weight_delta
                );
            }
            None => {
                warn!("Could not plan reorg from recovered fork — common ancestor not found");
            }
        }

        Ok(())
    }

    /// Try to start fork recovery from cached orphan blocks.
    /// Called from production gate when fork is detected (ChainMismatch, AheadOfPeers).
    async fn try_trigger_fork_recovery(&mut self) {
        let can_start = self.sync_manager.read().await.can_start_fork_recovery();
        if !can_start {
            return;
        }
        let orphan = {
            let cache = self.fork_block_cache.read().await;
            cache.values().next().cloned()
        };
        if let Some(orphan) = orphan {
            let peer = self.sync_manager.read().await.best_peer_for_recovery();
            if let Some(peer) = peer {
                let started = self
                    .sync_manager
                    .write()
                    .await
                    .start_fork_recovery(orphan, peer);
                if started {
                    info!("Fork recovery triggered from production gate");
                }
            }
        }
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
                    // Validate producer eligibility
                    if let Err(e) = self.check_producer_eligibility(&block).await {
                        warn!(
                            "Cached chain rejected block at slot {}: {}",
                            block.header.slot, e
                        );
                        anyhow::bail!("Cached chain contains invalid producer: {}", e);
                    }
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
    /// It performs a COMPLETE state reset to genesis and re-syncs from peers.
    ///
    /// ## Defense-in-Depth: Complete State Reset
    ///
    /// This function resets ALL chain-dependent state to ensure semantic completeness:
    /// - chain_state: height, hash, slot → genesis
    /// - utxo_set: cleared completely
    /// - producer_set: cleared (keeping exit_history for anti-Sybil)
    ///
    /// Auto-resync when fork detection has blocked production for too long.
    ///
    /// Triggers `force_resync_from_genesis()` after N consecutive fork-blocked
    /// slots, respecting cooldown to prevent resync loops.
    async fn maybe_auto_resync(&mut self, current_slot: u32) {
        // Threshold: trigger resync after 10 consecutive fork-blocked slots
        // This gives the node enough time for normal recovery (reorg, sync)
        // before escalating to a full resync.
        let fork_resync_threshold: u32 = match self.config.network {
            Network::Devnet => 5,
            _ => 10,
        };

        if self.consecutive_fork_blocks < fork_resync_threshold {
            return;
        }

        // Don't resync at height 0 — there's nothing to resync from.
        // At genesis, fork signals (NoGossipActivity, etc.) are normal startup noise.
        let local_height = {
            let cs = self.chain_state.read().await;
            cs.best_height
        };
        if local_height == 0 {
            debug!("Fork recovery: skipping resync at height 0 (genesis)");
            self.consecutive_fork_blocks = 0;
            return;
        }

        // Respect cooldown (reuse existing exponential backoff logic)
        let (resync_in_progress, consecutive_resyncs) = {
            let sync = self.sync_manager.read().await;
            (
                sync.is_resync_in_progress(),
                sync.consecutive_resync_count(),
            )
        };

        if resync_in_progress {
            debug!("Fork recovery: resync already in progress, waiting");
            return;
        }

        // Exponential backoff: 60s * 2^(resyncs), max ~16 min
        let cooldown_secs = 60u64 * (1u64 << consecutive_resyncs.min(4));
        let can_resync = match self.last_resync_time {
            Some(last) => last.elapsed() > Duration::from_secs(cooldown_secs),
            None => true,
        };

        if !can_resync {
            debug!(
                "Fork recovery: cooldown active ({}s remaining)",
                cooldown_secs.saturating_sub(
                    self.last_resync_time
                        .map(|t| t.elapsed().as_secs())
                        .unwrap_or(0)
                )
            );
            return;
        }

        // Mainnet: direct 1-block rollback (bypasses resolve_shallow_fork's
        // empty_headers precondition which is always 0 in same-height forks)
        if self.config.network == Network::Mainnet {
            warn!(
                "FORK RECOVERY: {} consecutive fork-blocked slots on mainnet — rolling back 1 block",
                self.consecutive_fork_blocks
            );
            match self.rollback_one_block().await {
                Ok(true) => {
                    info!("Mainnet fork recovery: 1-block rollback succeeded");
                }
                Ok(false) => {
                    warn!(
                        "Mainnet fork recovery: rollback not possible, triggering genesis resync"
                    );
                    if let Err(e) = self.force_resync_from_genesis().await {
                        error!("Mainnet fork recovery resync failed: {}", e);
                    }
                }
                Err(e) => {
                    error!("Mainnet fork recovery rollback failed: {}", e);
                }
            }
            self.consecutive_fork_blocks = 0;
            self.last_resync_time = Some(Instant::now());
            return;
        }

        error!(
            "FORK RECOVERY: {} consecutive fork-blocked slots (threshold {}). Triggering auto-resync at slot {}.",
            self.consecutive_fork_blocks, fork_resync_threshold, current_slot
        );

        self.consecutive_fork_blocks = 0;
        self.last_resync_time = Some(Instant::now());
        if let Err(e) = self.force_resync_from_genesis().await {
            error!("Fork recovery resync failed: {}", e);
        }
    }

    /// Apply a snap sync snapshot: verify state root, replace local state, save to disk.
    async fn apply_snap_snapshot(&mut self, snapshot: network::VerifiedSnapshot) -> Result<()> {
        info!(
            "[SNAP_SYNC] Applying snapshot: height={}, hash={:.16}, root={:.16}",
            snapshot.block_height, snapshot.block_hash, snapshot.state_root
        );

        // Step 1: Verify state root (node-side, since network crate has no storage dep)
        let computed_root = storage::compute_state_root_from_bytes(
            &snapshot.chain_state,
            &snapshot.utxo_set,
            &snapshot.producer_set,
        );
        if computed_root != snapshot.state_root {
            error!(
                "[SNAP_SYNC] State root mismatch! computed={}, expected={} — blacklisting peer",
                computed_root, snapshot.state_root
            );
            // Can't easily get the peer here, but snap_fallback handles it
            self.sync_manager.write().await.snap_fallback_to_normal();
            return Ok(());
        }

        // Step 2: Deserialize via storage::StateSnapshot (avoids bincode dep in node)
        let snap = storage::StateSnapshot {
            block_hash: snapshot.block_hash,
            block_height: snapshot.block_height,
            chain_state_bytes: snapshot.chain_state,
            utxo_set_bytes: snapshot.utxo_set,
            producer_set_bytes: snapshot.producer_set,
            state_root: snapshot.state_root,
        };
        let (new_chain_state, new_utxo_set, new_producer_set) = snap
            .deserialize()
            .map_err(|e| anyhow::anyhow!("[SNAP_SYNC] Failed to deserialize snapshot: {}", e))?;

        info!(
            "[SNAP_SYNC] Deserialized: chain_state(height={}, hash={:.16})",
            new_chain_state.best_height, new_chain_state.best_hash,
        );

        // Step 3: Replace local state (preserve genesis_hash from our chain)
        let genesis_hash = self.chain_state.read().await.genesis_hash;
        {
            let mut cs = self.chain_state.write().await;
            *cs = new_chain_state;
            cs.genesis_hash = genesis_hash; // Preserve our genesis identity
            cs.mark_snap_synced(snapshot.block_height); // Survives restart: block store empty by design
        }
        {
            let mut utxo = self.utxo_set.write().await;
            *utxo = new_utxo_set;
        }
        {
            let mut ps = self.producer_set.write().await;
            *ps = new_producer_set;
        }

        // Step 4: Update sync manager local tip
        {
            let cs = self.chain_state.read().await;
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(cs.best_height, cs.best_hash, cs.best_slot);
        }

        // Step 5: Save to disk (includes snap_sync_height flag)
        self.save_state().await?;

        // Step 5b: Cache state root (all state consistent after snap sync replacement)
        {
            let cs = self.chain_state.read().await;
            let utxo = self.utxo_set.read().await;
            let ps = self.producer_set.read().await;
            if let Ok(root) = storage::compute_state_root(&cs, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, cs.best_hash, cs.best_height));
            }
        }

        // Step 6: Seed the canonical index so the first post-snap-sync block can
        // call set_canonical_chain without walking into an empty block store.
        self.block_store
            .seed_canonical_index(snapshot.block_hash, snapshot.block_height)?;

        // Step 7: Track snap sync height in-memory for validation mode selection
        self.snap_sync_height = Some(snapshot.block_height);

        // Step 8: Seed reorg handler so fork detection works immediately after snap sync
        {
            let mut sync = self.sync_manager.write().await;
            sync.record_block_applied_after_snap(snapshot.block_hash, snapshot.block_height);
        }

        info!(
            "[SNAP_SYNC] Snapshot applied successfully — now at height {} hash={:.16}",
            snapshot.block_height, snapshot.block_hash
        );

        Ok(())
    }

    /// - known_producers: cleared
    /// - fork_block_cache: cleared
    /// - equivocation_detector: cleared
    /// - producer_gset: cleared
    /// - Production timing state: reset
    ///
    /// Production is blocked by the sync manager's ProductionGate until:
    /// 1. Sync completes (caught up with peers)
    /// 2. Grace period expires (default 30s, with exponential backoff)
    async fn force_resync_from_genesis(&mut self) -> Result<()> {
        warn!("Force resync initiated - performing COMPLETE state reset to genesis");

        // Get genesis hash before clearing state
        let genesis_hash = self.chain_state.read().await.genesis_hash;

        // =========================================================================
        // LAYER 1: Reset sync manager FIRST (blocks production via ProductionGate)
        // =========================================================================
        {
            let mut sync = self.sync_manager.write().await;
            // This calls start_resync() internally, blocking production
            sync.reset_local_state(genesis_hash);
        }

        // =========================================================================
        // LAYER 2: Reset chain state to genesis
        // =========================================================================
        {
            let mut state = self.chain_state.write().await;
            state.best_height = 0;
            state.best_hash = genesis_hash;
            state.best_slot = 0;
            // Note: genesis_timestamp preserved for consistency
        }

        // =========================================================================
        // LAYER 3: Clear UTXO set completely
        // =========================================================================
        {
            let mut utxo = self.utxo_set.write().await;
            utxo.clear();
        }

        // =========================================================================
        // LAYER 4: Clear producer set (CRITICAL - this was the bug!)
        //
        // This forces the node into TRUE bootstrap mode where sync-before-produce
        // checks are properly enforced. Without this, the node would take the
        // "normal mode" path and skip sync checks.
        // =========================================================================
        {
            let mut producers = self.producer_set.write().await;
            producers.clear();
            info!("Producer set cleared - will rebuild from synced blocks");
        }

        // =========================================================================
        // LAYER 5: Clear known producers list
        // =========================================================================
        {
            let mut known = self.known_producers.write().await;
            known.clear();
        }

        // =========================================================================
        // LAYER 6: Clear fork block cache
        // =========================================================================
        {
            let mut cache = self.fork_block_cache.write().await;
            cache.clear();
        }

        // =========================================================================
        // LAYER 7: Clear equivocation detector (signatures from old chain invalid)
        // =========================================================================
        {
            let mut detector = self.equivocation_detector.write().await;
            detector.clear();
        }

        // =========================================================================
        // LAYER 8: Clear producer discovery CRDT
        // =========================================================================
        {
            let mut gset = self.producer_gset.write().await;
            gset.clear();
        }

        // =========================================================================
        // LAYER 9: Reset all production timing state
        // =========================================================================
        self.last_produced_slot = 0;
        self.first_peer_connected = None;
        self.last_producer_list_change = None;
        // Note: last_resync_time is set by the caller before calling this function
        // to implement the cooldown between resyncs

        // =========================================================================
        // LAYER 10: Clear RocksDB block store (purge stale fork blocks)
        //
        // Without this, old fork blocks remain in HEIGHT_INDEX and SLOT_INDEX,
        // polluting queries like get_last_rewarded_epoch() which iterates from
        // the end of HEIGHT_INDEX and would hit stale fork entries first.
        // =========================================================================
        if let Err(e) = self.block_store.clear() {
            error!("Failed to clear block store during resync: {}", e);
            // Non-fatal: sync will overwrite canonical entries, but stale
            // entries beyond best_height remain. Log and continue.
        }

        info!(
            "Complete state reset to genesis (height 0). \
             Production blocked until sync completes + grace period. \
             Will actively request blocks from peers."
        );

        Ok(())
    }

    /// Check producer eligibility for a received block.
    ///
    /// Builds a lightweight ValidationContext and calls validate_producer_eligibility.
    /// During bootstrap, validates using fallback rank windows from the GSet producer list.
    async fn check_producer_eligibility(&self, block: &Block) -> Result<()> {
        let state = self.chain_state.read().await;
        let height = state.best_height + 1;

        // Build weighted producer list
        let producers = self.producer_set.read().await;
        let active = producers.active_producers_at_height(height);
        let weighted: Vec<(PublicKey, u64)> = active
            .iter()
            .map(|p| (p.public_key, p.bond_count as u64))
            .collect();
        drop(producers);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Build bootstrap producer list from GSet (same source as production side).
        // Must be sorted by pubkey for deterministic fallback rank order.
        let mut bootstrap_producers = {
            let gset = self.producer_gset.read().await;
            gset.active_producers(7200) // 2h liveness window, same as production
        };
        if bootstrap_producers.is_empty() {
            let known = self.known_producers.read().await;
            bootstrap_producers = known.clone();
        }
        bootstrap_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Build liveness split for bootstrap validation (must match production side).
        let num_bp = bootstrap_producers.len();
        let liveness_window = std::cmp::max(
            consensus::LIVENESS_WINDOW_MIN,
            (num_bp as u64).saturating_mul(3),
        );
        let chain_height = height.saturating_sub(1);
        let cutoff = chain_height.saturating_sub(liveness_window);
        let (live_bp, stale_bp): (Vec<PublicKey>, Vec<PublicKey>) = {
            let (live, stale): (Vec<_>, Vec<_>) = bootstrap_producers.iter().partition(|pk| {
                match self.producer_liveness.get(pk) {
                    Some(&last_h) => last_h >= cutoff,
                    // No chain record: live if chain is young, stale otherwise
                    None => chain_height < liveness_window,
                }
            });
            (
                live.into_iter().copied().collect(),
                stale.into_iter().copied().collect(),
            )
        };
        // Deadlock safety: if all stale, treat all as live (filter disabled)
        let (live_bp, stale_bp) = if live_bp.is_empty() {
            (bootstrap_producers.clone(), Vec::new())
        } else {
            (live_bp, stale_bp)
        };

        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(self.config.network),
            self.config.network,
            now,
            height,
        )
        .with_producers_weighted(weighted)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp);

        // Apply chainspec if present
        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        validation::validate_producer_eligibility(&block.header, &ctx)?;
        Ok(())
    }

    /// Validate a block before applying it to the chain.
    /// DISABLED (2026-02-25): merkle root mismatch — re-enable after fix.
    #[allow(dead_code)]
    ///
    /// Builds a full ValidationContext and calls `validate_block_with_mode`.
    /// In `Light` mode (gap blocks after snap sync), VDF is skipped.
    /// In `Full` mode (recent blocks near tip), VDF is verified.
    async fn validate_block_for_apply(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<(), validation::ValidationError> {
        let state = self.chain_state.read().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Build weighted producer list
        let producers = self.producer_set.read().await;
        let active = producers.active_producers_at_height(height);
        let weighted: Vec<(PublicKey, u64)> = active
            .iter()
            .map(|p| (p.public_key, p.bond_count as u64))
            .collect();
        drop(producers);

        // Build bootstrap producer list (same as check_producer_eligibility)
        let mut bootstrap_producers = {
            let gset = self.producer_gset.read().await;
            gset.active_producers(7200)
        };
        if bootstrap_producers.is_empty() {
            let known = self.known_producers.read().await;
            bootstrap_producers = known.clone();
        }
        bootstrap_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Build liveness split
        let num_bp = bootstrap_producers.len();
        let liveness_window = std::cmp::max(
            consensus::LIVENESS_WINDOW_MIN,
            (num_bp as u64).saturating_mul(3),
        );
        let chain_height = height.saturating_sub(1);
        let cutoff = chain_height.saturating_sub(liveness_window);
        let (live_bp, stale_bp): (Vec<PublicKey>, Vec<PublicKey>) = {
            let (live, stale): (Vec<_>, Vec<_>) =
                bootstrap_producers
                    .iter()
                    .partition(|pk| match self.producer_liveness.get(pk) {
                        Some(&last_h) => last_h >= cutoff,
                        None => chain_height < liveness_window,
                    });
            (
                live.into_iter().copied().collect(),
                stale.into_iter().copied().collect(),
            )
        };
        let (live_bp, stale_bp) = if live_bp.is_empty() {
            (bootstrap_producers.clone(), Vec::new())
        } else {
            (live_bp, stale_bp)
        };

        // Get previous block timestamp from block store for header validation
        let prev_timestamp = self
            .block_store
            .get_header(&state.best_hash)
            .ok()
            .flatten()
            .map(|h| h.timestamp)
            .unwrap_or(0);

        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(self.config.network),
            self.config.network,
            now,
            height,
        )
        .with_prev_block(state.best_slot, prev_timestamp, state.best_hash)
        .with_producers_weighted(weighted)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp);

        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        drop(state);

        validation::validate_block_with_mode(block, &ctx, mode)
    }

    /// Apply a block to the chain
    async fn apply_block(&mut self, block: Block) -> Result<()> {
        let block_hash = block.hash();

        // Defense-in-depth: skip blocks we already have.
        // Prevents height double-counting if sync delivers stored blocks (ISSUE-5).
        if self.block_store.has_block(&block_hash)? {
            warn!(
                "Block {} already in store, skipping apply to prevent height corruption",
                block_hash
            );
            return Ok(());
        }

        let height = self.chain_state.read().await.best_height + 1;

        // DISABLED (2026-02-25): validate_block_for_apply causes false "invalid merkle root"
        // rejections. The merkle root computation in verify_merkle_root() differs from how
        // the producer calculates it in produce_block(). Investigation needed before re-enabling.
        // See: validate_block_for_apply() and ValidationMode are ready for use once the
        // merkle root computation is made consistent between producer and validator.

        info!("Applying block {} at height {}", block_hash, height);

        // Store the block and update canonical chain index
        self.block_store.put_block(&block, height)?;
        self.block_store.set_canonical_chain(block_hash, height)?;

        // Update producer liveness tracker (for scheduling filter)
        self.producer_liveness.insert(block.header.producer, height);

        // Track newly registered producers to add to known_producers after lock release
        let mut new_registrations: Vec<PublicKey> = Vec::new();

        // Apply transactions to UTXO set and process special transactions
        {
            let mut utxo = self.utxo_set.write().await;
            let mut producers = self.producer_set.write().await;

            for (tx_index, tx) in block.transactions.iter().enumerate() {
                // Apply UTXO changes
                // Check for both regular coinbase and epoch reward coinbase (both mint new coins)
                let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                let _ = utxo.spend_transaction(tx); // Ignore errors for reward-minting txs
                utxo.add_transaction(tx, height, is_reward_tx);

                // Process registration transactions
                if tx.tx_type == TxType::Registration {
                    // During genesis, Registration TXs are VDF proof containers only.
                    // Actual producer registration happens at GENESIS PHASE COMPLETE (block 361).
                    if self.config.network.is_in_genesis(height) {
                        if let Some(reg_data) = tx.registration_data() {
                            info!(
                                "Genesis VDF proof submitted by {} (block {})",
                                hex::encode(&reg_data.public_key.as_bytes()[..8]),
                                height
                            );
                        }
                        // Skip immediate registration — handled at genesis end
                    } else if let Some(reg_data) = tx.registration_data() {
                        // Find the bond output
                        if let Some((bond_index, bond_output)) =
                            tx.outputs.iter().enumerate().find(|(_, o)| {
                                o.output_type == doli_core::transaction::OutputType::Bond
                            })
                        {
                            let tx_hash = tx.hash();
                            let era = self.params.height_to_era(height);

                            let producer_info = storage::ProducerInfo::new_with_bonds(
                                reg_data.public_key,
                                height,
                                bond_output.amount,
                                (tx_hash, bond_index as u32),
                                era,
                                reg_data.bond_count,
                            );

                            if let Err(e) = producers.register(producer_info, height) {
                                warn!("Failed to register producer: {}", e);
                            } else {
                                info!(
                                    "Registered producer {} at height {}",
                                    crypto_hash(reg_data.public_key.as_bytes()),
                                    height
                                );
                                // Track for known_producers update after lock release
                                new_registrations.push(reg_data.public_key);
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

                // Process AddBond transactions - add bonds to existing producer
                // AddBond txs have inputs (consumed) and NO outputs - funds go into bond state
                if tx.tx_type == TxType::AddBond {
                    if let Some(add_bond_data) = tx.add_bond_data() {
                        let tx_hash = tx.hash();
                        let bond_count = add_bond_data.bond_count;

                        // Create synthetic outpoints for tracking (tx_hash, index)
                        // These are used for FIFO withdrawal ordering
                        let bond_outpoints: Vec<(crypto::Hash, u32)> =
                            (0..bond_count).map(|i| (tx_hash, i)).collect();

                        let bond_unit = self.config.network.bond_unit();
                        if let Some(producer_info) =
                            producers.get_by_pubkey_mut(&add_bond_data.producer_pubkey)
                        {
                            let added = producer_info.add_bonds(bond_outpoints, bond_unit);
                            if added > 0 {
                                info!(
                                    "Added {} bonds to producer {} (total: {} bonds, {} DOLI)",
                                    added,
                                    crypto_hash(add_bond_data.producer_pubkey.as_bytes()),
                                    producer_info.bond_count,
                                    producer_info.bond_amount / UNITS_PER_COIN
                                );
                            }
                        } else {
                            warn!(
                                "AddBond for unknown producer: {}",
                                crypto_hash(add_bond_data.producer_pubkey.as_bytes())
                            );
                        }
                    }
                }

                // Process DelegateBond transactions
                if tx.tx_type == TxType::DelegateBond {
                    if let Some(data) = tx.delegate_bond_data() {
                        match producers.delegate_bonds(
                            &data.delegator,
                            &data.delegate,
                            data.bond_count,
                        ) {
                            Ok(()) => {
                                info!(
                                    "Delegated {} bonds from {} to {} at height {}",
                                    data.bond_count,
                                    crypto_hash(data.delegator.as_bytes()),
                                    crypto_hash(data.delegate.as_bytes()),
                                    height
                                );
                            }
                            Err(e) => {
                                warn!("Failed to delegate bonds: {}", e);
                            }
                        }
                    }
                }

                // Process RevokeDelegation transactions
                if tx.tx_type == TxType::RevokeDelegation {
                    if let Some(data) = tx.revoke_delegation_data() {
                        match producers.revoke_delegation(&data.delegator) {
                            Ok(()) => {
                                info!(
                                    "Revoked delegation for {} at height {}",
                                    crypto_hash(data.delegator.as_bytes()),
                                    height
                                );
                            }
                            Err(e) => {
                                warn!("Failed to revoke delegation: {}", e);
                            }
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

        // Add newly registered producers to known_producers for bootstrap round-robin
        // This ensures they can produce blocks immediately (without waiting for gossip discovery)
        let genesis_active = self.config.network.is_in_genesis(height);
        if !new_registrations.is_empty()
            && (self.config.network == Network::Testnet
                || self.config.network == Network::Devnet
                || genesis_active)
        {
            let mut known = self.known_producers.write().await;
            let mut added_any = false;
            for pubkey in new_registrations {
                if !known.contains(&pubkey) {
                    known.push(pubkey);
                    added_any = true;
                    let pubkey_hash = crypto_hash(pubkey.as_bytes());
                    info!(
                        "Added registered producer to known_producers: {} (now {} known)",
                        &pubkey_hash.to_hex()[..16],
                        known.len()
                    );
                }
            }
            // Sort for deterministic ordering (all nodes compute same order)
            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

            // Trigger stability check so round-robin waits for convergence
            if added_any {
                self.last_producer_list_change = Some(std::time::Instant::now());
            }
        }

        // Update chain state
        {
            let mut state = self.chain_state.write().await;
            state.update(block_hash, height, block.header.slot);
            // Clear snap sync marker: block store now has at least one real block,
            // so the next restart's integrity check will find it normally.
            state.clear_snap_sync();

            // For devnet: set genesis_timestamp from first block if not already set from chainspec
            // When chainspec has a fixed genesis time, we MUST use it (all nodes must agree)
            // Only fall back to deriving from block timestamp if no chainspec genesis was provided
            if state.genesis_timestamp == 0 && height <= 1 {
                // Check if we have a chainspec genesis time override
                if let Some(override_time) = self.config.genesis_time_override {
                    // Use the chainspec genesis time (already set in params)
                    state.genesis_timestamp = override_time;
                    info!(
                        "Genesis timestamp set from chainspec: {} (block {} received)",
                        state.genesis_timestamp, height
                    );
                } else {
                    // No chainspec override - derive from block timestamp (legacy behavior)
                    let block_timestamp = block.header.timestamp;
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
        }

        // Track this block for finality (attestation-based finality gadget).
        // The total_weight is the sum of all active producer bonds at this height.
        {
            let producers = self.producer_set.read().await;
            let total_weight: u64 = producers
                .active_producers_at_height(height)
                .iter()
                .map(|p| p.selection_weight())
                .sum();
            drop(producers);

            if total_weight > 0 {
                let mut sync = self.sync_manager.write().await;
                sync.track_block_for_finality(block_hash, height, block.header.slot, total_weight);
            }
        }

        // Recompute our tier at epoch boundaries and build EpochSnapshot
        self.recompute_tier(height).await;
        if doli_core::EpochSnapshot::is_epoch_boundary(height) {
            let epoch = doli_core::EpochSnapshot::epoch_from_height(height);
            let producers = self.producer_set.read().await;
            let active = producers.active_producers_at_height(height);
            let pws: Vec<(PublicKey, u64)> = active
                .iter()
                .map(|p| (p.public_key, p.selection_weight()))
                .collect();
            let total_w: u64 = pws.iter().map(|(_, w)| *w).sum();
            let tier1 = compute_tier1_set(&pws);
            let mut all_pks: Vec<PublicKey> = pws.into_iter().map(|(pk, _)| pk).collect();
            all_pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            drop(producers);

            let snapshot = doli_core::EpochSnapshot::new(epoch, tier1, &all_pks, total_w);
            info!(
                "EpochSnapshot built: epoch={}, producers={}, weight={}, merkle={}",
                epoch, snapshot.total_producers, snapshot.total_weight, snapshot.merkle_root
            );
        }

        // Attest to blocks we didn't produce (self-attestation for our own blocks
        // is handled in try_produce_block after broadcast)
        if let Some(ref kp) = self.producer_key {
            if block.header.producer != *kp.public_key() {
                self.create_and_broadcast_attestation(block_hash, block.header.slot, height)
                    .await;
            }
        }

        // GENESIS END: Derive and register producers with REAL bonds from the blockchain.
        // This runs when applying the FIRST post-genesis block (genesis_blocks + 1).
        // Each genesis producer's coinbase reward UTXOs are consumed to back their bond,
        // creating a proper bond-backed registration (no phantom bonds).
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks > 0 && height == genesis_blocks + 1 {
            info!("=== GENESIS PHASE COMPLETE at height {} ===", height);

            let genesis_producers = self.derive_genesis_producers_from_chain();
            let producer_count = genesis_producers.len();

            if producer_count > 0 {
                info!(
                    "Derived {} genesis producers from blockchain (blocks 1-{})...",
                    producer_count, genesis_blocks
                );

                let bond_unit = self.config.network.bond_unit();
                let era = self.params.height_to_era(height);
                let mut utxo = self.utxo_set.write().await;
                let mut producers = self.producer_set.write().await;

                // Clear bootstrap phantom producers — replace with real bond-backed ones
                producers.clear();

                for pubkey in &genesis_producers {
                    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_bytes());

                    // Find all UTXOs belonging to this producer
                    let mut producer_utxos = utxo.get_by_pubkey_hash(&pubkey_hash);
                    // Sort by (height, tx_hash, index) for deterministic selection
                    producer_utxos.sort_by(|(a_op, a_entry), (b_op, b_entry)| {
                        a_entry
                            .height
                            .cmp(&b_entry.height)
                            .then_with(|| a_op.tx_hash.cmp(&b_op.tx_hash))
                            .then_with(|| a_op.index.cmp(&b_op.index))
                    });

                    let total_available: u64 =
                        producer_utxos.iter().map(|(_, e)| e.output.amount).sum();

                    if total_available < bond_unit {
                        warn!(
                                "Genesis producer {} has insufficient balance ({} < {} bond_unit), skipping bond migration",
                                hex::encode(&pubkey.as_bytes()[..8]),
                                total_available,
                                bond_unit
                            );
                        continue;
                    }

                    // Select UTXOs to consume for the bond (oldest first)
                    let mut consumed_amount: u64 = 0;
                    let mut consumed_outpoints = Vec::new();
                    for (outpoint, entry) in &producer_utxos {
                        if consumed_amount >= bond_unit {
                            break;
                        }
                        consumed_amount += entry.output.amount;
                        consumed_outpoints.push(*outpoint);
                    }

                    // Remove consumed UTXOs from the set
                    for outpoint in &consumed_outpoints {
                        utxo.remove(outpoint);
                    }

                    // Create deterministic bond hash for tracking
                    let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());

                    // Return change if we consumed more than bond_unit
                    let change = consumed_amount.saturating_sub(bond_unit);
                    if change > 0 {
                        let change_outpoint = storage::Outpoint::new(bond_hash, 1);
                        let change_entry = storage::UtxoEntry {
                            output: doli_core::transaction::Output::normal(change, pubkey_hash),
                            height,
                            is_coinbase: false,
                            is_epoch_reward: false,
                        };
                        utxo.insert(change_outpoint, change_entry);
                    }

                    // Register producer with real bond outpoint
                    // registered_at = 0: Genesis producers are exempt from ACTIVATION_DELAY
                    // (they've been producing since block 1, no propagation delay needed)
                    let producer_info = storage::ProducerInfo::new_with_bonds(
                        *pubkey,
                        0,
                        bond_unit,
                        (bond_hash, 0),
                        era,
                        1, // 1 bond
                    );

                    match producers.register(producer_info, height) {
                        Ok(()) => {
                            info!(
                                    "  Registered genesis producer {} with real bond ({} UTXOs consumed, {} DOLI bonded, {} DOLI change)",
                                    hex::encode(&pubkey.as_bytes()[..8]),
                                    consumed_outpoints.len(),
                                    bond_unit / UNITS_PER_COIN,
                                    change / UNITS_PER_COIN,
                                );
                        }
                        Err(e) => {
                            warn!(
                                "  Failed to register genesis producer {}: {}",
                                hex::encode(&pubkey.as_bytes()[..8]),
                                e
                            );
                        }
                    }
                }

                // Save producer set immediately
                let producers_path = self.config.data_dir.join("producers.bin");
                if let Err(e) = producers.save(&producers_path) {
                    warn!("Failed to save producer set after genesis: {}", e);
                }

                info!(
                    "Genesis complete: {} producers now active in scheduler (real bonds)",
                    producers.active_producers().len()
                );
            } else {
                warn!("Genesis ended with no producers found in blockchain!");
            }
        }

        // Remove transactions from mempool and prune any with now-spent inputs
        {
            let mut mempool = self.mempool.write().await;
            mempool.remove_for_block(&block.transactions);
            let utxo = self.utxo_set.read().await;
            let height = self.chain_state.read().await.best_height;
            mempool.revalidate(&utxo, height);
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

        // Cache state root while all state is consistent (same height).
        // GetStateRoot reads this cache instead of acquiring 3 separate locks,
        // which avoids the race condition where UTXO/ProducerSet are at height N+1
        // but ChainState is still at height N.
        {
            let cs = self.chain_state.read().await;
            let utxo = self.utxo_set.read().await;
            let ps = self.producer_set.read().await;
            if let Ok(root) = storage::compute_state_root(&cs, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, cs.best_hash, cs.best_height));
            }
        }

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

                // Determine starting height via O(1) hash→height index.
                // The hash_to_height index is populated by:
                // 1. rebuild_canonical_index (one-time migration on startup)
                // 2. Normal block insertion during sync/production
                // No linear fallback — avoids O(n) scans that caused timeouts.
                let start_height = if start_hash == genesis_hash {
                    0
                } else {
                    match self
                        .block_store
                        .get_height_by_hash(&start_hash)
                        .ok()
                        .flatten()
                    {
                        Some(h) => h,
                        None => {
                            // Unknown hash — respond empty so requester doesn't timeout
                            debug!(
                                "GetHeaders: unknown start_hash {} (responding with empty)",
                                start_hash
                            );
                            if let Some(ref network) = self.network {
                                let _ = network
                                    .send_sync_response(channel, SyncResponse::Headers(vec![]))
                                    .await;
                            }
                            return Ok(());
                        }
                    }
                };

                // Return headers from start_height+1 up to max_count
                // Use get_hash_by_height → get_header to avoid deserializing full blocks
                let end_height = (start_height + max_count as u64).min(best_height);
                for height in (start_height + 1)..=end_height {
                    if let Ok(Some(hash)) = self.block_store.get_hash_by_height(height) {
                        if let Ok(Some(header)) = self.block_store.get_header(&hash) {
                            headers.push(header);
                        } else {
                            break;
                        }
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

            SyncRequest::GetStateRoot { block_hash: _ } => {
                // Use cached state root to avoid race conditions.
                // The cache is updated atomically after each apply_block, so all
                // three components (ChainState, UTXO, ProducerSet) are guaranteed
                // to be at the same height.
                let cache = self.cached_state_root.read().await;
                if let Some((root, hash, height)) = *cache {
                    SyncResponse::StateRoot {
                        block_hash: hash,
                        block_height: height,
                        state_root: root,
                    }
                } else {
                    // Fallback: compute on-the-fly if cache not yet populated (pre-first-block)
                    drop(cache);
                    let chain_state = self.chain_state.read().await;
                    let current_hash = chain_state.best_hash;
                    let current_height = chain_state.best_height;
                    let utxo_set = self.utxo_set.read().await;
                    let ps = self.producer_set.read().await;
                    match storage::compute_state_root(&chain_state, &utxo_set, &ps) {
                        Ok(root) => SyncResponse::StateRoot {
                            block_hash: current_hash,
                            block_height: current_height,
                            state_root: root,
                        },
                        Err(e) => SyncResponse::Error(format!("State root error: {}", e)),
                    }
                }
            }

            SyncRequest::GetStateSnapshot { block_hash } => {
                let chain_state = self.chain_state.read().await;
                if chain_state.best_hash != block_hash {
                    SyncResponse::Error(format!(
                        "Snapshot only available for current tip {}",
                        chain_state.best_hash
                    ))
                } else {
                    let utxo_set = self.utxo_set.read().await;
                    let ps = self.producer_set.read().await;
                    match storage::StateSnapshot::create(&chain_state, &utxo_set, &ps) {
                        Ok(snap) => {
                            info!(
                                "[SNAP_SYNC] Serving snapshot at height={}, size={}KB, root={}",
                                snap.block_height,
                                snap.total_bytes() / 1024,
                                snap.state_root
                            );
                            SyncResponse::StateSnapshot {
                                block_hash: snap.block_hash,
                                block_height: snap.block_height,
                                chain_state: snap.chain_state_bytes,
                                utxo_set: snap.utxo_set_bytes,
                                producer_set: snap.producer_set_bytes,
                                state_root: snap.state_root,
                            }
                        }
                        Err(e) => SyncResponse::Error(format!("Snapshot error: {}", e)),
                    }
                }
            }
        };

        if let Some(ref network) = self.network {
            let _ = network.send_sync_response(channel, response).await;
        }

        Ok(())
    }

    // NOTE: try_broadcast_heartbeat removed in deterministic scheduler model
    // Rewards go 100% to block producer via coinbase, no heartbeats needed

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

        // =========================================================================
        // PRODUCTION GATE CHECK - Single source of truth for production safety
        //
        // This is the FIRST and MOST CRITICAL check. The SyncManager's ProductionGate
        // implements defense-in-depth with multiple layers:
        // 1. Explicit block check (invariant violations)
        // 2. Resync-in-progress check
        // 3. Active sync check (downloading headers/bodies)
        // 4. Bootstrap gate (must have fresh peer status before producing)
        // 5. Post-resync grace period with exponential backoff
        // 6. Peer synchronization check (within N slots/heights)
        //
        // ALL checks must pass. This prevents the infinite resync loop bug where
        // nodes at height 0 would produce orphan blocks for far-ahead slots.
        // =========================================================================
        let auth_result = {
            let sync_state = self.sync_manager.read().await;
            let result = sync_state.can_produce(current_slot);
            info!(
                "[NODE_PRODUCE] slot={} can_produce result: {:?}",
                current_slot, result
            );
            result
        }; // sync_state guard dropped here — safe to call &mut self methods below

        match auth_result {
            ProductionAuthorization::Authorized => {
                self.consecutive_fork_blocks = 0;
                self.shallow_rollback_count = 0;
                info!(
                    "[NODE_PRODUCE] slot={} AUTHORIZED - proceeding",
                    current_slot
                );
            }
            ProductionAuthorization::BlockedSyncing => {
                info!("[NODE_PRODUCE] slot={} BLOCKED: Syncing", current_slot);
                return Ok(());
            }
            ProductionAuthorization::BlockedResync { .. } => {
                info!("[NODE_PRODUCE] slot={} BLOCKED: Resync", current_slot);
                return Ok(());
            }
            ProductionAuthorization::BlockedBehindPeers {
                local_height,
                peer_height,
                height_diff,
            } => {
                self.consecutive_fork_blocks += 1;
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: BehindPeers local_h={} peer_h={} diff={} (consecutive={})",
                    current_slot, local_height, peer_height, height_diff, self.consecutive_fork_blocks
                );
                self.maybe_auto_resync(current_slot).await;
                return Ok(());
            }
            ProductionAuthorization::BlockedAheadOfPeers {
                local_height,
                peer_height,
                height_ahead,
            } => {
                self.consecutive_fork_blocks += 1;
                warn!(
                    "[NODE_PRODUCE] FORK DETECTED via AheadOfPeers: slot={} local_h={} peer_h={} ahead={} (consecutive={})",
                    current_slot, local_height, peer_height, height_ahead, self.consecutive_fork_blocks
                );
                // Try fork recovery first, then auto-resync if threshold exceeded
                self.try_trigger_fork_recovery().await;
                self.maybe_auto_resync(current_slot).await;
                return Ok(());
            }
            ProductionAuthorization::BlockedSyncFailures { failure_count } => {
                // Low sync failure counts (< 50) are normal network churn.
                // But 50+ consecutive failures means our chain has genuinely diverged
                // from ALL peers — every GetHeaders returns empty because no peer
                // recognizes our tip hash. This IS a fork.
                if failure_count >= 50 {
                    self.consecutive_fork_blocks += 1;
                    error!(
                        "[NODE_PRODUCE] FORK DETECTED via persistent SyncFailures: slot={} failures={} (consecutive={})",
                        current_slot, failure_count, self.consecutive_fork_blocks
                    );
                    self.maybe_auto_resync(current_slot).await;
                } else {
                    warn!(
                        "[NODE_PRODUCE] slot={} BLOCKED: SyncFailures (failures={})",
                        current_slot, failure_count
                    );
                }
                return Ok(());
            }
            ProductionAuthorization::BlockedInsufficientPeers {
                peer_count,
                min_required,
            } => {
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: InsufficientPeers - only {} peers (need {})",
                    current_slot, peer_count, min_required
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedChainMismatch {
                peer_id,
                local_hash,
                peer_hash,
                local_height,
            } => {
                self.consecutive_fork_blocks += 1;
                error!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Chain mismatch with peer {} at height {} (local={}, peer={}) (consecutive={})",
                    current_slot, peer_id, local_height, local_hash, peer_hash, self.consecutive_fork_blocks
                );
                // Try fork recovery first, then auto-resync if threshold exceeded
                self.try_trigger_fork_recovery().await;
                self.maybe_auto_resync(current_slot).await;
                return Ok(());
            }
            ProductionAuthorization::BlockedNoGossipActivity {
                seconds_since_gossip,
                peer_count,
            } => {
                // No gossip activity is NOT a definitive fork signal — it can happen
                // during network startup or temporary connectivity issues.
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: No gossip activity for {}s with {} peers",
                    current_slot, seconds_since_gossip, peer_count
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedExplicit { reason } => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Explicit - {}",
                    current_slot, reason
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedBootstrap { reason } => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Bootstrap - {}",
                    current_slot, reason
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedConflictsFinality {
                local_finalized_height,
            } => {
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Chain conflicts with finalized block at height {}",
                    current_slot, local_finalized_height
                );
                return Ok(());
            }
        }

        // Log slot info periodically (every ~60 seconds)
        if current_slot.is_multiple_of(60) && current_slot != self.last_produced_slot {
            info!(
                "Production check: now={}, genesis={}, current_slot={}, last_produced={}",
                now, self.params.genesis_time, current_slot, self.last_produced_slot
            );
        }

        // Don't produce for the same slot twice
        if current_slot <= self.last_produced_slot {
            return Ok(());
        }

        // EARLY BLOCK EXISTENCE CHECK (optimization)
        // Check if a block already exists for this slot before spending time on eligibility
        // checks and VDF computation. This is safe because:
        // 1. If we see a block, another producer already succeeded
        // 2. We'll check again after VDF to catch blocks that appeared during computation
        if self.block_store.has_block_for_slot(current_slot as u64) {
            debug!(
                "Block already exists for slot {} - skipping production",
                current_slot
            );
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

        // =========================================================================
        // DEFENSE-IN-DEPTH: Peer-Aware Behind-Network Check
        //
        // Prevents producing orphan blocks when we're significantly behind the
        // network. A node at height 0 should never produce for slot 92 if peers
        // are at height 90 — the block would be an orphan.
        //
        // Key insight: Compare HEIGHTS, not slot-height gap. In sparse chains
        // (genesis phase, network downtime), slots advance via wall-clock while
        // blocks are produced infrequently. A large slot-height gap (e.g.,
        // slot 100,000 at height 175) is normal and does NOT indicate we're
        // behind — it means the chain has been producing sparsely. What matters
        // is whether peers have blocks we're missing.
        //
        // Previous bug: Using current_slot - height as the gap metric caused a
        // permanent deadlock where joining nodes could never produce because
        // the slot-height gap always exceeded max_gap in sparse chains.
        //
        // Thresholds:
        // - Height < 10: max 3 blocks behind (tight - prevent orphan forks)
        // - Height 10+:  max 5 blocks behind (allow propagation delay)
        // =========================================================================
        let network_tip_height = {
            let sync = self.sync_manager.read().await;
            sync.best_peer_height()
        };

        let network_height_ahead = network_tip_height > height.saturating_sub(1);

        if height > 1 && network_height_ahead {
            let blocks_behind = network_tip_height.saturating_sub(height.saturating_sub(1));
            let max_behind: u64 = if height < 10 {
                3 // Tight during early chain - prevent orphan forks
            } else {
                5 // Normal operation - allow some propagation delay
            };

            if blocks_behind > max_behind {
                debug!(
                    "Behind network by {} blocks (next_height={}, network_tip={}) - deferring production",
                    blocks_behind, height, network_tip_height
                );
                return Ok(());
            }
        }

        // Get active producers with their bond counts for weighted round-robin selection.
        // Per WHITEPAPER Section 7: each bond unit = one ticket in the rotation.
        // Use active_producers_at_height to ensure all nodes have the same view -
        // new producers must wait ACTIVATION_DELAY blocks before entering the scheduler.
        let producers = self.producer_set.read().await;
        let active_with_weights: Vec<(PublicKey, u64)> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| (p.public_key, p.bond_count as u64))
            .collect();
        let total_producers = producers.total_count();
        drop(producers);

        // Check if we're in genesis phase (bond-free production)
        let in_genesis = self.config.network.is_in_genesis(height);

        let our_pk = producer_key.public_key();
        let _we_are_active = active_with_weights.iter().any(|(pk, _)| pk == our_pk);

        // =========================================================================
        // INVARIANT CHECK: Detect inconsistent state after resync
        //
        // If we're at a very low height (< 10) but have many active producers with
        // weights, AND we're not in genesis phase, this indicates an inconsistent
        // state - possibly a failed or incomplete resync.
        //
        // After a proper resync via force_resync_from_genesis(), the producer_set
        // should be cleared. If it's not, we're in a dangerous state where
        // production could create orphan blocks.
        //
        // This check catches edge cases like:
        // - Interrupted resync
        // - State corruption
        // - Race conditions in state updates
        // =========================================================================
        if height < 10 && !in_genesis && active_with_weights.len() > 5 {
            error!(
                "INVARIANT VIOLATION: height {} < 10 but {} active producers (total: {}) \
                 outside genesis phase. This indicates inconsistent state - blocking production.",
                height,
                active_with_weights.len(),
                total_producers
            );
            self.sync_manager.write().await.block_production(&format!(
                "invariant violation: height {} with {} active producers outside genesis",
                height,
                active_with_weights.len()
            ));
            return Ok(());
        }

        // Use bootstrap mode if:
        // 1. Still in genesis phase (no bond required), OR
        // 2. No active producers registered (transition block or testnet/devnet)
        let our_pubkey = *producer_key.public_key();
        let genesis_blocks = self.config.network.genesis_blocks();
        let (eligible, our_bootstrap_rank) = if in_genesis || active_with_weights.is_empty() {
            match self.config.network {
                Network::Mainnet if !in_genesis && height > genesis_blocks + 1 => {
                    // After the transition block, mainnet requires registered producers.
                    // Height genesis_blocks+1 is the transition block — produced via bootstrap,
                    // and its apply_block triggers genesis producer registration.
                    return Ok(());
                }
                Network::Mainnet | Network::Testnet | Network::Devnet => {
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

                    // BOOTSTRAP NODE CONNECTION CHECK: Wait for peer status before producing
                    // This is bootstrap-specific: seed nodes have no bootstrap config,
                    // joining nodes have bootstrap config and must wait for connection.
                    // (Sync-before-produce checks are handled globally above)
                    //
                    // Skip during genesis: all nodes are bootstrapping together from scratch,
                    // there's no existing chain to sync from and no seed to wait for.
                    let has_bootstrap_nodes = !self.config.bootstrap_nodes.is_empty();
                    if has_bootstrap_nodes && !in_genesis {
                        let sync_state = self.sync_manager.read().await;
                        let peer_count = sync_state.peer_count();
                        let best_peer_height = sync_state.best_peer_height();
                        drop(sync_state);

                        if peer_count == 0 {
                            // We have bootstrap nodes configured but haven't received their status yet
                            // Wait for connection before producing to avoid chain splits
                            debug!(
                                "Bootstrap sync: waiting for peer status (bootstrap nodes configured but no peers yet)"
                            );
                            return Ok(());
                        }

                        // JOINING NODE BOOTSTRAP GUARD: Ensure chain tip is fresh before producing.
                        //
                        // Problem 1: Joining nodes at height 0 may produce before receiving blocks
                        // from the network, creating an isolated fork with different genesis.
                        //
                        // Problem 2: Even at height > 0, joining nodes may produce before receiving
                        // the latest blocks due to network propagation delay. For example:
                        // - Node A produces block at slot 3, height 2
                        // - Node B (at height 1) doesn't receive it before slot 4
                        // - Node B produces at slot 4, height 2 (different parent!) → fork
                        //
                        // Solution: During early bootstrap (height < 10), joining nodes must ensure
                        // their chain tip is recent (within 1 slot of current) before producing.
                        // This gives time for in-flight blocks to arrive.
                        //
                        // The bootstrap_sync_grace_secs timeout allows production to proceed if:
                        // - The network is genuinely starting fresh (no blocks to receive)
                        // - Or this node happens to be the designated producer for early slots

                        let bootstrap_sync_grace_secs: u64 =
                            if self.config.network == Network::Devnet {
                                15 // ~15 slots at 1s/slot for faster devnet testing
                            } else {
                                90 // ~9 slots at 10s/slot for testnet/mainnet
                            };

                        let within_bootstrap_grace =
                            if let Some(first_peer_time) = self.first_peer_connected {
                                first_peer_time.elapsed().as_secs() < bootstrap_sync_grace_secs
                            } else {
                                true // No peers yet - handled above
                            };

                        // BOOTSTRAP MIN HEIGHT: Joining nodes must wait for seed to establish
                        // a canonical prefix before they can produce. This prevents race conditions
                        // where multiple nodes produce competing blocks at height 1-2.
                        //
                        // The seed node (no bootstrap config) is exempt and can produce at any height.
                        // After the seed establishes the first few blocks, joining nodes sync via
                        // gossip and can then participate in production.
                        //
                        // NOTE: height = best_height + 1, so BOOTSTRAP_MIN_HEIGHT = 3 means
                        // joining nodes must have best_height >= 2 (received 2 blocks from seed).
                        const BOOTSTRAP_MIN_HEIGHT: u64 = 3;

                        // BOOTSTRAP TIMEOUT: After waiting too long, allow production anyway.
                        // This handles the case where the seed node failed or is very slow.
                        // Devnet uses shorter timeout for faster iteration.
                        let bootstrap_timeout_secs: u64 = if self.config.network == Network::Devnet
                        {
                            60 // 1 minute for devnet
                        } else {
                            180 // 3 minutes for testnet/mainnet
                        };

                        let past_bootstrap_timeout = self
                            .first_peer_connected
                            .map(|t| t.elapsed().as_secs() >= bootstrap_timeout_secs)
                            .unwrap_or(false);

                        if height < BOOTSTRAP_MIN_HEIGHT
                            && best_peer_height > 0
                            && !past_bootstrap_timeout
                        {
                            debug!(
                                "Joining node at height {}: waiting for seed to establish chain (min_height={}, peer_count={}, best_peer_height={})",
                                height, BOOTSTRAP_MIN_HEIGHT, peer_count, best_peer_height
                            );
                            return Ok(());
                        }

                        if height < BOOTSTRAP_MIN_HEIGHT && past_bootstrap_timeout {
                            warn!(
                                "Bootstrap timeout reached ({}s) - joining node proceeding to produce at height {} (seed may have failed)",
                                bootstrap_timeout_secs, height
                            );
                        }

                        // Check 2: During bootstrap grace, ensure chain tip is fresh
                        // If current_slot - chain_tip_slot > 1, we might be missing in-flight blocks
                        let chain_tip_slot = {
                            let state = self.chain_state.read().await;
                            state.best_slot
                        };
                        let slot_gap = current_slot.saturating_sub(chain_tip_slot);

                        // During bootstrap grace period, require chain tip to be recent (gap <= 1)
                        // Skip this check if we've passed the bootstrap timeout
                        if height > 0
                            && within_bootstrap_grace
                            && slot_gap > 1
                            && !past_bootstrap_timeout
                        {
                            debug!(
                                "Joining node bootstrap: chain tip not fresh (height={}, chain_tip_slot={}, current_slot={}, gap={}), waiting for in-flight blocks",
                                height, chain_tip_slot, current_slot, slot_gap
                            );
                            return Ok(());
                        }

                        // Check 3: ALWAYS compare against best peer height before producing
                        // This prevents nodes from producing orphan blocks even after grace period expires
                        // If we're more than 2 blocks behind the best peer, defer production
                        if best_peer_height > 0 && height + 2 < best_peer_height {
                            debug!(
                                "Behind peers: our height {} vs best peer height {} (diff={}) - deferring production",
                                height, best_peer_height, best_peer_height - height
                            );
                            return Ok(());
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

                    // Get peer count for discovery checks
                    let peer_count = self.sync_manager.read().await.peer_count();

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

                    // Build sorted list of known producers.
                    // Primary source: GSet CRDT (persistent, converges via signed gossip).
                    // Fallback: known_producers Vec (non-persistent, rebuilt from peer status).
                    let gset_producers = {
                        let gset = self.producer_gset.read().await;
                        gset.active_producers(7200) // 2h liveness window
                    };
                    let mut known_producers: Vec<PublicKey> = if !gset_producers.is_empty() {
                        gset_producers
                    } else {
                        let known = self.known_producers.read().await;
                        known.clone()
                    };

                    // During genesis, use the registered producer set (from chainspec)
                    // instead of relying on gossip discovery, which may not have
                    // converged yet. This ensures all nodes agree on the producer
                    // list from the start, preventing independent production and forks.
                    if in_genesis {
                        let producers = self.producer_set.read().await;
                        let active = producers.active_producers_at_height(height);
                        for p in &active {
                            if !known_producers.contains(&p.public_key) {
                                known_producers.push(p.public_key);
                            }
                        }
                    }

                    // Always include ourselves
                    if !known_producers.iter().any(|p| p == &our_pubkey) {
                        known_producers.push(our_pubkey);
                    }

                    // Filter out producers who are registered but haven't passed
                    // ACTIVATION_DELAY yet. Without this, a registration changes the
                    // round-robin denominator (slot % N) before all nodes agree on N,
                    // causing forks when new producers join mid-chain.
                    //
                    // Exception: genesis producers (registered_at == 0) are always
                    // included — they ARE the initial set and waiting would deadlock.
                    {
                        let producers = self.producer_set.read().await;
                        known_producers.retain(|pk| {
                            match producers.get_by_pubkey(pk) {
                                Some(info) => {
                                    if !info.is_active() {
                                        return false;
                                    }
                                    // Genesis producers: always eligible
                                    if info.registered_at == 0 {
                                        return true;
                                    }
                                    // Late joiners: must wait activation delay
                                    height >= info.registered_at + storage::ACTIVATION_DELAY
                                }
                                None => {
                                    // Not registered (gossip-discovered): include in bootstrap
                                    true
                                }
                            }
                        });
                    }

                    // Sort for deterministic ordering (all nodes compute same order)
                    known_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                    let num_producers = known_producers.len();
                    if num_producers == 0 {
                        warn!("Bootstrap round-robin: no eligible producers after filtering, skipping slot {}", current_slot);
                        return Ok(());
                    }

                    // Liveness filter: split producers into live vs stale.
                    // Live = produced a block within the liveness window.
                    // Stale = haven't produced recently (slots wasted waiting for them).
                    let liveness_window = std::cmp::max(
                        consensus::LIVENESS_WINDOW_MIN,
                        (num_producers as u64).saturating_mul(3),
                    );
                    let chain_height = height.saturating_sub(1);
                    let cutoff = chain_height.saturating_sub(liveness_window);
                    let (live_producers, stale_producers): (Vec<PublicKey>, Vec<PublicKey>) = {
                        let (live, stale): (Vec<_>, Vec<_>) =
                            known_producers.iter().partition(|pk| {
                                match self.producer_liveness.get(pk) {
                                    Some(&last_h) => last_h >= cutoff,
                                    // No chain record: live if chain is young, stale otherwise
                                    None => chain_height < liveness_window,
                                }
                            });
                        (
                            live.into_iter().copied().collect(),
                            stale.into_iter().copied().collect(),
                        )
                    };

                    // Deadlock safety: if all stale, treat all as live
                    let (live_producers, stale_producers) = if live_producers.is_empty() {
                        (known_producers.clone(), Vec::new())
                    } else {
                        (live_producers, stale_producers)
                    };

                    if !stale_producers.is_empty() {
                        debug!(
                            "Liveness filter: {} live, {} stale (window={})",
                            live_producers.len(),
                            stale_producers.len(),
                            liveness_window,
                        );
                    }

                    // Build eligible list using liveness-aware scheduling.
                    // Normal slots: all ranks from live producers only.
                    // Re-entry slots: one stale producer at rank 0, live at ranks 1+.
                    let eligible = doli_core::validation::bootstrap_schedule_with_liveness(
                        current_slot,
                        &live_producers,
                        &stale_producers,
                        consensus::REENTRY_INTERVAL,
                    );

                    if !eligible.contains(&our_pubkey) {
                        return Ok(());
                    }

                    debug!(
                        "Bootstrap fallback: slot={}, {} producers, eligible={}",
                        current_slot,
                        num_producers,
                        eligible.len()
                    );

                    // Pass eligible list to the standard time-window check below.
                    // our_bootstrap_rank = None means it uses is_producer_eligible_ms
                    // (the same 2s exclusive windows as the epoch scheduler).
                    (eligible, None::<u8>)
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
            // Uses cached DeterministicScheduler for O(log n) lookups per slot.
            let current_epoch = self.params.slot_to_epoch(current_slot) as u64;
            let active_count = active_with_weights.len();
            let total_bonds: u64 = active_with_weights.iter().map(|(_, b)| *b).sum();
            let scheduler = match &self.cached_scheduler {
                Some((epoch, count, bonds, sched))
                    if *epoch == current_epoch
                        && *count == active_count
                        && *bonds == total_bonds =>
                {
                    sched
                }
                _ => {
                    // Build new scheduler (epoch changed or active producer set changed)
                    info!(
                        "Rebuilding scheduler: epoch={}, producers={}, total_bonds={}",
                        current_epoch, active_count, total_bonds
                    );
                    let producers: Vec<ScheduledProducer> = active_with_weights
                        .iter()
                        .map(|(pk, bonds)| ScheduledProducer::new(*pk, *bonds as u32))
                        .collect();
                    self.cached_scheduler = Some((
                        current_epoch,
                        active_count,
                        total_bonds,
                        DeterministicScheduler::new(producers),
                    ));
                    &self.cached_scheduler.as_ref().unwrap().3
                }
            };
            // Dedup: a producer may appear at multiple ranks (small producer sets).
            // Without dedup, position() returns the first occurrence, masking later ranks.
            // This matches the dedup in select_producer_for_slot() (consensus.rs).
            let mut eligible: Vec<PublicKey> =
                Vec::with_capacity(self.config.network.params().max_fallback_ranks);
            for rank in 0..self.config.network.params().max_fallback_ranks {
                if let Some(pk) = scheduler.select_producer(current_slot, rank).cloned() {
                    if !eligible.contains(&pk) {
                        eligible.push(pk);
                    }
                }
            }
            (eligible, None)
        };

        if eligible.is_empty() {
            return Ok(());
        }

        // Calculate slot offset in milliseconds for eligibility window
        let slot_start = self.params.slot_to_timestamp(current_slot);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let slot_start_ms = slot_start * 1000;
        let slot_offset_ms = now_ms.saturating_sub(slot_start_ms);

        // Check if we're eligible at this time
        // For bootstrap mode, use continuous time-based scheduling
        // For normal mode, use the standard eligibility check (ms-precision)
        let is_eligible = if let Some(score) = our_bootstrap_rank {
            // Bootstrap mode: continuous time-based scheduling
            // The score (0-255) determines when we should produce within the slot.
            // We can produce when the current time offset exceeds our target offset.
            let slot_duration_ms = self.params.slot_duration * 1000;
            let max_offset_percent = 80; // Leave 20% for propagation
            let target_offset_percent = (score as u64 * max_offset_percent) / 255;
            let target_offset_ms = (slot_duration_ms * target_offset_percent) / 100;

            // We're eligible if current offset >= our target offset
            slot_offset_ms >= target_offset_ms
        } else {
            // Normal mode: ms-precision sequential eligibility check
            consensus::is_producer_eligible_ms(&our_pubkey, &eligible, slot_offset_ms)
        };

        if !is_eligible {
            return Ok(());
        }

        // For devnet, add a minimum delay to allow heartbeat collection
        // Scale delay to 20% of slot duration (capped at 700ms for long slots)
        // This prevents the delay from consuming too much of short slots
        if self.config.network == Network::Devnet {
            let slot_duration_ms = self.params.slot_duration * 1000;
            let heartbeat_collection_ms = std::cmp::min(slot_duration_ms / 5, 700);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let slot_start_ms = slot_start * 1000;
            let offset_ms = now_ms.saturating_sub(slot_start_ms);
            if offset_ms < heartbeat_collection_ms {
                return Ok(()); // Too early, wait for heartbeats
            }
        }

        // We're eligible - produce a block!
        info!(
            "Producing block for slot {} at height {} (offset {}ms)",
            current_slot, height, slot_offset_ms
        );

        // =========================================================================
        // PROPAGATION RACE MITIGATION: Yield before VDF to catch in-flight blocks
        //
        // Problem: During VDF computation (~55ms), the event loop is blocked and
        // cannot process incoming gossip blocks. If another producer's block for
        // slot S is in-flight while we start producing for slot S+1, we won't see
        // it until after we've already broadcast our block, creating a fork.
        //
        // Solution: Before starting VDF computation, yield control briefly to allow
        // any pending network events to be processed. This gives in-flight blocks
        // a chance to arrive before we commit to production.
        //
        // We yield if:
        // 1. Network tip slot suggests there might be a recent block we haven't seen
        // 2. We're not too far into the slot (leave time for our own production)
        //
        // This is a lightweight "micro-sync" that doesn't require protocol changes.
        // =========================================================================
        {
            let network_tip_slot = self.sync_manager.read().await.best_peer_slot();

            // If network tip is at current_slot-1 or current_slot, there might be
            // an in-flight block we should wait for before producing
            if network_tip_slot >= prev_slot && current_slot > prev_slot + 1 {
                // We're potentially missing a block - yield briefly
                debug!(
                    "Propagation race mitigation: yielding before VDF (prev_slot={}, network_tip={}, current={})",
                    prev_slot, network_tip_slot, current_slot
                );

                // Yield for a short time to allow pending network events to be processed
                // This is much shorter than a full slot - just enough for gossip propagation
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                // Re-check: did a block arrive for a more recent slot?
                let new_state = self.chain_state.read().await;
                let new_prev_slot = new_state.best_slot;
                drop(new_state);

                if new_prev_slot > prev_slot {
                    debug!(
                        "Block arrived during yield (prev_slot: {} -> {}) - restarting production check",
                        prev_slot, new_prev_slot
                    );
                    // A block arrived - abort this production attempt
                    // Next production tick will re-evaluate with updated chain state
                    return Ok(());
                }
            }
        }

        // SIGNED SLOTS PROTECTION: Check if we already signed this slot
        // This prevents double-signing if we restart quickly
        if let Some(ref signed_slots) = self.signed_slots_db {
            if let Err(e) = signed_slots.check_and_mark(current_slot as u64) {
                error!("SLASHING PROTECTION: {}", e);
                return Ok(());
            }
        }

        // Build the block
        let mut builder =
            BlockBuilder::new(prev_hash, prev_slot, our_pubkey).with_params(self.params.clone());

        // AUTOMATIC EPOCH REWARDS: At epoch boundaries, calculate and distribute
        // rewards to all producers who were present in the completed epoch.
        // Rewards are immediately available in producer wallets (no claim needed).
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let epoch_coinbase: Option<Transaction> = if height > 0
            && reward_epoch::is_epoch_start_with(height, blocks_per_epoch)
        {
            let completed_epoch = (height / blocks_per_epoch) - 1;
            info!(
                "Epoch {} completed at height {}, calculating automatic rewards...",
                completed_epoch, height
            );

            // Calculate rewards for all present producers in the completed epoch
            let epoch_outputs = self.calculate_epoch_rewards(completed_epoch).await;

            if !epoch_outputs.is_empty() {
                let total_reward: u64 = epoch_outputs.iter().map(|(amt, _)| *amt).sum();
                info!(
                    "Distributing {} total reward to {} producers for epoch {}",
                    total_reward,
                    epoch_outputs.len(),
                    completed_epoch
                );

                // Create epoch reward coinbase transaction with all producer rewards
                let coinbase =
                    Transaction::new_epoch_reward_coinbase(epoch_outputs, height, completed_epoch);
                builder.add_transaction(coinbase.clone());
                Some(coinbase)
            } else {
                debug!(
                    "No producers present in epoch {}, skipping reward distribution",
                    completed_epoch
                );
                None
            }
        } else {
            None
        };

        // Add transactions from mempool
        let mempool_txs: Vec<Transaction> = {
            let mempool = self.mempool.read().await;
            mempool.select_for_block(1_000_000) // Up to ~1MB of transactions per block
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

        // =========================================================================
        // DRAIN PENDING BLOCKS: Process all queued network events before VDF
        //
        // This is the key fix for the propagation race bug. The problem:
        // - VDF computation takes ~550-700ms
        // - During this time, the event loop is blocked (we await spawn_blocking)
        // - Blocks from other producers may arrive in the network channel queue
        // - Without draining, we don't see them until after we've already produced
        //
        // Solution: Non-blocking drain of all pending NewBlock events from the
        // network channel BEFORE starting VDF. This ensures we have the latest
        // chain state before committing to production.
        //
        // The try_next_event() method uses try_recv() which returns immediately
        // if no events are queued, so this adds negligible latency.
        // =========================================================================
        {
            // First, collect pending events (releases borrow of self.network quickly)
            let pending_events: Vec<NetworkEvent> = {
                if let Some(ref mut network) = self.network {
                    let mut events = Vec::new();
                    // Drain ALL pending events (not just 10)
                    // With 20+ nodes, limiting to 10 can leave blocks unprocessed
                    while let Some(event) = network.try_next_event() {
                        events.push(event);
                    }
                    events
                } else {
                    Vec::new()
                }
            };

            // Now process collected events (self.network borrow is released)
            if !pending_events.is_empty() {
                let mut block_count = 0;
                for event in pending_events {
                    if matches!(event, NetworkEvent::NewBlock(_, _)) {
                        block_count += 1;
                    }
                    if let Err(e) = self.handle_network_event(event).await {
                        warn!("Error handling drained event: {}", e);
                    }
                }

                if block_count > 0 {
                    debug!("Drained {} pending block events before VDF", block_count);

                    // Check if chain advanced during drain
                    let new_state = self.chain_state.read().await;
                    if new_state.best_slot > prev_slot || new_state.best_height >= height {
                        debug!(
                            "Chain advanced during drain (new tip after prev_slot={}) - aborting production",
                            prev_slot
                        );
                        return Ok(());
                    }
                }
            }
        }

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

            info!("VDF computed in {:?} (target: ~55ms)", vdf_duration);
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

        // SAFETY CHECK: Verify chain state hasn't changed during VDF computation
        //
        // The VDF takes ~700ms during which the event loop is blocked. Other
        // producers' blocks may have arrived and been queued. We must check:
        // 1. No block exists for our current slot (same-slot duplicate)
        // 2. Chain tip hasn't advanced (stale parent detection)
        //
        // Without check #2, we'd build on a stale parent and create a fork.
        if self.block_store.has_block_for_slot(current_slot as u64) {
            debug!(
                "Block appeared during VDF computation for slot {} - aborting production",
                current_slot
            );
            return Ok(());
        }

        // Check if chain tip advanced during VDF (stale parent detection)
        {
            let post_vdf_state = self.chain_state.read().await;
            if post_vdf_state.best_height >= height || post_vdf_state.best_hash != prev_hash {
                info!(
                    "Chain advanced during VDF computation (tip moved from height {} to {}) - aborting to avoid fork",
                    height - 1, post_vdf_state.best_height
                );
                return Ok(());
            }
        }

        // Create final block header with VDF
        let final_header = BlockHeader {
            version: header.version,
            prev_hash: header.prev_hash,
            merkle_root: header.merkle_root,
            presence_root: header.presence_root,
            timestamp: header.timestamp,
            slot: header.slot,
            producer: header.producer,
            vdf_output,
            vdf_proof,
        };

        // Collect all transactions for the block:
        // 1. Block reward coinbase (100% to producer, like Bitcoin)
        // 2. Epoch reward coinbase (if this is an epoch boundary)
        // 3. Transactions from mempool
        let mut transactions = Vec::new();

        // Create block reward coinbase - producer gets 100% of block reward
        let block_reward = self.params.block_reward(height);
        // Use domain-separated hash to match wallet address format
        let producer_pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, our_pubkey.as_bytes());
        let block_coinbase = Transaction::new_coinbase(block_reward, producer_pubkey_hash, height);
        transactions.push(block_coinbase);
        info!(
            "Block coinbase: {} units to producer {}",
            block_reward,
            hex::encode(&producer_pubkey_hash.as_bytes()[..8])
        );

        // Add epoch reward coinbase if at epoch boundary
        if let Some(coinbase) = epoch_coinbase {
            transactions.push(coinbase);
        }

        // During genesis: include VDF proof Registration TX (zero-bond, VDF-only)
        if self.config.network.is_in_genesis(height)
            && self.genesis_vdf_output.is_some()
            && !self.genesis_vdf_submitted
        {
            let vdf_output_bytes = self.genesis_vdf_output.unwrap();
            let reg_data = RegistrationData {
                public_key: our_pubkey,
                epoch: 0,
                vdf_output: vdf_output_bytes.to_vec(),
                vdf_proof: vec![],
                prev_registration_hash: Hash::ZERO,
                sequence_number: 0,
                bond_count: 0, // Zero bond — handled at genesis end
            };
            let extra_data =
                bincode::serialize(&reg_data).expect("RegistrationData serialization cannot fail");
            let reg_tx = Transaction {
                version: 1,
                tx_type: TxType::Registration,
                inputs: vec![],
                outputs: vec![],
                extra_data,
            };
            transactions.push(reg_tx);
            self.genesis_vdf_submitted = true;
            info!(
                "Included genesis VDF proof Registration TX in block {} for {}",
                height,
                hex::encode(&our_pubkey.as_bytes()[..8])
            );
        }

        transactions.extend(mempool_txs);

        let block = Block {
            header: final_header,
            transactions,
        };

        let block_hash = block.hash();
        info!(
            "[BLOCK_PRODUCED] hash={} height={} slot={} parent={}",
            block_hash, height, current_slot, block.header.prev_hash
        );

        // Apply the block locally
        self.apply_block(block.clone()).await?;

        // Reset stale chain timer — we just produced a block
        self.sync_manager
            .write()
            .await
            .note_block_received_via_gossip();

        // Mark that we produced for this slot
        self.last_produced_slot = current_slot;

        // Broadcast the block to the network
        // This is only done for blocks we produce ourselves - received blocks
        // are already on the network and don't need to be re-broadcast.
        // Header is sent first so peers can pre-validate before the full block arrives.
        if let Some(ref network) = self.network {
            let _ = network.broadcast_header(block.header.clone()).await;
            let _ = network.broadcast_block(block).await;
        }

        // Self-attest: producer immediately attests to their own block
        self.create_and_broadcast_attestation(block_hash, current_slot, height)
            .await;

        Ok(())
    }

    /// Calculate epoch rewards for all present producers in the completed epoch.
    ///
    /// This method scans all blocks in the specified epoch and calculates the
    /// weighted presence reward for each producer who was present.
    ///
    /// Returns a vector of (amount, pubkey_hash) tuples for creating the
    /// epoch reward coinbase transaction.
    async fn calculate_epoch_rewards(&self, epoch: u64) -> Vec<(u64, Hash)> {
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();

        // Calculate the height at the end of this epoch for producer eligibility
        let epoch_end_height = (epoch + 1) * blocks_per_epoch;

        // Get the list of producers eligible at epoch end (clone to release the lock)
        let mut sorted_producers: Vec<storage::producer::ProducerInfo> = {
            let producers = self.producer_set.read().await;
            producers
                .active_producers_at_height(epoch_end_height)
                .iter()
                .map(|p| (*p).clone())
                .collect()
        };

        if sorted_producers.is_empty() {
            return Vec::new();
        }

        // Sort producers by pubkey for deterministic ordering (same as presence commitment)
        sorted_producers.sort_by(|a, b| a.public_key.as_bytes().cmp(b.public_key.as_bytes()));

        // Create reward calculator with network-specific epoch size
        let calculator = WeightedRewardCalculator::with_blocks_per_epoch(
            self.block_store.as_ref(),
            &self.params,
            blocks_per_epoch,
        );

        // Calculate rewards for each producer
        let mut reward_outputs = Vec::new();

        for (index, producer_info) in sorted_producers.iter().enumerate() {
            #[allow(deprecated)]
            match calculator.calculate_producer_reward(&producer_info.public_key, index, epoch) {
                Ok(result) => {
                    if result.has_reward() {
                        let total_reward = result.reward_amount;
                        let pubkey_hash =
                            hash_with_domain(ADDRESS_DOMAIN, producer_info.public_key.as_bytes());

                        info!(
                            "Producer {} earned {} in epoch {} (present in {}/{} blocks)",
                            producer_info.public_key,
                            total_reward,
                            epoch,
                            result.blocks_present,
                            result.total_blocks
                        );

                        // Split rewards if producer has received delegations
                        if producer_info.received_delegations.is_empty() {
                            // No delegations: 100% to producer
                            reward_outputs.push((total_reward, pubkey_hash));
                        } else {
                            let own_bonds = producer_info.bond_count as u64;
                            let delegated: u64 = producer_info
                                .received_delegations
                                .iter()
                                .map(|(_, c)| *c as u64)
                                .sum();
                            let total_bonds = own_bonds + delegated;

                            // Producer keeps: own_bond_share + DELEGATE_REWARD_PCT of delegated share
                            let own_share = total_reward * own_bonds / total_bonds;
                            let delegated_share = total_reward - own_share;
                            let delegate_fee = delegated_share * DELEGATE_REWARD_PCT as u64 / 100;
                            let producer_total = own_share + delegate_fee;

                            if producer_total > 0 {
                                reward_outputs.push((producer_total, pubkey_hash));
                            }

                            // Distribute STAKER_REWARD_PCT to each delegator proportionally
                            let staker_pool = delegated_share * STAKER_REWARD_PCT as u64 / 100;
                            for (delegator_hash, bond_count) in &producer_info.received_delegations
                            {
                                let delegator_reward =
                                    staker_pool * (*bond_count as u64) / delegated;
                                if delegator_reward > 0 {
                                    reward_outputs.push((delegator_reward, *delegator_hash));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to calculate reward for producer {} in epoch {}: {}",
                        producer_info.public_key, epoch, e
                    );
                }
            }
        }

        reward_outputs
    }

    // NOTE: build_presence_commitment removed in deterministic scheduler model
    // Rewards go 100% to block producer via coinbase, no presence tracking needed

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

    /// Rebuild the producer set by replaying all producer-modifying transactions
    /// from blocks 1 through `target_height`. Called by rollback/reorg paths.
    ///
    /// Processes: Registration, Exit, SlashProducer, AddBond, DelegateBond,
    /// RevokeDelegation, and unbonding transitions — mirroring `apply_block()`.
    fn rebuild_producer_set_from_blocks(
        &self,
        producers: &mut ProducerSet,
        target_height: u64,
    ) -> Result<()> {
        producers.clear();
        let bond_unit = self.config.network.bond_unit();
        let genesis_blocks = self.config.network.genesis_blocks();

        for height in 1..=target_height {
            let block = self
                .block_store
                .get_block_by_height(height)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Producer set rebuild: missing block at height {} (store corrupted)",
                        height
                    )
                })?;
            for tx in &block.transactions {
                match tx.tx_type {
                    TxType::Registration => {
                        // During genesis, Registration TXs are VDF proof containers
                        // (zero-bond). Skip — genesis producers are registered below
                        // when crossing the genesis boundary.
                        if genesis_blocks > 0 && height <= genesis_blocks {
                            continue;
                        }
                        if let Some(reg_data) = tx.registration_data() {
                            if let Some((bond_index, bond_output)) =
                                tx.outputs.iter().enumerate().find(|(_, o)| {
                                    o.output_type == doli_core::transaction::OutputType::Bond
                                })
                            {
                                let tx_hash = tx.hash();
                                let era = self.params.height_to_era(height);
                                let producer_info = storage::ProducerInfo::new_with_bonds(
                                    reg_data.public_key,
                                    height,
                                    bond_output.amount,
                                    (tx_hash, bond_index as u32),
                                    era,
                                    reg_data.bond_count,
                                );
                                let _ = producers.register(producer_info, height);
                            }
                        }
                    }
                    TxType::Exit => {
                        if let Some(exit_data) = tx.exit_data() {
                            let _ = producers.request_exit(&exit_data.public_key, height);
                        }
                    }
                    TxType::SlashProducer => {
                        if let Some(slash_data) = tx.slash_data() {
                            let _ = producers.slash_producer(&slash_data.producer_pubkey, height);
                        }
                    }
                    TxType::AddBond => {
                        if let Some(add_bond_data) = tx.add_bond_data() {
                            let tx_hash = tx.hash();
                            let bond_outpoints: Vec<(crypto::Hash, u32)> = (0..add_bond_data
                                .bond_count)
                                .map(|i| (tx_hash, i))
                                .collect();
                            if let Some(producer_info) =
                                producers.get_by_pubkey_mut(&add_bond_data.producer_pubkey)
                            {
                                producer_info.add_bonds(bond_outpoints, bond_unit);
                            }
                        }
                    }
                    TxType::DelegateBond => {
                        if let Some(data) = tx.delegate_bond_data() {
                            let _ = producers.delegate_bonds(
                                &data.delegator,
                                &data.delegate,
                                data.bond_count,
                            );
                        }
                    }
                    TxType::RevokeDelegation => {
                        if let Some(data) = tx.revoke_delegation_data() {
                            let _ = producers.revoke_delegation(&data.delegator);
                        }
                    }
                    _ => {} // Other TX types don't modify producer set
                }
            }

            // Replicate GENESIS PHASE COMPLETE: register VDF-proven producers
            // when crossing the genesis boundary during rebuild.
            if genesis_blocks > 0 && height == genesis_blocks + 1 {
                let genesis_producers = self.derive_genesis_producers_from_chain();
                let era = self.params.height_to_era(height);
                for pubkey in &genesis_producers {
                    let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());
                    // registered_at = 0: Genesis producers exempt from ACTIVATION_DELAY
                    let producer_info = storage::ProducerInfo::new_with_bonds(
                        *pubkey,
                        0,
                        bond_unit,
                        (bond_hash, 0),
                        era,
                        1,
                    );
                    let _ = producers.register(producer_info, height);
                }
            }

            // Process completed unbonding periods after each block
            producers.process_unbonding(height, UNBONDING_PERIOD);
        }
        Ok(())
    }

    /// Unconditionally roll back 1 block for fork recovery.
    ///
    /// Unlike `resolve_shallow_fork()`, this has no `empty_headers` or
    /// `shallow_rollback_count` preconditions — it just rolls back. Used by
    /// `maybe_auto_resync()` on mainnet where same-height forks never trigger
    /// empty header responses (peers are at the same height, not ahead).
    ///
    /// Returns `Ok(true)` if rollback succeeded, `Ok(false)` if at height 0.
    async fn rollback_one_block(&mut self) -> Result<bool> {
        let local_height = {
            let sync = self.sync_manager.read().await;
            sync.local_tip().0
        };

        if local_height == 0 {
            return Ok(false);
        }

        let target_height = local_height - 1;
        let genesis_hash = self.chain_state.read().await.genesis_hash;

        let (parent_hash, parent_slot) = if target_height == 0 {
            (genesis_hash, 0u32)
        } else {
            match self.block_store.get_block_by_height(target_height)? {
                Some(parent_block) => (parent_block.hash(), parent_block.header.slot),
                None => {
                    error!("Cannot rollback: no block at height {}", target_height);
                    return Ok(false);
                }
            }
        };

        info!(
            "Rolling back from height {} to {} for fork recovery",
            local_height, target_height
        );

        // Rebuild UTXO set from blocks 1..=target_height
        {
            let mut utxo = self.utxo_set.write().await;
            utxo.clear();
            for height in 1..=target_height {
                let block = self
                    .block_store
                    .get_block_by_height(height)?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Rollback UTXO rebuild: missing block at height {} (store corrupted)",
                            height
                        )
                    })?;
                for (tx_index, tx) in block.transactions.iter().enumerate() {
                    let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                    if !is_reward_tx {
                        let _ = utxo.spend_transaction(tx);
                    }
                    utxo.add_transaction(tx, height, is_reward_tx);
                }
            }
        }

        // Rebuild producer set from blocks 1..=target_height (all TX types)
        {
            let mut producers = self.producer_set.write().await;
            self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
        }

        // Update chain state to parent
        {
            let mut state = self.chain_state.write().await;
            state.best_height = target_height;
            state.best_hash = parent_hash;
            state.best_slot = parent_slot;
        }

        // Update sync manager: local tip + reset fork signals
        {
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(target_height, parent_hash, parent_slot);
            sync.reset_sync_for_rollback();
        }

        // Save state to persist the rollback
        self.save_state().await?;

        info!(
            "Fork recovery rollback complete: now at height {} (hash {:.8})",
            target_height, parent_hash
        );

        Ok(true)
    }

    /// Detect and resolve shallow forks (1-block orphan tips from ungraceful shutdown).
    ///
    /// When a node is killed during block production, it may persist a block that
    /// peers never received. On restart, peers don't recognize our tip hash, causing
    /// consecutive empty header responses. If the gap to peers is small (<100 blocks),
    /// this is a shallow fork — not a deep fork needing genesis resync.
    ///
    /// Fix: roll back one block at a time until we reach a tip peers recognize.
    /// Capped at 10 rollbacks — if the fork is deeper than that, it's not shallow.
    /// Returns `true` if a rollback was performed (caller should skip other periodic tasks).
    async fn resolve_shallow_fork(&mut self) -> Result<bool> {
        let (empty_headers, local_height, fork_sync_active) = {
            let sync = self.sync_manager.read().await;
            (
                sync.consecutive_empty_headers(),
                sync.local_tip().0,
                sync.is_fork_sync_active(),
            )
        };

        // Don't start a new fork sync if one is already running
        if fork_sync_active {
            return Ok(false);
        }

        // Need at least 3 fork evidence signals before activating
        if empty_headers < 3 || local_height == 0 {
            return Ok(false);
        }

        let started = self.sync_manager.write().await.start_fork_sync();
        if started {
            info!(
                "Fork sync: binary search for common ancestor initiated \
                 (empty_headers={}, local_height={})",
                empty_headers, local_height
            );
            return Ok(true);
        }
        Ok(false)
    }

    /// Run periodic tasks
    async fn run_periodic_tasks(&mut self) -> Result<()> {
        // Apply pending sync blocks in correct order BEFORE cleanup.
        //
        // The body downloader fetches blocks in parallel, so they arrive out of order.
        // handle_response() returns them to handle_new_block() which requires strict
        // chain order (prev_hash == tip). Out-of-order blocks get orphaned.
        //
        // get_blocks_to_apply() walks pending_headers in order and extracts matching
        // bodies from pending_blocks, returning them in the correct chain order.
        // This MUST run before cleanup() so blocks get applied before the stuck
        // timeout fires and clears pending state.
        {
            let blocks = self.sync_manager.write().await.get_blocks_to_apply();
            if !blocks.is_empty() {
                info!("Applying {} pending sync blocks in order", blocks.len());
                for block in blocks {
                    if let Err(e) = self.apply_block(block).await {
                        warn!("Failed to apply pending sync block: {}", e);
                        break;
                    }
                }
            }
        }

        // Clean up sync manager and prune stale finality entries
        {
            let current_slot = {
                let state = self.chain_state.read().await;
                state.best_slot
            };
            let mut sync = self.sync_manager.write().await;
            sync.cleanup();
            sync.prune_finality(current_slot);
        }

        // Expire old mempool transactions
        self.mempool.write().await.expire_old();

        // Poll fork recovery: check if parent chain reached our block_store
        {
            let parent_hash = self
                .sync_manager
                .read()
                .await
                .fork_recovery_current_parent();
            if let Some(parent_hash) = parent_hash {
                let parent_known = self.block_store.has_block(&parent_hash).unwrap_or(false);
                if parent_known {
                    let completed = self
                        .sync_manager
                        .write()
                        .await
                        .check_fork_recovery_connection(true);
                    if let Some(recovery) = completed {
                        if let Err(e) = self.handle_completed_fork_recovery(recovery).await {
                            warn!("Fork recovery reorg failed: {}", e);
                        }
                    }
                }
            }
        }

        // PEER MAINTENANCE: Periodically redial bootstrap nodes when isolated.
        // Unlike stale chain detection (which requires chain inactivity), this runs
        // even when the node is producing solo — ensuring reconnection attempts
        // happen regardless of local production state.
        {
            let peer_count = self.sync_manager.read().await.peer_count();
            if peer_count == 0 && !self.config.bootstrap_nodes.is_empty() {
                // Redial every 10 seconds (slot duration) when isolated
                let should_redial = self
                    .last_peer_redial
                    .map(|t| t.elapsed().as_secs() >= self.params.slot_duration)
                    .unwrap_or(true);
                if should_redial {
                    self.last_peer_redial = Some(std::time::Instant::now());
                    if let Some(ref network) = self.network {
                        for addr in &self.config.bootstrap_nodes {
                            let _ = network.connect(addr).await;
                        }
                    }
                }
            }
        }

        // STALE CHAIN DETECTION (Ethereum-style):
        // If we haven't received any block (gossip or sync) for 3 slots, something is wrong.
        // Diagnose: no peers → re-bootstrap Kademlia; peers exist → aggressive status requests.
        // Status responses trigger update_peer() → should_sync() → start_sync() automatically.
        {
            let stale_threshold = Duration::from_secs(self.params.slot_duration * 3);
            let (is_stale, is_syncing, peer_count) = {
                let sync = self.sync_manager.read().await;
                (
                    sync.is_chain_stale(stale_threshold),
                    sync.state().is_syncing(),
                    sync.peer_count(),
                )
            };

            if is_stale && !is_syncing {
                if peer_count == 0 {
                    // No peers — redial bootstrap nodes and re-bootstrap DHT
                    info!("Stale chain detected (no blocks for 3 slots) with 0 peers — redialing bootstrap nodes");
                    if let Some(ref network) = self.network {
                        for addr in &self.config.bootstrap_nodes {
                            if let Err(e) = network.connect(addr).await {
                                warn!("Failed to redial bootstrap {}: {}", addr, e);
                            }
                        }
                        let _ = network.bootstrap().await;
                    }
                } else {
                    // Peers exist but no blocks — request status from ALL peers
                    // This forces update_peer() which triggers should_sync()/start_sync()
                    debug!(
                        "Stale chain detected with {} peers — requesting status from all",
                        peer_count
                    );
                    if let Some(ref network) = self.network {
                        let genesis_hash = self.chain_state.read().await.genesis_hash;
                        let status_request = if let Some(ref key) = self.producer_key {
                            network::protocols::StatusRequest::with_producer(
                                self.config.network.id(),
                                genesis_hash,
                                *key.public_key(),
                            )
                        } else {
                            network::protocols::StatusRequest::new(
                                self.config.network.id(),
                                genesis_hash,
                            )
                        };
                        let peer_ids: Vec<_> = {
                            let sync = self.sync_manager.read().await;
                            sync.peer_ids().collect()
                        };
                        for peer_id in peer_ids.iter().take(10) {
                            let _ = network
                                .request_status(*peer_id, status_request.clone())
                                .await;
                        }
                    }
                }
            }
        }

        // FORK SYNC: Binary search for common ancestor when on a dead fork.
        // Triggers after 3+ consecutive empty header responses. O(log N) recovery.
        if self.resolve_shallow_fork().await? {
            // Fork sync was just initiated, don't do anything else this tick
            return Ok(());
        }

        // Drive active fork sync: compare probes with block_store, handle transitions
        {
            // Phase 1: Binary search — compare peer's block hash with ours
            let probe = self.sync_manager.read().await.fork_sync_pending_probe();
            if let Some((height, peer_hash)) = probe {
                let our_hash = self.block_store.get_hash_by_height(height).ok().flatten();
                let matches = our_hash.map(|h| h == peer_hash).unwrap_or(false);
                self.sync_manager
                    .write()
                    .await
                    .fork_sync_handle_probe(matches);
            }

            // Transition: search complete — provide ancestor hash from our block_store
            let ancestor_height = self.sync_manager.read().await.fork_sync_ancestor_height();
            if let Some(height) = ancestor_height {
                let ancestor_hash = self
                    .block_store
                    .get_hash_by_height(height)
                    .ok()
                    .flatten()
                    .unwrap_or(self.chain_state.read().await.genesis_hash);
                self.sync_manager
                    .write()
                    .await
                    .fork_sync_set_ancestor(height, ancestor_hash);
            }

            // Phase 2/3 complete: take result and execute reorg
            let result = self.sync_manager.write().await.fork_sync_take_result();
            if let Some(result) = result {
                if let Err(e) = self.execute_fork_sync_reorg(result).await {
                    warn!("Fork sync reorg failed: {}", e);
                }
                return Ok(());
            }
        }

        // GENESIS RESYNC: sync manager detected persistent chain rejection.
        // Peers don't recognize our tip and the gap is too large for shallow fork recovery.
        // Full state reset needed — force_resync_from_genesis() handles the 10-layer reset.
        {
            let needs_resync = self.sync_manager.read().await.needs_genesis_resync();
            if needs_resync {
                warn!("Genesis resync triggered: peers consistently reject our chain tip. Full state reset.");
                self.force_resync_from_genesis().await?;
                return Ok(());
            }
        }

        // DEEP FORK ESCALATION: If peers consistently reject our chain tip (10+ empty
        // header responses), normal sync and fork recovery can't bridge the gap.
        // Escalate to genesis resync — rebuild state from the canonical chain.
        {
            let is_deep_fork = self.sync_manager.read().await.is_deep_fork_detected();
            if is_deep_fork {
                warn!("Deep fork detected: peers consistently reject our chain tip. Initiating genesis resync.");
                self.force_resync_from_genesis().await?;
                return Ok(());
            }
        }

        // Check if we need to request sync
        if let Some((peer_id, request)) = self.sync_manager.write().await.next_request() {
            if let Some(ref network) = self.network {
                let _ = network.request_sync(peer_id, request).await;
            }
        }

        // PERIODIC STATUS REFRESH: Request status from peers to keep network tip updated
        // This is critical for production gating - without fresh peer status, we can't
        // know if other nodes have produced blocks we haven't received via gossip yet.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // During bootstrap (height 0), be VERY aggressive about requesting status
        // from ALL peers - we need to find at least one peer with height > 0
        let local_height = self.chain_state.read().await.best_height;
        let is_bootstrap = local_height == 0;

        // Request status every ~2 seconds during bootstrap, ~5 seconds during normal ops
        let status_interval = if is_bootstrap {
            2 // Aggressive during bootstrap
        } else {
            5 // All networks: 5s keeps peer status fresh for fork detection
        };

        if now_secs.is_multiple_of(status_interval) {
            if let Some(ref network) = self.network {
                let peer_ids: Vec<_> = {
                    let sync = self.sync_manager.read().await;
                    sync.peer_ids().collect()
                };

                if !peer_ids.is_empty() {
                    let genesis_hash = self.chain_state.read().await.genesis_hash;
                    let status_request = if let Some(ref key) = self.producer_key {
                        network::protocols::StatusRequest::with_producer(
                            self.config.network.id(),
                            genesis_hash,
                            *key.public_key(),
                        )
                    } else {
                        network::protocols::StatusRequest::new(
                            self.config.network.id(),
                            genesis_hash,
                        )
                    };

                    if is_bootstrap {
                        // During bootstrap, request from ALL peers to find any with height > 0
                        for peer_id in peer_ids.iter().take(5) {
                            // Limit to 5 to avoid flooding
                            debug!("Bootstrap status request to peer {}", peer_id);
                            let _ = network
                                .request_status(*peer_id, status_request.clone())
                                .await;
                        }
                    } else {
                        // Normal operation - request from one peer at a time
                        let peer_idx = (now_secs as usize) % peer_ids.len();
                        let peer_id = peer_ids[peer_idx];
                        debug!("Periodic status request to peer {}", peer_id);
                        let _ = network.request_status(peer_id, status_request).await;
                    }
                }
            }
        }

        // Periodic health diagnostic — one-line summary every 30s for fork debugging
        if now_secs.is_multiple_of(30) {
            let cs = self.chain_state.read().await;
            let sync = self.sync_manager.read().await;
            let peer_count = sync.peer_count();
            let best_peer_h = sync.best_peer_height();
            let best_peer_s = sync.best_peer_slot();
            let net_tip_h = sync.network_tip_height();
            let net_tip_s = sync.network_tip_slot();
            let sync_fails = sync.consecutive_sync_failure_count();
            warn!(
                "[HEALTH] h={} s={} hash={:.8} | peers={} best_peer_h={} best_peer_s={} net_tip_h={} net_tip_s={} | sync_fails={} fork_counter={} state={:?}",
                cs.best_height, cs.best_slot, cs.best_hash,
                peer_count, best_peer_h, best_peer_s, net_tip_h, net_tip_s,
                sync_fails, self.consecutive_fork_blocks, sync.sync_state_name()
            );
        }

        Ok(())
    }

    /// Get current chain height
    #[allow(dead_code)]
    pub async fn height(&self) -> u64 {
        self.chain_state.read().await.best_height
    }

    /// Get current best hash
    #[allow(dead_code)]
    pub async fn best_hash(&self) -> Hash {
        self.chain_state.read().await.best_hash
    }

    /// Get sync state
    #[allow(dead_code)]
    pub async fn sync_state(&self) -> network::SyncState {
        self.sync_manager.read().await.state().clone()
    }

    /// Get peer count
    #[allow(dead_code)]
    pub async fn peer_count(&self) -> usize {
        if let Some(ref network) = self.network {
            network.peer_count().await
        } else {
            0
        }
    }

    /// Get mempool size
    #[allow(dead_code)]
    pub async fn mempool_size(&self) -> usize {
        self.mempool.read().await.len()
    }

    /// Derive genesis producers from the blockchain.
    ///
    /// Scans blocks 1 through genesis_blocks and extracts unique producers.
    /// This is the source of truth for genesis producers - no gossip or
    /// external configuration required. Any node can verify this by
    /// examining the blockchain.
    /// Derive genesis producers from on-chain VDF proof Registration TXs.
    ///
    /// Scans genesis blocks for Registration TXs, validates VDF proofs,
    /// and returns only producers with valid proofs. Producers who produced
    /// blocks but did NOT submit a VDF proof are NOT registered.
    fn derive_genesis_producers_from_chain(&self) -> Vec<PublicKey> {
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks == 0 {
            return Vec::new();
        }

        let iterations = self.config.network.vdf_register_iterations();
        let mut seen = std::collections::HashSet::new();
        let mut proven_producers = Vec::new();

        for height in 1..=genesis_blocks {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                for tx in &block.transactions {
                    if tx.tx_type == TxType::Registration {
                        if let Some(reg_data) = tx.registration_data() {
                            if reg_data.vdf_output.len() == 32 {
                                let vdf_input = vdf::registration_input(&reg_data.public_key, 0);
                                let output: [u8; 32] =
                                    reg_data.vdf_output.as_slice().try_into().unwrap();
                                if verify_hash_chain_vdf(&vdf_input, &output, iterations) {
                                    if seen.insert(reg_data.public_key) {
                                        info!(
                                            "  VDF proof valid for genesis producer {}",
                                            hex::encode(&reg_data.public_key.as_bytes()[..8])
                                        );
                                        proven_producers.push(reg_data.public_key);
                                    }
                                } else {
                                    warn!(
                                        "  VDF proof INVALID for {} in block {}",
                                        hex::encode(&reg_data.public_key.as_bytes()[..8]),
                                        height
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(
            "Derived {} genesis producers with valid VDF proofs (from {} genesis blocks)",
            proven_producers.len(),
            genesis_blocks
        );
        proven_producers
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
        // Save chain state
        let state_path = self.config.data_dir.join("chain_state.bin");
        self.chain_state.read().await.save(&state_path)?;

        // Save UTXO set
        let utxo_path = self.config.data_dir.join("utxo.bin");
        self.utxo_set.read().await.save(&utxo_path)?;

        // Save producer set
        let producers_path = self.config.data_dir.join("producers.bin");
        self.producer_set.read().await.save(&producers_path)?;

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

// Note: The weighted presence reward system uses automatic EpochReward
// transactions distributed at epoch boundaries. Validation is in
// crates/core/src/validation.rs and tests in crates/core/src/rewards.rs.
