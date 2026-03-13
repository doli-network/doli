//! Node implementation
//!
//! Integrates all DOLI components: storage, networking, RPC, mempool, and sync.
// v0.2.1-test: upgrade pipeline validation

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
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
    DELEGATE_REWARD_PCT, SLOTS_PER_EPOCH, UNBONDING_PERIOD,
};
// WeightedRewardCalculator removed — replaced by attestation-qualified bond-weighted distribution
use doli_core::tpop::calibration::VdfCalibrator;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{RegistrationData, TxType};
use doli_core::types::UNITS_PER_COIN;
use doli_core::validation;
use doli_core::validation::ValidationMode;
use doli_core::{
    attestation_minute, decode_attestation_bitfield, encode_attestation_bitfield, AdaptiveGossip,
    Attestation, Block, BlockHeader, MinuteAttestationTracker, Network, ProducerAnnouncement,
    ProducerGSet, Transaction, ATTESTATION_QUALIFICATION_THRESHOLD,
};
use doli_core::{DeterministicScheduler, ScheduledProducer};
use network::protocols::{SyncRequest, SyncResponse};
use network::{
    EquivocationDetector, EquivocationProof, NetworkCommand, NetworkConfig, NetworkEvent,
    NetworkService, PeerId, ProductionAuthorization, ReorgResult, SyncConfig, SyncManager,
};
use rpc::{Mempool, MempoolPolicy, RpcContext, RpcServer, RpcServerConfig, SyncStatus};
use storage::archiver::ArchiveBlock;
use storage::{BlockStore, ChainState, PendingProducerUpdate, ProducerSet, StateDb, UtxoSet};
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
    /// Unified state database (UTXO + producers + chain state, atomic WriteBatch per block)
    state_db: Arc<StateDb>,
    /// UTXO set (in-memory working copy, populated from state_db on startup)
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
    /// BLS key pair for aggregate attestation signatures
    bls_key: Option<crypto::BlsKeyPair>,
    /// Last slot we successfully produced a block for (to avoid double-producing).
    /// Set after successful broadcast — checked at the top of try_produce_block()
    /// before any eligibility/scheduler work to save CPU and silence fallback-rank noise.
    last_produced_slot: Option<u64>,
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
    /// Cached state root, updated atomically after each block application.
    /// Avoids race conditions when GetStateRoot reads during apply_block.
    /// Tuple: (state_root, block_hash, block_height)
    cached_state_root: Arc<RwLock<Option<(Hash, Hash, u64)>>>,
    /// Cached genesis producers. Invalidated on reorgs crossing genesis boundary.
    cached_genesis_producers: std::sync::OnceLock<Vec<PublicKey>>,
    /// Whether we've already checked inbound peer connectivity (one-shot after 60s)
    port_check_done: bool,
    /// On-chain maintainer set (3-5 members, persisted, bootstrapped from first 5 producers).
    /// Used by the auto-update system for release signature verification.
    maintainer_state: Option<Arc<RwLock<storage::MaintainerState>>>,
    /// Channel to send blocks to the archiver (if --archive-to is set)
    archive_tx: Option<tokio::sync::mpsc::Sender<ArchiveBlock>>,
    /// Blocks waiting for finality before being archived
    pending_archive: std::collections::VecDeque<ArchiveBlock>,
    /// Archive directory path (for catch-up after sync)
    archive_dir: Option<PathBuf>,
    /// Whether archive catch-up has been performed after sync
    archive_caught_up: bool,
    /// WebSocket broadcast sender for real-time events (new blocks, new txs)
    ws_sender: Arc<RwLock<Option<tokio::sync::broadcast::Sender<rpc::WsEvent>>>>,
    /// In-memory tracker for minute attestations received via gossip.
    /// Used by block producer to build the presence_root bitfield.
    minute_tracker: MinuteAttestationTracker,
    /// Consecutive forced recoveries (for exponential backoff: 60→120→300→600→960s)
    consecutive_forced_recoveries: u32,
}

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
        bls_key: Option<crypto::BlsKeyPair>,
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

        // Open unified StateDb (atomic WriteBatch per block)
        let state_db_path = config.data_dir.join("state_db");
        let state_db = Arc::new(StateDb::open(&state_db_path)?);

        // Migration: if state_db is empty but old files exist, migrate into it
        if !state_db.has_state() {
            let state_path = config.data_dir.join("chain_state.bin");
            let producers_path = config.data_dir.join("producers.bin");
            let utxo_rocks_path = config.data_dir.join("utxo_rocks");
            let utxo_path = config.data_dir.join("utxo.bin");

            if state_path.exists() || utxo_rocks_path.exists() || utxo_path.exists() {
                info!("[MIGRATION] Migrating to unified state_db...");

                // Load chain state
                let old_cs = if state_path.exists() {
                    ChainState::load(&state_path)?
                } else {
                    let spec = match config.network {
                        Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
                        Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
                        Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
                    };
                    ChainState::new(spec.genesis_hash())
                };

                // Load producer set
                let old_ps = if producers_path.exists() {
                    ProducerSet::load(&producers_path)?
                } else {
                    ProducerSet::new()
                };

                // Load UTXOs (priority: utxo_rocks > utxo.bin)
                if utxo_rocks_path.exists() {
                    let rocks = storage::RocksDbUtxoStore::open(&utxo_rocks_path)?;
                    let entries = rocks.iter_entries();
                    state_db.import_utxos(entries.iter().map(|(o, e)| (o, e)));
                    info!(
                        "[MIGRATION] Imported {} UTXOs from utxo_rocks",
                        state_db.utxo_len()
                    );
                } else if utxo_path.exists() {
                    let legacy = storage::InMemoryUtxoStore::load(&utxo_path)?;
                    state_db.import_utxos(legacy.iter());
                    info!(
                        "[MIGRATION] Imported {} UTXOs from utxo.bin",
                        state_db.utxo_len()
                    );
                }

                // Write chain state + producers atomically
                state_db.put_chain_state(&old_cs)?;
                state_db.write_producer_set(&old_ps)?;

                info!(
                    "[MIGRATION] Migrated to unified state_db (height={}, {} UTXOs, {} producers)",
                    old_cs.best_height,
                    state_db.utxo_len(),
                    old_ps.active_count(),
                );

                // Backup old files (safety net for rollback to old binary)
                for path in [&state_path, &producers_path] {
                    if path.exists() {
                        let backup = path.with_extension("bin.backup");
                        if let Err(e) = std::fs::rename(path, &backup) {
                            warn!("[MIGRATION] Failed to backup {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        // Load state from StateDb (or create fresh genesis)
        let mut chain_state = if let Some(cs) = state_db.get_chain_state() {
            cs
        } else {
            let spec = match config.network {
                Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
                Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
                Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
            };
            let genesis_hash = spec.genesis_hash();
            let cs = ChainState::new(genesis_hash);
            state_db.put_chain_state(&cs)?;
            cs
        };

        // Load UTXOs from StateDb into in-memory working set
        let utxo_set = {
            let utxo_count = state_db.utxo_len();
            if utxo_count > 0 {
                info!(
                    "[STATE_DB] Loading {} UTXOs into in-memory working set...",
                    utxo_count
                );
                let mut mem = storage::InMemoryUtxoStore::new();
                for (outpoint, entry) in state_db.iter_utxos() {
                    mem.insert(outpoint, entry);
                }
                info!("[STATE_DB] Loaded {} UTXOs", mem.len());
                UtxoSet::InMemory(mem)
            } else {
                UtxoSet::new()
            }
        };
        let utxo_set = Arc::new(RwLock::new(utxo_set));

        // Validate genesis hash against embedded chainspec (detect state_db corruption).
        let canonical = {
            let spec = match config.network {
                Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
                Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
                Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
            };
            spec.genesis_hash()
        };
        if chain_state.genesis_hash != canonical {
            warn!(
                "Genesis hash mismatch: state_db={} chainspec={} — NEW CHAIN DETECTED. Wiping all state and block store.",
                &chain_state.genesis_hash.to_string()[..16],
                &canonical.to_string()[..16],
            );
            // Wipe block store — old-chain blocks poison sync via "already in store" guard
            if let Err(e) = block_store.clear() {
                warn!("Failed to clear block store during genesis reset: {}", e);
            }
            // Reset chain state and UTXOs to genesis
            chain_state = ChainState::new(canonical);
            state_db.clear_and_write_genesis(&chain_state);
            // Clear the in-memory UTXO set we already loaded (it's from the old chain)
            *utxo_set.write().await = UtxoSet::new();
            info!(
                "Chain reset complete. Node will sync from genesis on the new chain (genesis={})",
                &canonical.to_string()[..16]
            );
        }
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
            let mut h = 1u64;
            while let Ok(Some(block)) = block_store.get_block_by_height(h) {
                chain_state.best_hash = block.hash();
                chain_state.best_height = h;
                chain_state.best_slot = block.header.slot;
                recovered_height = h;
                h += 1;
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

        // Load or create producer set.
        // ALWAYS try StateDb first — it has the authoritative persisted state
        // (including post-genesis registrations). The provided set from main.rs
        // only has hardcoded genesis producers and would lose any runtime registrations.
        let producer_set = {
            let loaded = state_db.load_producer_set();
            let set = if loaded.active_count() > 0 {
                info!(
                    "[STATE_DB] Loaded {} producers from state_db",
                    loaded.active_count()
                );
                // If caller provided an Arc, replace its contents with the StateDb version
                // so the UpdateService (which shares this Arc) also sees the correct producers.
                if let Some(ref provided) = producer_set {
                    let mut guard = provided.write().await;
                    *guard = loaded.clone();
                    drop(guard);
                }
                loaded
            } else if let Some(ref provided) = producer_set {
                // StateDb is empty — use the provided set (hardcoded genesis producers)
                let guard = provided.read().await;
                let cloned = guard.clone();
                info!(
                    "StateDb empty, using provided producer set ({} producers)",
                    cloned.active_count()
                );
                drop(guard);
                cloned
            } else if config.network == Network::Testnet {
                // For testnet: initialize with genesis producers
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
            };
            // Startup verification: ensure genesis producers match block store.
            // Catches stale ProducerSet persisted from a pre-reorg chain.
            let genesis_blocks = config.network.genesis_blocks();
            let best_height = chain_state.read().await.best_height;
            if genesis_blocks > 0 && best_height > genesis_blocks && set.active_count() > 0 {
                // Check if genesis blocks exist (snap-synced nodes may be missing them)
                let has_genesis_blocks =
                    block_store.get_block_by_height(1).ok().flatten().is_some();
                if !has_genesis_blocks {
                    info!(
                        "[STARTUP] Snap sync detected (genesis blocks missing). \
                         Trusting StateDb producer set ({} producers).",
                        set.active_count()
                    );
                } else {
                    let mut seen = std::collections::HashSet::new();
                    let mut chain_genesis_count = 0usize;
                    for h in 1..=genesis_blocks {
                        if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                            for tx in &block.transactions {
                                if tx.tx_type == TxType::Registration {
                                    if let Some(reg_data) = tx.registration_data() {
                                        if reg_data.vdf_output.len() == 32
                                            && seen.insert(reg_data.public_key)
                                        {
                                            chain_genesis_count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if chain_genesis_count > 0 && chain_genesis_count != set.active_count() {
                        warn!(
                            "[STARTUP] [INTEGRITY_RISK] Genesis producer mismatch: StateDb has {} producers \
                             but block store has {}. Block store may be incomplete (snap sync gap). \
                             If this persists across restarts, wipe data and full-sync from genesis.",
                            set.active_count(),
                            chain_genesis_count
                        );
                    }
                }
            }

            // Reuse the caller's Arc if provided (so UpdateService shares the same reference).
            // We already updated its contents above if StateDb had data.
            if let Some(ref provided) = producer_set {
                // Contents already set (either StateDb version or kept as-is)
                provided.clone()
            } else {
                Arc::new(RwLock::new(set))
            }
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

            // Set block store floor so fork sync knows where our block coverage begins.
            // For snap-synced nodes missing block 1, find the lowest available height.
            // For full-sync nodes this stays at 1 (the default).
            if block_store.get_block_by_height(1).ok().flatten().is_none() {
                // Snap-synced: find lowest available block by scanning from chain state height downward.
                // The snap sync anchor is typically the only indexed block.
                let state = chain_state.read().await;
                let floor = if state.is_snap_synced() {
                    state.snap_sync_height().unwrap_or(state.best_height)
                } else {
                    state.best_height
                };
                sm.set_store_floor(floor);
                info!(
                    "[STARTUP] Block store floor set to {} (snap sync gap — block 1 missing)",
                    floor
                );
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
            network_id,
            genesis_hash,
            gset_path,
        )));

        // Initialize adaptive gossip controller
        let adaptive_gossip = Arc::new(RwLock::new(AdaptiveGossip::new()));

        // Use provided shutdown flag or create a new one
        let shutdown = shutdown_flag.unwrap_or_else(|| Arc::new(RwLock::new(false)));

        Ok(Self {
            config,
            params,
            block_store,
            state_db,
            utxo_set,
            chain_state,
            producer_set,
            mempool,
            network: None,
            sync_manager,
            shutdown,
            producer_key,
            bls_key,
            last_produced_slot: None,
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
            cached_state_root: Arc::new(RwLock::new(None)),
            cached_genesis_producers: std::sync::OnceLock::new(),
            port_check_done: false,
            maintainer_state: None,
            archive_tx: None,
            pending_archive: std::collections::VecDeque::new(),
            archive_dir: None,
            archive_caught_up: false,
            ws_sender: Arc::new(RwLock::new(None)),
            minute_tracker: MinuteAttestationTracker::new(),
            consecutive_forced_recoveries: 0,
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

    /// Set the archive channel and directory (connects apply_block to BlockArchiver)
    pub fn set_archive_tx(&mut self, tx: tokio::sync::mpsc::Sender<ArchiveBlock>, dir: PathBuf) {
        self.archive_tx = Some(tx);
        self.archive_dir = Some(dir);
    }

    /// Get a reference to the block store (for archiver catch-up)
    pub fn block_store(&self) -> &Arc<BlockStore> {
        &self.block_store
    }

    /// Flush pending archive blocks up to the last finalized height.
    /// Only blocks that the protocol has declared irreversible get archived.
    async fn flush_finalized_to_archive(&mut self) {
        let finalized_height = {
            let sync = self.sync_manager.read().await;
            match sync.last_finalized_height() {
                Some(h) => h,
                None => return, // No finality yet
            }
        };

        if let Some(ref tx) = self.archive_tx {
            while let Some(front) = self.pending_archive.front() {
                if front.height > finalized_height {
                    break;
                }
                let block = self.pending_archive.pop_front().unwrap();
                let _ = tx.try_send(block);
            }
        }
    }

    /// Detect if there's a gap in historical blocks (e.g., from snap sync).
    /// Get the current chain tip height
    pub async fn best_height(&self) -> u64 {
        self.chain_state.read().await.best_height
    }

    pub fn set_maintainer_state(&mut self, state: Arc<RwLock<storage::MaintainerState>>) {
        self.maintainer_state = Some(state);
    }

    /// Bootstrap the maintainer set from the first 5 registered producers.
    /// Called once at the epoch boundary where the 5th producer is first available.
    /// After bootstrap, maintainer membership only changes via MaintainerAdd/Remove txs.
    async fn maybe_bootstrap_maintainer_set(&self, height: u64) {
        use doli_core::maintainer::INITIAL_MAINTAINER_COUNT;

        let maintainer_state = match &self.maintainer_state {
            Some(ms) => ms,
            None => return,
        };

        // Already bootstrapped?
        {
            let state = maintainer_state.read().await;
            if state.set.is_fully_bootstrapped() {
                return;
            }
        }

        // Need at least INITIAL_MAINTAINER_COUNT producers to bootstrap
        let producers = self.producer_set.read().await;
        let mut sorted: Vec<_> = producers.all_producers().into_iter().cloned().collect();
        if sorted.len() < INITIAL_MAINTAINER_COUNT {
            return;
        }

        // Take the first 5 by registration height (deterministic)
        sorted.sort_by_key(|p| p.registered_at);
        let bootstrap_keys: Vec<_> = sorted
            .into_iter()
            .take(INITIAL_MAINTAINER_COUNT)
            .map(|p| p.public_key)
            .collect();

        let mut state = maintainer_state.write().await;
        // Double-check under write lock
        if state.set.is_fully_bootstrapped() {
            return;
        }

        let set =
            doli_core::maintainer::MaintainerSet::with_members(bootstrap_keys.clone(), height);
        state.set = set;
        state.last_derived_height = height;

        // Persist to disk
        if let Err(e) = state.save(&self.config.data_dir) {
            warn!("Failed to persist maintainer state: {}", e);
        }

        info!(
            "Bootstrapped maintainer set from first {} producers at height {} (keys: {})",
            INITIAL_MAINTAINER_COUNT,
            height,
            bootstrap_keys
                .iter()
                .map(|k| format!("{}...", &k.to_hex()[..16]))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    /// Run the node
    pub async fn run(&mut self) -> Result<()> {
        info!("Node starting...");

        // Check for placeholder maintainer keys
        if self.config.network == Network::Mainnet && is_using_placeholder_keys(Network::Mainnet) {
            error!("CRITICAL: Placeholder maintainer keys detected!");
            error!("This node is NOT suitable for mainnet operation.");
            error!("Replace BOOTSTRAP_MAINTAINER_KEYS_MAINNET in doli-updater/src/lib.rs with real Ed25519 keys.");
            return Err(anyhow::anyhow!(
                "Cannot start mainnet node with placeholder maintainer keys"
            ));
        } else if is_using_placeholder_keys(self.config.network) {
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

                let gh = self.chain_state.read().await.genesis_hash;
                let announcement =
                    ProducerAnnouncement::new(key, self.config.network.id(), start_seq, gh);

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

    /// Return the canonical genesis hash from the embedded chainspec.
    /// Always correct regardless of state_db corruption.
    fn canonical_genesis_hash(&self) -> Hash {
        let spec = match self.config.network {
            Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
            Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
            Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
        };
        spec.genesis_hash()
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
        // Store node_key in parent of data_dir so chain resets (which wipe data_dir)
        // don't regenerate the peer ID. Stable peer IDs prevent Kademlia mismatch storms.
        // Falls back to data_dir if parent doesn't exist (e.g., data_dir is root-level).
        let node_key_dir = self
            .config
            .data_dir
            .parent()
            .filter(|p| p.exists())
            .unwrap_or(&self.config.data_dir);
        network_config.node_key_path = Some(node_key_dir.join("node_key"));
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

        // Wire up peer info so getNetworkInfo reports real values
        if let Some(ref network) = self.network {
            let peers = network.peers_arc();
            let peers_for_list = peers.clone();
            context = context
                .with_peer_id(network.local_peer_id().to_string())
                .with_peer_count(move || peers.try_read().map(|p| p.len()).unwrap_or(0))
                .with_peer_list(move || {
                    peers_for_list
                        .try_read()
                        .map(|p| {
                            p.values()
                                .map(|info| rpc::PeerInfoEntry {
                                    peer_id: info.id.clone(),
                                    address: info.address.clone(),
                                    best_height: info.best_height,
                                    connected_secs: info.connected_at.elapsed().as_secs(),
                                    last_seen_secs: info.last_seen.elapsed().as_secs(),
                                    latency_ms: info.latency.map(|d| d.as_millis() as u64),
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                });
        }

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
        // AND processed locally (gossip doesn't echo back to publisher)
        if let Some(ref network) = self.network {
            let cmd_tx = network.command_sender();
            let local_vote_tx = self.vote_tx.clone();
            context = context.with_broadcast_vote(move |vote_data: Vec<u8>| {
                let cmd_tx = cmd_tx.clone();
                let local_tx = local_vote_tx.clone();
                tokio::spawn(async move {
                    // Broadcast to network
                    let _ = cmd_tx
                        .send(NetworkCommand::BroadcastVote(vote_data.clone()))
                        .await;
                    // Also deliver to local update service (gossip won't echo back)
                    if let Some(tx) = local_tx {
                        if let Ok(vote_msg) =
                            serde_json::from_slice::<node_updater::VoteMessage>(&vote_data)
                        {
                            let _ = tx.send(vote_msg).await;
                        }
                    }
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

        // Wire up on-chain maintainer set for getMaintainerSet RPC
        if let Some(ref ms) = self.maintainer_state {
            context.maintainer_state = Some(ms.clone());
        }

        let (ws_tx, _ws_rx) = rpc::ws::broadcast_channel();
        *self.ws_sender.write().await = Some(ws_tx.clone());

        let server = RpcServer::new(rpc_config, context, ws_tx);
        info!("Starting RPC server on {} (WebSocket at /ws)", listen_addr);
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
        // Reuse same sort order as compute_tier1_set (weight desc, pubkey asc)
        let mut all_sorted = producers_with_weights.clone();
        all_sorted.sort_unstable_by(|a, b| {
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
    ///
    /// Adds attestation weight for the finality gadget.
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

        let attestation = if let Some(ref bls_kp) = self.bls_key {
            Attestation::new_with_bls(
                block_hash,
                slot,
                height,
                weight,
                &private_key,
                public_key,
                bls_kp,
            )
        } else {
            Attestation::new(block_hash, slot, height, weight, &private_key, public_key)
        };

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
                        if let Err(e) = self.handle_network_event(event).await {
                            warn!("Error handling network event: {}", e);
                        }
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
                                let gh = self.chain_state.read().await.genesis_hash;
                                let announcement = ProducerAnnouncement::new(key, self.config.network.id(), seq, gh);

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

                // Rate-limited reconnect: delegate to the periodic redial in
                // run_periodic_tasks() (every slot_duration ≈ 10s).  Only do an
                // immediate dial if we haven't tried recently — this prevents a
                // spin loop where rapid connect/disconnect floods the event queue
                // and starves the production timer via the biased select!.
                let peer_count = self.sync_manager.read().await.peer_count();
                if peer_count == 0 && !self.config.bootstrap_nodes.is_empty() {
                    let recently_dialed = self
                        .last_peer_redial
                        .map(|t| t.elapsed().as_secs() < self.params.slot_duration)
                        .unwrap_or(false);
                    if !recently_dialed {
                        info!("Lost all peers — reconnecting to bootstrap nodes");
                        self.last_peer_redial = Some(std::time::Instant::now());
                        if let Some(ref network) = self.network {
                            for addr in &self.config.bootstrap_nodes {
                                if let Err(e) = network.connect(addr).await {
                                    warn!("Failed to reconnect to bootstrap {}: {}", addr, e);
                                }
                            }
                        }
                    }
                }
            }

            NetworkEvent::NewBlock(block, source_peer) => {
                // Skip gossip blocks during snap sync — they cause spurious fork
                // recovery that corrupts state mid-download.
                if self.sync_manager.read().await.state().is_snap_syncing() {
                    debug!("Ignoring gossip block {} during snap sync", block.hash());
                    return Ok(());
                }

                debug!("Received new block: {} from {}", block.hash(), source_peer);

                // DEFENSE: Slot sanity — reject gossip blocks with wildly wrong slots.
                // Prevents genesis-time-hijack attacks where a node compiled with a
                // different GENESIS_TIME produces blocks with desfasados slots.
                // Only applies to GOSSIP blocks — sync blocks (header-first download)
                // bypass this via apply_block() directly, so initial sync is unaffected.
                {
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let current_slot = self.params.timestamp_to_slot(now_secs) as u64;
                    let block_slot = block.header.slot as u64;

                    if block_slot > current_slot + consensus::MAX_FUTURE_SLOTS {
                        warn!(
                            "SLOT_SANITY: Rejecting gossip block {} — slot {} too far in future (current={}, limit=+{})",
                            block.hash(), block_slot, current_slot, consensus::MAX_FUTURE_SLOTS
                        );
                        return Ok(());
                    }
                    if current_slot > block_slot + consensus::MAX_PAST_SLOTS {
                        warn!(
                            "SLOT_SANITY: Rejecting gossip block {} — slot {} too far in past (current={}, limit=-{})",
                            block.hash(), block_slot, current_slot, consensus::MAX_PAST_SLOTS
                        );
                        return Ok(());
                    }
                }

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
                // Decode and apply attestation for finality + liveness tracking
                if let Some(attestation) = doli_core::Attestation::from_bytes(&data) {
                    if attestation.verify().is_ok() {
                        // Finality gadget: accumulate weight per block
                        let mut sync = self.sync_manager.write().await;
                        sync.add_attestation_weight(
                            &attestation.block_hash,
                            attestation.attester_weight,
                        );
                        drop(sync);

                        // Minute tracker: record for on-chain bitfield + BLS aggregation
                        let minute = attestation_minute(attestation.slot);
                        if attestation.bls_signature.is_empty() {
                            self.minute_tracker.record(attestation.attester, minute);
                        } else {
                            self.minute_tracker.record_with_bls(
                                attestation.attester,
                                minute,
                                attestation.bls_signature.clone(),
                            );
                        }

                        // Flush any blocks that just reached finality
                        self.flush_finalized_to_archive().await;
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
        self.apply_block(block, ValidationMode::Full).await?;

        // A canonical gossip block was applied on our tip — clear the post-snap gate.
        // This proves we're on the canonical chain and our block store has a real parent.
        self.sync_manager
            .write()
            .await
            .clear_awaiting_canonical_block();

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

        // No-op reorg: rollback_count=0 means we're already at the common ancestor.
        // Skip the rollback path entirely — there's nothing to undo, and calling
        // get_undo(target_height + 1) would panic because that undo doesn't exist.
        if rollback_count > 0 {
            // Invalidate genesis producer cache if reorg crosses genesis boundary
            let genesis_blocks = self.config.network.genesis_blocks();
            if genesis_blocks > 0 && target_height <= genesis_blocks {
                info!("[REORG] Crossing genesis boundary — invalidating genesis producer cache");
                self.cached_genesis_producers = std::sync::OnceLock::new();
            }

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

            // Undo-based rollback: apply undo data in reverse from current_height to target_height+1.
            // This is O(rollback_depth) instead of O(chain_height).
            // Fallback: if undo data is missing (pre-undo blocks), use legacy rebuild.
            let has_undo =
                (target_height + 1..=current_height).all(|h| self.state_db.get_undo(h).is_some());

            if has_undo {
                info!(
                    "Undo-based rollback: reverting {} blocks ({} → {})",
                    rollback_count, current_height, target_height
                );

                {
                    let mut utxo = self.utxo_set.write().await;

                    // Apply undo data in reverse order (highest block first)
                    for h in (target_height + 1..=current_height).rev() {
                        let undo = self.state_db.get_undo(h).unwrap();

                        // Remove UTXOs created by this block
                        for outpoint in &undo.created_utxos {
                            utxo.remove(outpoint);
                        }

                        // Restore UTXOs spent by this block
                        for (outpoint, entry) in &undo.spent_utxos {
                            utxo.insert(*outpoint, entry.clone());
                        }
                    }

                    // Restore ProducerSet from the undo snapshot at target_height + 1
                    // (which captured the state BEFORE that block was applied = state AT target_height)
                    let first_undo = self.state_db.get_undo(target_height + 1).unwrap();
                    if let Ok(restored_producers) =
                        bincode::deserialize::<storage::ProducerSet>(&first_undo.producer_snapshot)
                    {
                        let mut producers = self.producer_set.write().await;
                        *producers = restored_producers;
                    } else {
                        warn!("Failed to deserialize producer snapshot from undo data, rebuilding from blocks");
                        let mut producers = self.producer_set.write().await;
                        self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
                    }
                }

                // Update chain state
                {
                    let mut state = self.chain_state.write().await;
                    state.best_height = target_height;
                    state.best_hash = common_ancestor_hash;
                    state.best_slot = common_ancestor_slot;
                }
            } else {
                // Legacy fallback: rebuild from genesis (no undo data available)
                warn!(
                "Undo data missing for rollback range {}..={} — falling back to rebuild from genesis",
                target_height + 1,
                current_height
            );

                let genesis_blocks = self.config.network.genesis_blocks();
                let genesis_producers = if genesis_blocks > 0 && target_height > genesis_blocks {
                    self.derive_genesis_producers_from_chain()
                } else {
                    Vec::new()
                };
                let bond_unit = self.config.network.bond_unit();

                {
                    let mut state = self.chain_state.write().await;
                    let mut utxo = self.utxo_set.write().await;
                    state.best_height = target_height;
                    state.best_hash = common_ancestor_hash;
                    state.best_slot = common_ancestor_slot;
                    utxo.clear();
                    for height in 1..=target_height {
                        if let Some(block) =
                            self.block_store.get_block_by_height(height).ok().flatten()
                        {
                            for (tx_index, tx) in block.transactions.iter().enumerate() {
                                let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                                if !is_reward_tx {
                                    let _ = utxo.spend_transaction(tx);
                                }
                                utxo.add_transaction(tx, height, is_reward_tx, block.header.slot);
                            }
                        }
                        if genesis_blocks > 0 && height == genesis_blocks + 1 {
                            Self::consume_genesis_bond_utxos(
                                &mut utxo,
                                &genesis_producers,
                                bond_unit,
                                height,
                            );
                        }
                    }
                }

                {
                    let mut producers = self.producer_set.write().await;
                    self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
                }
            }

            // Force scheduler rebuild after producer set reconstruction
            self.cached_scheduler = None;

            // Atomically persist common ancestor state to StateDb
            {
                let state = self.chain_state.read().await;
                let utxo = self.utxo_set.read().await;
                let producers = self.producer_set.read().await;
                let utxo_pairs: Vec<_> = match &*utxo {
                    UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
                    UtxoSet::RocksDb(_) => self.state_db.iter_utxos(),
                };
                self.state_db
                    .atomic_replace(&state, &producers, utxo_pairs.into_iter())
                    .map_err(|e| anyhow::anyhow!("Reorg StateDb atomic_replace failed: {}", e))?;
            }
        } // end if rollback_count > 0

        // Now apply the new blocks through normal path
        // Note: we skip check_producer_eligibility here because the fork blocks were
        // validated when originally produced, and re-validating against rolled-back
        // state uses the wrong producer set (common ancestor, not fork chain).
        info!("Applying {} new blocks from fork", new_blocks.len());
        for block in new_blocks {
            if let Err(e) = self.apply_block(block, ValidationMode::Light).await {
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

        // Safety limit: reject forks deeper than MAX_SAFE_REORG_DEPTH blocks.
        // Undo data is kept for UNDO_KEEP_DEPTH (2000) blocks, so reorgs up to 500
        // are safe with a 4x margin. Previous limit of 10 caused infinite loops when
        // fork_sync found valid ancestors after snap sync gaps.
        const MAX_SAFE_REORG_DEPTH: u64 = 500;
        if rollback_depth > MAX_SAFE_REORG_DEPTH {
            warn!(
                "Fork sync: reorg depth {} exceeds safety limit ({} blocks). \
                 Refusing automatic reorg (ancestor h={}, current h={}). \
                 Investigate manually — this is likely a snap sync gap or isolation issue, \
                 not a legitimate fork.",
                rollback_depth, MAX_SAFE_REORG_DEPTH, result.ancestor_height, current_height
            );
            {
                let mut sync = self.sync_manager.write().await;
                sync.reset_sync_for_rollback();
            }
            // Do NOT auto-recover. The operator must investigate.
            return Ok(());
        }

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
            let mut sync = self.sync_manager.write().await;
            sync.reset_sync_for_rollback();
            return Ok(());
        }

        // Guard: don't reorg to a shorter chain — this causes cascading height loss
        let new_chain_height = result.ancestor_height + result.canonical_blocks.len() as u64;
        if new_chain_height < current_height {
            warn!(
                "Fork sync: new chain ({}) shorter than current ({}) — aborting reorg",
                new_chain_height, current_height
            );
            let mut sync = self.sync_manager.write().await;
            sync.reset_sync_for_rollback();
            return Ok(());
        }

        // Pre-check: verify block store has blocks from height 1 (required for UTXO rebuild).
        // Snap-synced nodes don't have early blocks — reorg would fail. Re-snap to recover.
        if self.block_store.get_block_by_height(1)?.is_none() {
            warn!(
                "Fork sync reorg: snap sync gap detected (block 1 missing). \
                 Re-snapping to get back to tip."
            );
            {
                let mut sync = self.sync_manager.write().await;
                sync.reset_sync_for_rollback();
            }
            self.reset_state_only().await?;
            return Ok(());
        }

        // Build rollback list: our blocks from current_height down to ancestor+1
        let mut rollback = Vec::new();
        for height in (result.ancestor_height + 1..=current_height).rev() {
            if let Some(hash) = self.block_store.get_hash_by_height(height)? {
                rollback.push(hash);
            }
        }

        // Compute weight of new canonical blocks vs rolled-back blocks
        let new_chain_weight: i64 = {
            let producers = self.producer_set.read().await;
            result
                .canonical_blocks
                .iter()
                .map(|b| {
                    producers
                        .get_by_pubkey(&b.header.producer)
                        .map(|p| p.effective_weight(current_height) as i64)
                        .unwrap_or(1)
                })
                .sum()
        };
        let old_chain_weight: i64 = {
            let producers = self.producer_set.read().await;
            let mut weight = 0i64;
            for height in (result.ancestor_height + 1)..=current_height {
                if let Some(block) = self.block_store.get_block_by_height(height)? {
                    weight += producers
                        .get_by_pubkey(&block.header.producer)
                        .map(|p| p.effective_weight(current_height) as i64)
                        .unwrap_or(1);
                }
            }
            weight
        };
        let weight_delta = new_chain_weight - old_chain_weight;

        if weight_delta <= 0 {
            info!(
                "Fork sync: new chain not heavier (delta={}, new={}, old={}) — keeping current",
                weight_delta, new_chain_weight, old_chain_weight
            );
            let mut sync = self.sync_manager.write().await;
            sync.reset_sync_for_rollback();
            return Ok(());
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
            weight_delta,
        };

        // Execute the reorg using the existing atomic reorg path.
        // IMPORTANT: Always reset sync state afterward, even on failure, to prevent
        // infinite fork sync retry loops.
        match self.execute_reorg(reorg_result, trigger_block).await {
            Ok(()) => {
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

                self.shallow_rollback_count = 0;
                self.consecutive_fork_blocks = 0;

                info!(
                    "Fork sync reorg complete: now at height {}",
                    self.chain_state.read().await.best_height
                );

                Ok(())
            }
            Err(e) => {
                // Always reset sync state to prevent infinite retry loop
                {
                    let mut sync = self.sync_manager.write().await;
                    sync.reset_sync_for_rollback();
                }

                let err_msg = e.to_string();
                if err_msg.contains("missing block") || err_msg.contains("UTXO rebuild") {
                    if self.block_store.get_block_by_height(1)?.is_none() {
                        warn!(
                            "Fork sync reorg failed due to snap sync gap: {}. \
                             Re-snapping to recover.",
                            e
                        );
                        self.reset_state_only().await?;
                    } else {
                        warn!(
                            "Reorg failed with block store intact — possible corruption: {}",
                            e
                        );
                        self.force_recover_from_peers().await?;
                    }
                    return Ok(());
                }

                Err(e)
            }
        }
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
                    self.apply_block(block, ValidationMode::Full).await?;
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
    /// Triggers `recover_from_peers()` after N consecutive fork-blocked
    /// slots, respecting cooldown to prevent resync loops.
    async fn maybe_auto_resync(&mut self, _current_slot: u32) {
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

        // Fix 2: Hard cap on consecutive resyncs — stop the loop after N failures.
        // Exponential backoff alone is insufficient; after 5 failed resyncs the node
        // needs manual intervention (recover --yes or wipe + resync).
        if consecutive_resyncs >= network::MAX_CONSECUTIVE_RESYNCS {
            warn!(
                "Fork recovery: reached max consecutive resyncs ({}) — manual intervention required. \
                 Run `doli-node recover --yes` or wipe and resync.",
                consecutive_resyncs
            );
            return;
        }

        // Fix 3: Don't interrupt active sync progress. If blocks are being applied,
        // the node is making forward progress and should not be reset to genesis.
        // This prevents the cycle: force_resync → sync to h=720 → fork_blocked → resync → h=0.
        let blocks_applied = {
            let sync = self.sync_manager.read().await;
            sync.blocks_applied()
        };
        if blocks_applied > 0 {
            debug!(
                "Fork recovery: sync in progress ({} blocks applied), skipping resync",
                blocks_applied
            );
            self.consecutive_fork_blocks = 0;
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

        // All networks: try 1-block rollback first (lightweight).
        // Only escalate to full snap sync if rollback fails.
        // This prevents micro-forks from triggering expensive state wipes.
        warn!(
            "FORK RECOVERY: {} consecutive fork-blocked slots — rolling back 1 block",
            self.consecutive_fork_blocks
        );
        match self.rollback_one_block().await {
            Ok(true) => {
                info!("Fork recovery: 1-block rollback succeeded");
            }
            Ok(false) => {
                warn!("Fork recovery: rollback not possible, recovering from peers");
                if let Err(e) = self.force_recover_from_peers().await {
                    error!("Fork recovery failed: {}", e);
                }
            }
            Err(e) => {
                error!("Fork recovery rollback failed: {}", e);
                if let Err(e2) = self.force_recover_from_peers().await {
                    error!("Fork recovery fallback also failed: {}", e2);
                }
            }
        }
        self.consecutive_fork_blocks = 0;
        self.last_resync_time = Some(Instant::now());
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

        // C3 defense: envelope must match deserialized state
        if new_chain_state.best_hash != snapshot.block_hash
            || new_chain_state.best_height != snapshot.block_height
        {
            error!(
                "[SNAP_SYNC] Envelope/state mismatch: envelope=({}, {:.16}) vs deserialized=({}, {:.16})",
                snapshot.block_height, snapshot.block_hash,
                new_chain_state.best_height, new_chain_state.best_hash,
            );
            self.sync_manager.write().await.snap_fallback_to_normal();
            return Ok(());
        }

        // Step 3: Replace local state (preserve genesis_hash from our chain)
        let genesis_hash = self.chain_state.read().await.genesis_hash;
        {
            let mut cs = self.chain_state.write().await;
            *cs = new_chain_state;
            cs.genesis_hash = genesis_hash; // Preserve our genesis identity
            cs.mark_snap_synced(snapshot.block_height); // Survives restart: block store empty by design

            // Replace in-memory UTXO set (snap sync always deserializes to InMemory)
            let mut utxo = self.utxo_set.write().await;
            *utxo = new_utxo_set;

            let mut ps = self.producer_set.write().await;
            *ps = new_producer_set;

            // Cache state root atomically while all three write locks are held.
            // No TOCTOU race window possible.
            if let Ok(root) = storage::compute_state_root(&cs, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, cs.best_hash, cs.best_height));
            }

            // Atomically persist to StateDb (single WriteBatch — crash-safe)
            let utxo_pairs: Vec<_> = match &*utxo {
                UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
                UtxoSet::RocksDb(_) => self.state_db.iter_utxos(),
            };
            if let Err(e) = self
                .state_db
                .atomic_replace(&cs, &ps, utxo_pairs.into_iter())
            {
                error!("[SNAP_SYNC] StateDb atomic_replace failed: {}", e);
            }

            // Update sync manager local tip while chain_state is still locked
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(cs.best_height, cs.best_hash, cs.best_slot);
        }

        // Step 6: Seed the canonical index so the first post-snap-sync block can
        // call set_canonical_chain without walking into an empty block store.
        self.block_store
            .seed_canonical_index(snapshot.block_hash, snapshot.block_height)?;

        // Step 7: Track snap sync height in-memory for validation mode selection
        self.snap_sync_height = Some(snapshot.block_height);

        // Step 7b: Inform sync manager of block store floor so fork sync
        // won't binary-search below available block data.
        {
            let mut sync = self.sync_manager.write().await;
            sync.set_store_floor(snapshot.block_height);
        }

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
    /// Automatic recovery: reset state and let snap sync restore from peers.
    ///
    /// Guard: skips recovery if the node is already at >90% of the network tip,
    /// since a near-tip node should wait for reorg rather than wipe state.
    ///
    /// Calls `reset_state_only()` (layers 1-9 + index clear) then lets the sync
    /// manager pick up snap sync on the next tick (h=0 + gap > threshold + peers >= 3).
    ///
    /// Block data (headers/bodies) is preserved — only indexes are cleared.
    /// This is the ONLY automatic recovery path; `force_resync_from_genesis()`
    /// (which also wipes block data) is reserved for manual `recover --yes`.
    ///
    /// Force recovery bypassing the 90% guard — used when fork sync or deep
    /// fork detection has already proven the node is on a different chain.
    ///
    /// Rate-limited: no more than 1 forced recovery per epoch (360 blocks ≈ 1 hour).
    /// Prevents cascade loops where snap sync → fork → re-snap repeats unbounded.
    async fn force_recover_from_peers(&mut self) -> Result<()> {
        // Rate limit: exponential backoff between forced recoveries.
        // 60s → 120s → 300s → 600s → 960s (cap at 4 doublings).
        // Old behavior was a flat 3600s cooldown which left nodes stuck for an hour.
        let cooldown_secs = 60u64 * (1u64 << self.consecutive_forced_recoveries.min(4));
        if let Some(last) = self.last_resync_time {
            let elapsed = last.elapsed().as_secs();
            if elapsed < cooldown_secs {
                warn!(
                    "Recovery: cooldown active — last forced recovery {}s ago \
                     (min {}s, attempt #{} between recoveries). Skipping to prevent cascade.",
                    elapsed, cooldown_secs, self.consecutive_forced_recoveries
                );
                return Ok(());
            }
        }
        self.consecutive_forced_recoveries = self.consecutive_forced_recoveries.saturating_add(1);
        self.last_resync_time = Some(std::time::Instant::now());
        self.recover_from_peers_inner(true).await
    }

    async fn recover_from_peers_inner(&mut self, force: bool) -> Result<()> {
        let local_height = self.chain_state.read().await.best_height;
        let best_peer = self.sync_manager.read().await.best_peer_height();

        // Snap-synced nodes missing genesis blocks can never reorg — re-snap to recover
        if self.block_store.get_block_by_height(1)?.is_none() {
            warn!(
                "Recovery: snap sync gap detected (block 1 missing). \
                 Re-snapping to get back to tip (local h={}, peer h={})",
                local_height, best_peer
            );
            self.reset_state_only().await?;
            return Ok(());
        }

        // Guard: don't wipe a node that's close to the tip — let reorg handle it.
        // Bypassed when force=true (fork sync proved we're on a different chain).
        if !force && best_peer > 0 && local_height > best_peer * 90 / 100 {
            warn!(
                "Recovery: at {}% of network tip (h={}/{}), skipping state wipe — waiting for reorg",
                local_height * 100 / best_peer,
                local_height,
                best_peer
            );
            return Ok(());
        }

        warn!(
            "Recovery{}: resetting state (preserving block data), snap sync will restore (local h={}, peer h={})",
            if force { " (forced)" } else { "" },
            local_height, best_peer
        );

        self.reset_state_only().await?;

        info!(
            "State reset complete (height 0). Block data preserved. \
             Production blocked until sync completes + grace period."
        );

        Ok(())
    }

    /// Reset all mutable state to genesis WITHOUT wiping block data.
    ///
    /// Layers 1-9: sync manager, chain state, UTXO, producers, StateDb, caches.
    /// Block store: only indexes cleared (height_index, slot_index, hash_to_height).
    /// Block data (headers, bodies, presence) preserved for future rollbacks.
    ///
    /// After this, snap sync activates on next tick: h=0 + gap > threshold + peers >= 3.
    async fn reset_state_only(&mut self) -> Result<()> {
        // Use canonical chainspec genesis hash, not state_db (may be corrupt).
        let genesis_hash = self.canonical_genesis_hash();

        // LAYER 1: Reset sync manager (blocks production via ProductionGate)
        {
            let mut sync = self.sync_manager.write().await;
            sync.reset_local_state(genesis_hash);
        }

        // LAYER 2: Reset chain state to genesis
        {
            let mut state = self.chain_state.write().await;
            state.best_height = 0;
            state.best_hash = genesis_hash;
            state.best_slot = 0;
        }

        // LAYER 3: Clear UTXO set (in-memory)
        {
            let mut utxo = self.utxo_set.write().await;
            utxo.clear();
        }

        // LAYER 4: Clear producer set (forces true bootstrap mode)
        {
            let mut producers = self.producer_set.write().await;
            producers.clear();
        }

        // LAYER 4.5: Atomic clear of StateDb (UTXOs + producers + chain state)
        {
            let genesis_cs = ChainState::new(genesis_hash);
            self.state_db.clear_and_write_genesis(&genesis_cs);
        }

        // LAYER 5: Clear known producers list
        {
            let mut known = self.known_producers.write().await;
            known.clear();
        }

        // LAYER 6: Clear fork block cache
        {
            let mut cache = self.fork_block_cache.write().await;
            cache.clear();
        }

        // LAYER 7: Clear equivocation detector
        {
            let mut detector = self.equivocation_detector.write().await;
            detector.clear();
        }

        // LAYER 8: Clear producer discovery CRDT
        {
            let mut gset = self.producer_gset.write().await;
            gset.clear();
        }

        // LAYER 9: Reset production timing state
        self.last_produced_slot = None;
        self.first_peer_connected = None;
        self.last_producer_list_change = None;

        // LAYER 9.5: Clear block store INDEXES only (not block data).
        // Stale indexes from fork blocks would pollute get_last_rewarded_epoch()
        // and other height-based queries. Block data stays for future rollbacks.
        if let Err(e) = self.block_store.clear_indexes() {
            error!("Failed to clear block store indexes during recovery: {}", e);
        }

        Ok(())
    }

    /// Nuclear reset: wipe ALL state INCLUDING block data.
    ///
    /// This is `reset_state_only()` + full block store clear.
    /// Reserved for manual CLI `recover --yes` — NEVER called automatically.
    #[allow(dead_code)]
    async fn force_resync_from_genesis(&mut self) -> Result<()> {
        warn!("Force resync initiated - performing COMPLETE state reset to genesis (including block data)");

        self.reset_state_only().await?;

        // LAYER 10: Clear block store entirely (headers, bodies, indexes)
        if let Err(e) = self.block_store.clear() {
            error!("Failed to clear block store during resync: {}", e);
        }

        info!(
            "Complete state reset to genesis (height 0, block store wiped). \
             Production blocked until sync completes + grace period."
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

        // Build weighted producer list (bond counts derived from UTXO set)
        let producers = self.producer_set.read().await;
        let active: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        drop(producers);

        let utxo = self.utxo_set.read().await;
        let weighted: Vec<(PublicKey, u64)> = active
            .into_iter()
            .map(|pk| {
                let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes());
                let count = utxo
                    .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                    .max(1) as u64;
                (pk, count)
            })
            .collect();
        drop(utxo);

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

        // Build weighted producer list (bond counts from UTXO set)
        let producers = self.producer_set.read().await;
        let active: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        let pending_keys = producers.pending_registration_keys();
        drop(producers);

        let utxo = self.utxo_set.read().await;
        let weighted: Vec<(PublicKey, u64)> = active
            .into_iter()
            .map(|pk| {
                let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes());
                let count = utxo
                    .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                    .max(1) as u64;
                (pk, count)
            })
            .collect();
        drop(utxo);

        // Build bootstrap producer list for validation.
        //
        // For Light mode (sync): the GSet reflects CURRENT network state
        // (includes producers that joined after genesis, e.g. N6/N8), but
        // historical blocks were produced with a DIFFERENT GSet composition.
        // bootstrap_fallback_order uses (slot + rank) % n — a different n
        // means completely different rank assignments → "invalid producer
        // for slot". Pass empty bootstrap_producers for ALL synced blocks:
        // - Genesis-phase blocks: accepted via empty-bootstrap-list fallback
        // - Transition block (361): same — producer_set not yet populated
        // - Post-genesis blocks: validated by deterministic bond-weighted
        //   scheduler (on-chain data), bypassing bootstrap path entirely
        // This is safe: header chain continuity is verified during header
        // download, and blocks were already validated by the network.
        let (bootstrap_producers, live_bp, stale_bp) = if mode == ValidationMode::Light {
            (Vec::new(), Vec::new(), Vec::new())
        } else {
            let mut bp = {
                let gset = self.producer_gset.read().await;
                gset.active_producers(7200)
            };
            if bp.is_empty() {
                let known = self.known_producers.read().await;
                bp = known.clone();
            }

            // ACTIVATION_DELAY filter: mirror the production code's filtering
            // (node.rs try_produce_block lines 4993-5014). Without this, the
            // validation path may compute a different producer count N than
            // production, causing slot % N mismatches → "invalid producer for slot".
            {
                let producers = self.producer_set.read().await;
                bp.retain(|pk| match producers.get_by_pubkey(pk) {
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
                });
            }

            bp.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

            // Build liveness split
            let num_bp = bp.len();
            let liveness_window = std::cmp::max(
                consensus::LIVENESS_WINDOW_MIN,
                (num_bp as u64).saturating_mul(3),
            );
            let chain_height = height.saturating_sub(1);
            let cutoff = chain_height.saturating_sub(liveness_window);
            let (live, stale): (Vec<PublicKey>, Vec<PublicKey>) = {
                let (l, s): (Vec<_>, Vec<_>) =
                    bp.iter()
                        .partition(|pk| match self.producer_liveness.get(pk) {
                            Some(&last_h) => last_h >= cutoff,
                            None => chain_height < liveness_window,
                        });
                (
                    l.into_iter().copied().collect(),
                    s.into_iter().copied().collect(),
                )
            };
            if live.is_empty() {
                (bp.clone(), bp, Vec::new())
            } else {
                (bp, live, stale)
            }
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
        .with_pending_producer_keys(pending_keys)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp);

        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        drop(state);

        validation::validate_block_with_mode(block, &ctx, mode)
    }

    /// Validate block economics — prevents inflation and reward theft.
    ///
    /// Checks that cannot be done in the core validation crate because they
    /// require access to the UTXO set, producer registry, and block store.
    ///
    /// ## Coinbase validation (every block)
    /// - First TX must be coinbase (Transfer, no inputs, 1 output)
    /// - Amount must equal `block_reward(height)`
    /// - Recipient must be `reward_pool_pubkey_hash()`
    ///
    /// ## EpochReward validation (epoch boundary blocks)
    /// - EpochReward TX only allowed at epoch boundaries, post-genesis, epoch > 0
    /// - At most one EpochReward TX per block
    /// - Total distributed must not exceed pool balance (conservation)
    /// - (Full mode) Exact match of amounts and recipients
    async fn validate_block_economics(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<()> {
        // === Coinbase validation ===
        if block.transactions.is_empty() {
            anyhow::bail!("block has no transactions (missing coinbase)");
        }

        let coinbase = &block.transactions[0];
        if !coinbase.is_coinbase() {
            anyhow::bail!("first transaction is not a valid coinbase");
        }

        let expected_reward = self.params.block_reward(height);
        if coinbase.outputs[0].amount != expected_reward {
            anyhow::bail!(
                "coinbase amount {} != expected block reward {}",
                coinbase.outputs[0].amount,
                expected_reward
            );
        }

        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        if coinbase.outputs[0].pubkey_hash != pool_hash {
            anyhow::bail!("coinbase recipient is not the reward pool — possible theft attempt");
        }

        // === EpochReward validation ===
        let epoch_reward_txs: Vec<&Transaction> = block
            .transactions
            .iter()
            .filter(|tx| tx.tx_type == TxType::EpochReward)
            .collect();

        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let is_epoch_boundary = height > 0
            && !self.config.network.is_in_genesis(height)
            && reward_epoch::is_epoch_start_with(height, blocks_per_epoch);

        if !epoch_reward_txs.is_empty() {
            // EpochReward only allowed at epoch boundaries, post-genesis
            if !is_epoch_boundary {
                anyhow::bail!(
                    "EpochReward transaction at non-epoch-boundary height {}",
                    height
                );
            }

            let completed_epoch = (height / blocks_per_epoch) - 1;

            // No EpochReward at epoch 0 (genesis bonds drained the pool)
            if completed_epoch == 0 {
                anyhow::bail!("EpochReward not allowed at epoch 0 (genesis pool used for bonds)");
            }

            // Exactly one EpochReward TX per block
            if epoch_reward_txs.len() != 1 {
                anyhow::bail!(
                    "expected at most 1 EpochReward TX, got {}",
                    epoch_reward_txs.len()
                );
            }
            let epoch_tx = epoch_reward_txs[0];

            // Validate extra_data contains correct height + epoch
            if epoch_tx.extra_data.len() >= 16 {
                let embedded_height =
                    u64::from_le_bytes(epoch_tx.extra_data[0..8].try_into().unwrap());
                let embedded_epoch =
                    u64::from_le_bytes(epoch_tx.extra_data[8..16].try_into().unwrap());
                if embedded_height != height {
                    anyhow::bail!(
                        "EpochReward embedded height {} != block height {}",
                        embedded_height,
                        height
                    );
                }
                if embedded_epoch != completed_epoch {
                    anyhow::bail!(
                        "EpochReward embedded epoch {} != completed epoch {}",
                        embedded_epoch,
                        completed_epoch
                    );
                }
            }

            // Conservation: total distributed must not exceed pool balance
            let total_distributed: u64 = epoch_tx.outputs.iter().map(|o| o.amount).sum();
            let pool_balance = {
                let utxo = self.utxo_set.read().await;
                let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                let utxo_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                // Include current block's coinbase (not yet in UTXO set)
                utxo_total + self.params.block_reward(height)
            };

            if total_distributed > pool_balance {
                anyhow::bail!(
                    "EpochReward total {} exceeds pool balance {} — inflation attack",
                    total_distributed,
                    pool_balance
                );
            }

            // Full mode: exact match of amounts and recipients
            if mode == ValidationMode::Full {
                let expected = self.calculate_epoch_rewards(completed_epoch).await;

                let mut expected_sorted: Vec<(u64, crypto::Hash)> = expected;
                expected_sorted.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

                let mut actual_sorted: Vec<(u64, crypto::Hash)> = epoch_tx
                    .outputs
                    .iter()
                    .map(|o| (o.amount, o.pubkey_hash))
                    .collect();
                actual_sorted.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

                if expected_sorted != actual_sorted {
                    let expected_total: u64 = expected_sorted.iter().map(|(a, _)| *a).sum();
                    anyhow::bail!(
                        "EpochReward distribution mismatch: expected {} outputs totaling {}, \
                         got {} outputs totaling {} — possible reward theft",
                        expected_sorted.len(),
                        expected_total,
                        actual_sorted.len(),
                        total_distributed
                    );
                }
            }
        }

        Ok(())
    }

    /// Apply a block to the chain
    ///
    /// `mode`: `Full` for gossip/production (checks MAX_PAST_SLOTS, VDF),
    ///         `Light` for sync/reorg (skips time-based checks that reject old blocks).
    async fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()> {
        let block_hash = block.hash();

        let height = self.chain_state.read().await.best_height + 1;

        // Defense-in-depth: skip blocks we already have AND have applied.
        // Prevents height double-counting if sync delivers stored blocks (ISSUE-5).
        //
        // Critical: We must check that the block was actually APPLIED (chain state
        // advanced past it), not just stored. A block can be in the store but not
        // applied if a previous apply_block() wrote the block then failed during
        // UTXO validation (block store poisoning — see N4 incident 2026-03-13).
        // In that case, we allow re-apply — put_block() will overwrite harmlessly.
        if self.block_store.has_block(&block_hash)? {
            if let Some(stored_height) = self.block_store.get_height_by_hash(&block_hash)? {
                if stored_height < height {
                    // Block is in store but chain state didn't advance past it.
                    // This is a poisoned block from a failed apply — proceed with
                    // re-validation and re-apply (put_block overwrites safely).
                    warn!(
                        "Block {} in store at height {} but chain at height {} — \
                         poisoned block, re-applying",
                        block_hash,
                        stored_height,
                        height - 1
                    );
                } else {
                    // Block is in store AND chain state is at or past it — truly applied.
                    debug!(
                        "Block {} already applied at height {}, skipping",
                        block_hash, stored_height
                    );
                    return Ok(());
                }
            } else {
                // Block in store but no height index — proceed with apply.
                warn!(
                    "Block {} in store but no height index — re-applying",
                    block_hash
                );
            }
        }

        self.validate_block_for_apply(&block, height, mode).await?;
        self.validate_block_economics(&block, height, mode).await?;

        info!("Applying block {} at height {}", block_hash, height);

        // NOTE: Block store write is deferred to AFTER all transaction validation.
        // Writing here would poison the store if UTXO validation fails later —
        // the "already in store" check would then skip the block forever.
        // See: N4 stuck-at-22888 incident (2026-03-13).

        // Update producer liveness tracker (for scheduling filter)
        self.producer_liveness.insert(block.header.producer, height);

        // Track newly registered producers to add to known_producers after lock release
        let mut new_registrations: Vec<PublicKey> = Vec::new();

        // Deferred protocol activation (verified in tx loop, applied when chain_state lock acquired)
        let mut pending_protocol_activation_data: Option<(u32, u64)> = None;

        // Create BlockBatch for atomic persistence (all state changes in one WriteBatch)
        let mut batch = self.state_db.begin_batch();

        // Track dirty producers for efficient batch writes
        let mut dirty_producer_keys: std::collections::HashSet<Hash> =
            std::collections::HashSet::new();
        let removed_producer_keys: std::collections::HashSet<Hash> =
            std::collections::HashSet::new();
        let dirty_exit_keys: std::collections::HashSet<Hash> = std::collections::HashSet::new();

        // Undo log: snapshot ProducerSet BEFORE any mutations
        let undo_producer_snapshot = {
            let producers = self.producer_set.read().await;
            bincode::serialize(&*producers).unwrap_or_default()
        };

        // Undo log: track UTXO changes for this block
        let mut undo_spent_utxos: Vec<(storage::Outpoint, storage::UtxoEntry)> = Vec::new();
        let mut undo_created_utxos: Vec<storage::Outpoint> = Vec::new();

        // Apply transactions to UTXO set and process special transactions
        {
            let mut utxo = self.utxo_set.write().await;
            let mut producers = self.producer_set.write().await;

            for (tx_index, tx) in block.transactions.iter().enumerate() {
                // Apply UTXO changes (in-memory for reads + batch for atomic persistence)
                let is_reward_tx = tx_index == 0 && tx.is_reward_minting();

                // Undo log: capture UTXOs BEFORE spending them
                if !is_reward_tx {
                    for input in &tx.inputs {
                        let outpoint =
                            storage::Outpoint::new(input.prev_tx_hash, input.output_index);
                        if let Some(entry) = utxo.get(&outpoint) {
                            undo_spent_utxos.push((outpoint, entry));
                        }
                    }
                }

                // EpochReward TX: consume all pool UTXOs (the pool collected per-block coinbase)
                if tx.tx_type == TxType::EpochReward {
                    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
                    let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                    for (outpoint, entry) in &pool_utxos {
                        undo_spent_utxos.push((*outpoint, entry.clone()));
                        utxo.remove(outpoint);
                        let _ = batch.spend_utxo(outpoint);
                    }
                    if !pool_utxos.is_empty() {
                        let total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                        info!(
                            "Consumed {} pool UTXOs ({} units) for epoch reward distribution",
                            pool_utxos.len(),
                            total
                        );
                    }
                }

                // Validate UTXO-level spending conditions (signatures, covenant evaluation,
                // lock times, balance). Coinbase/EpochReward skip internally.
                if !is_reward_tx {
                    let utxo_ctx = validation::ValidationContext::new(
                        ConsensusParams::for_network(self.config.network),
                        self.config.network,
                        0, // wall-clock not needed for UTXO validation
                        height,
                    );
                    validation::validate_transaction_with_utxos(tx, &utxo_ctx, &*utxo).map_err(
                        |e| anyhow::anyhow!("UTXO validation failed for tx {}: {}", tx.hash(), e),
                    )?;
                }

                let _ = utxo.spend_transaction(tx); // In-memory
                utxo.add_transaction(tx, height, is_reward_tx, block.header.slot); // In-memory

                // Undo log: track created UTXOs
                let tx_hash = tx.hash();
                for (index, _) in tx.outputs.iter().enumerate() {
                    undo_created_utxos.push(storage::Outpoint::new(tx_hash, index as u32));
                }

                // Batch: track the same UTXO changes for atomic persistence
                if !is_reward_tx {
                    let _ = batch.spend_transaction_utxos(tx);
                }
                batch.add_transaction_utxos(tx, height, is_reward_tx, block.header.slot);

                // Process registration transactions — deferred to epoch boundary
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
                        // Find all bond outputs and use the first as the primary outpoint
                        let bond_outputs: Vec<(usize, &doli_core::transaction::Output)> = tx
                            .outputs
                            .iter()
                            .enumerate()
                            .filter(|(_, o)| {
                                o.output_type == doli_core::transaction::OutputType::Bond
                            })
                            .collect();

                        if let Some(&(bond_index, _)) = bond_outputs.first() {
                            let tx_hash = tx.hash();
                            let era = self.params.height_to_era(height);
                            let total_bond_amount: u64 =
                                bond_outputs.iter().map(|(_, o)| o.amount).sum();

                            let mut producer_info = storage::ProducerInfo::new_with_bonds(
                                reg_data.public_key,
                                height,
                                total_bond_amount,
                                (tx_hash, bond_index as u32),
                                era,
                                reg_data.bond_count,
                            );
                            producer_info.bls_pubkey = reg_data.bls_pubkey.clone();

                            producers.queue_update(PendingProducerUpdate::Register {
                                info: Box::new(producer_info),
                                height,
                            });
                            info!(
                                "Queued producer registration {} at height {} (deferred to epoch boundary)",
                                crypto_hash(reg_data.public_key.as_bytes()),
                                height
                            );
                            // Track for known_producers update after lock release
                            new_registrations.push(reg_data.public_key);
                        }
                    }
                }

                // Process exit transactions — withdraw ALL bonds (deferred to epoch boundary)
                if tx.tx_type == TxType::Exit {
                    if let Some(exit_data) = tx.exit_data() {
                        if let Some(info) = producers.get_by_pubkey(&exit_data.public_key) {
                            let all_bonds = info.bond_count;
                            if all_bonds > 0 {
                                let bond_unit = self.config.network.bond_unit();
                                // Mark all bonds as pending withdrawal
                                if let Some(producer) =
                                    producers.get_by_pubkey_mut(&exit_data.public_key)
                                {
                                    producer.withdrawal_pending_count += all_bonds;
                                    dirty_producer_keys
                                        .insert(crypto_hash(exit_data.public_key.as_bytes()));
                                }
                                producers.queue_update(PendingProducerUpdate::RequestWithdrawal {
                                    pubkey: exit_data.public_key,
                                    bond_count: all_bonds,
                                    bond_unit,
                                });
                                info!(
                                    "Queued exit (withdraw all {} bonds) for producer {} at height {} (deferred to epoch boundary)",
                                    all_bonds,
                                    crypto_hash(exit_data.public_key.as_bytes()),
                                    height
                                );
                            } else {
                                // No bonds — just mark as exited directly
                                producers.queue_update(PendingProducerUpdate::Exit {
                                    pubkey: exit_data.public_key,
                                    height,
                                });
                            }
                        } else {
                            warn!("Exit: producer not found");
                        }
                    }
                }

                // Process slash transactions — deferred to epoch boundary
                if tx.tx_type == TxType::SlashProducer {
                    if let Some(slash_data) = tx.slash_data() {
                        producers.queue_update(PendingProducerUpdate::Slash {
                            pubkey: slash_data.producer_pubkey,
                            height,
                        });
                        warn!(
                            "Queued SLASH for producer {} at height {} (deferred to epoch boundary)",
                            crypto_hash(slash_data.producer_pubkey.as_bytes()),
                            height
                        );
                    }
                }

                // Process AddBond transactions — deferred to epoch boundary
                if tx.tx_type == TxType::AddBond {
                    if let Some(add_bond_data) = tx.add_bond_data() {
                        // Guard: producer must be registered (or have a pending registration)
                        let pubkey = &add_bond_data.producer_pubkey;
                        let is_registered = producers.get_by_pubkey(pubkey).is_some();
                        let has_pending_reg = producers
                            .pending_updates_for(pubkey)
                            .iter()
                            .any(|u| matches!(u, PendingProducerUpdate::Register { .. }));

                        if !is_registered && !has_pending_reg {
                            warn!(
                                "AddBond ignored: producer {} is not registered (Bond UTXO orphaned at height {})",
                                crypto_hash(pubkey.as_bytes()),
                                height
                            );
                        } else {
                            let tx_hash = tx.hash();
                            let bond_count = add_bond_data.bond_count;

                            // Lock/unlock: Bond output is a real UTXO in the UTXO set.
                            // Find Bond output indices for outpoint references.
                            let bond_outpoints: Vec<(crypto::Hash, u32)> = tx
                                .outputs
                                .iter()
                                .enumerate()
                                .filter(|(_, o)| {
                                    o.output_type == doli_core::transaction::OutputType::Bond
                                })
                                .map(|(i, _)| (tx_hash, i as u32))
                                .collect();

                            let bond_unit = self.config.network.bond_unit();
                            producers.queue_update(PendingProducerUpdate::AddBond {
                                pubkey: add_bond_data.producer_pubkey,
                                outpoints: bond_outpoints,
                                bond_unit,
                                creation_slot: block.header.slot,
                            });
                            info!(
                                "Queued AddBond ({} bonds, lock/unlock) for producer {} at height {} (deferred to epoch boundary)",
                                bond_count,
                                crypto_hash(pubkey.as_bytes()),
                                height
                            );
                        }
                    }
                }

                // Process WithdrawalRequest transactions — deferred to epoch boundary
                if tx.tx_type == TxType::RequestWithdrawal {
                    if let Some(data) = tx.withdrawal_request_data() {
                        // Validate: producer exists and has enough bonds
                        if let Some(info) = producers.get_by_pubkey(&data.producer_pubkey) {
                            let available = info
                                .bond_count
                                .saturating_sub(info.withdrawal_pending_count);
                            if data.bond_count > available {
                                warn!(
                                    "WithdrawalRequest: not enough bonds (requested {}, available {})",
                                    data.bond_count, available
                                );
                            } else {
                                // Validate: output amount <= FIFO net calculation (from UTXO bonds)
                                let pubkey_hash = hash_with_domain(
                                    ADDRESS_DOMAIN,
                                    data.producer_pubkey.as_bytes(),
                                );
                                let bond_entries = utxo.get_bond_entries(&pubkey_hash);
                                let expected = storage::producer::calculate_withdrawal_from_bonds(
                                    &bond_entries,
                                    data.bond_count,
                                    block.header.slot,
                                    self.config.network.params().vesting_quarter_slots,
                                );
                                let tx_output = tx.outputs.first().map(|o| o.amount).unwrap_or(0);
                                if let Some((expected_net, _penalty)) = expected {
                                    if tx_output > expected_net {
                                        warn!(
                                            "WithdrawalRequest: output {} exceeds FIFO net {}",
                                            tx_output, expected_net
                                        );
                                    } else {
                                        // Increment pending count (prevents double-withdrawal in same epoch)
                                        if let Some(producer) =
                                            producers.get_by_pubkey_mut(&data.producer_pubkey)
                                        {
                                            producer.withdrawal_pending_count += data.bond_count;
                                            // Track as dirty for atomic batch write
                                            dirty_producer_keys.insert(crypto_hash(
                                                data.producer_pubkey.as_bytes(),
                                            ));
                                        }

                                        let bond_unit = self.config.network.bond_unit();
                                        producers.queue_update(
                                            PendingProducerUpdate::RequestWithdrawal {
                                                pubkey: data.producer_pubkey,
                                                bond_count: data.bond_count,
                                                bond_unit,
                                            },
                                        );
                                        info!(
                                            "Queued WithdrawalRequest ({} bonds) for producer {} at height {} (deferred to epoch boundary)",
                                            data.bond_count,
                                            crypto_hash(data.producer_pubkey.as_bytes()),
                                            height
                                        );
                                    }
                                } else {
                                    warn!(
                                        "WithdrawalRequest: FIFO calculation failed for producer"
                                    );
                                }
                            }
                        } else {
                            warn!("WithdrawalRequest: producer not found");
                        }
                    }
                }

                // Process DelegateBond transactions — deferred to epoch boundary
                if tx.tx_type == TxType::DelegateBond {
                    if let Some(data) = tx.delegate_bond_data() {
                        producers.queue_update(PendingProducerUpdate::DelegateBond {
                            delegator: data.delegator,
                            delegate: data.delegate,
                            bond_count: data.bond_count,
                        });
                        info!(
                            "Queued DelegateBond ({} bonds) from {} to {} at height {} (deferred to epoch boundary)",
                            data.bond_count,
                            crypto_hash(data.delegator.as_bytes()),
                            crypto_hash(data.delegate.as_bytes()),
                            height
                        );
                    }
                }

                // Process RevokeDelegation transactions — deferred to epoch boundary
                if tx.tx_type == TxType::RevokeDelegation {
                    if let Some(data) = tx.revoke_delegation_data() {
                        producers.queue_update(PendingProducerUpdate::RevokeDelegation {
                            delegator: data.delegator,
                        });
                        info!(
                            "Queued RevokeDelegation for {} at height {} (deferred to epoch boundary)",
                            crypto_hash(data.delegator.as_bytes()),
                            height
                        );
                    }
                }

                // Process MaintainerAdd transactions — applied immediately (governance, not epoch-deferred)
                if tx.tx_type == TxType::AddMaintainer {
                    if let Some(maintainer_state) = &self.maintainer_state {
                        if let Some(data) =
                            doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                        {
                            let mut ms = maintainer_state.write().await;
                            let message = data.signing_message(true);
                            if ms.set.verify_multisig(&data.signatures, &message) {
                                match ms.set.add_maintainer(data.target, height) {
                                    Ok(()) => {
                                        ms.last_derived_height = height;
                                        if let Err(e) = ms.save(&self.config.data_dir) {
                                            warn!("Failed to persist maintainer state: {}", e);
                                        }
                                        info!(
                                            "[MAINTAINER] Added maintainer {} at height {}",
                                            data.target.to_hex(),
                                            height
                                        );
                                    }
                                    Err(e) => warn!("[MAINTAINER] Add failed: {}", e),
                                }
                            } else {
                                warn!(
                                    "[MAINTAINER] Rejected AddMaintainer: insufficient signatures"
                                );
                            }
                        }
                    }
                }

                // Process MaintainerRemove transactions — applied immediately
                if tx.tx_type == TxType::RemoveMaintainer {
                    if let Some(maintainer_state) = &self.maintainer_state {
                        if let Some(data) =
                            doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                        {
                            let mut ms = maintainer_state.write().await;
                            let message = data.signing_message(false);
                            if ms.set.verify_multisig_excluding(
                                &data.signatures,
                                &message,
                                &data.target,
                            ) {
                                match ms.set.remove_maintainer(&data.target, height) {
                                    Ok(()) => {
                                        ms.last_derived_height = height;
                                        if let Err(e) = ms.save(&self.config.data_dir) {
                                            warn!("Failed to persist maintainer state: {}", e);
                                        }
                                        info!(
                                            "[MAINTAINER] Removed maintainer {} at height {}",
                                            data.target.to_hex(),
                                            height
                                        );
                                    }
                                    Err(e) => warn!("[MAINTAINER] Remove failed: {}", e),
                                }
                            } else {
                                warn!("[MAINTAINER] Rejected RemoveMaintainer: insufficient signatures");
                            }
                        }
                    }
                }

                // Process ProtocolActivation transactions — verified against on-chain maintainer set
                if tx.tx_type == TxType::ProtocolActivation {
                    if let Some(data) = tx.protocol_activation_data() {
                        // Use on-chain MaintainerSet if available, fall back to ad-hoc derivation
                        let mset = if let Some(maintainer_state) = &self.maintainer_state {
                            let ms = maintainer_state.read().await;
                            if ms.set.is_fully_bootstrapped() {
                                ms.set.clone()
                            } else {
                                // Not yet bootstrapped — derive ad-hoc
                                let mut sorted = producers.all_producers().to_vec();
                                sorted.sort_by_key(|p| p.registered_at);
                                let keys: Vec<crypto::PublicKey> = sorted
                                    .iter()
                                    .take(doli_core::maintainer::INITIAL_MAINTAINER_COUNT)
                                    .map(|p| p.public_key)
                                    .collect();
                                doli_core::MaintainerSet::with_members(keys, height)
                            }
                        } else {
                            let mut sorted = producers.all_producers().to_vec();
                            sorted.sort_by_key(|p| p.registered_at);
                            let keys: Vec<crypto::PublicKey> = sorted
                                .iter()
                                .take(doli_core::maintainer::INITIAL_MAINTAINER_COUNT)
                                .map(|p| p.public_key)
                                .collect();
                            doli_core::MaintainerSet::with_members(keys, height)
                        };

                        let message = data.signing_message();
                        if mset.verify_multisig(&data.signatures, &message) {
                            pending_protocol_activation_data =
                                Some((data.protocol_version, data.activation_epoch));
                            info!(
                                "[PROTOCOL] Verified activation tx: v{} at epoch {}",
                                data.protocol_version, data.activation_epoch
                            );
                        } else {
                            warn!("[PROTOCOL] Rejected activation: insufficient maintainer signatures");
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

            // Apply deferred protocol activation (verified during tx processing)
            if let Some((version, activation_epoch)) = pending_protocol_activation_data {
                let current_epoch = height / SLOTS_PER_EPOCH as u64;
                if activation_epoch > current_epoch && version > state.active_protocol_version {
                    state.pending_protocol_activation = Some((version, activation_epoch));
                    info!(
                        "[PROTOCOL] Scheduled activation: v{} at epoch {} (current epoch {})",
                        version, activation_epoch, current_epoch
                    );
                } else {
                    warn!(
                        "[PROTOCOL] Rejected activation: v{} at epoch {} (current v{}, epoch {})",
                        version, activation_epoch, state.active_protocol_version, current_epoch
                    );
                }
            }

            // Check pending protocol activation at epoch boundaries
            if doli_core::EpochSnapshot::is_epoch_boundary(height) {
                if let Some((version, activation_epoch)) = state.pending_protocol_activation {
                    let current_epoch = height / SLOTS_PER_EPOCH as u64;
                    if current_epoch >= activation_epoch {
                        state.active_protocol_version = version;
                        state.pending_protocol_activation = None;
                        info!(
                            "[PROTOCOL] Activated protocol version {} at epoch {} (height {})",
                            version, current_epoch, height
                        );
                    }
                }
            }

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

            // Cache state root atomically: chain_state is still write-locked,
            // utxo and producer_set were already updated earlier in apply_block.
            // No TOCTOU race window possible.
            let utxo = self.utxo_set.read().await;
            let ps = self.producer_set.read().await;
            if let Ok(root) = storage::compute_state_root(&state, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, state.best_hash, state.best_height));
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

        // Apply deferred producer updates at epoch boundaries.
        // During epoch 0 (blocks 1-359), apply every block so genesis producers work immediately.
        // After epoch 0, apply only at epoch boundaries (height % 360 == 0).
        let mut needs_full_producer_write = false;
        {
            let is_epoch_0 = height < SLOTS_PER_EPOCH as u64;
            let is_boundary = doli_core::EpochSnapshot::is_epoch_boundary(height);
            if is_epoch_0 || is_boundary {
                let mut producers = self.producer_set.write().await;
                if producers.has_pending_updates() {
                    let count = producers.pending_update_count();
                    producers.apply_pending_updates();
                    self.cached_scheduler = None; // Force scheduler rebuild
                    needs_full_producer_write = true; // Many producers may have changed
                    info!(
                        "Applied {} deferred producer updates at height {} (epoch_0={}, boundary={})",
                        count, height, is_epoch_0, is_boundary
                    );
                }
            }
        }

        // Bootstrap maintainer set once we have enough producers.
        // Must run after apply_pending_updates so newly registered producers are visible.
        self.maybe_bootstrap_maintainer_set(height).await;

        // NOTE: recompute_tier + EpochSnapshot + attestation moved after batch commit
        // to avoid borrow conflict with BlockBatch (which borrows self.state_db).

        // GENESIS END: Derive and register producers with REAL bonds from the blockchain.
        // This runs when applying the FIRST post-genesis block (genesis_blocks + 1).
        // Each genesis producer's coinbase reward UTXOs are consumed to back their bond,
        // creating a proper bond-backed registration (no phantom bonds).
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks > 0 && height == genesis_blocks + 1 {
            info!("=== GENESIS PHASE COMPLETE at height {} ===", height);

            let genesis_producers = self.derive_genesis_producers_from_chain();
            let genesis_bls = self.genesis_bls_pubkeys();
            let producer_count = genesis_producers.len();

            if producer_count > 0 {
                info!(
                    "Derived {} genesis producers from blockchain (blocks 1-{}), {} with BLS keys",
                    producer_count,
                    genesis_blocks,
                    genesis_bls.len()
                );

                let bond_unit = self.config.network.bond_unit();
                let era = self.params.height_to_era(height);
                let mut utxo = self.utxo_set.write().await;
                let mut producers = self.producer_set.write().await;

                // Clear bootstrap phantom producers — replace with real bond-backed ones
                producers.clear();

                // Consume pool UTXOs for genesis bonds.
                // All per-block coinbase went to the reward pool — draw bonds from there.
                let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
                let mut pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                pool_utxos.sort_by(|(a_op, a_entry), (b_op, b_entry)| {
                    a_entry
                        .height
                        .cmp(&b_entry.height)
                        .then_with(|| a_op.tx_hash.cmp(&b_op.tx_hash))
                        .then_with(|| a_op.index.cmp(&b_op.index))
                });

                let total_pool: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                let total_bonds_needed = bond_unit * producer_count as u64;
                info!(
                    "Genesis bond migration: pool={} DOLI, bonds_needed={} DOLI ({} producers × {} bond)",
                    total_pool / 100_000_000,
                    total_bonds_needed / 100_000_000,
                    producer_count,
                    bond_unit / 100_000_000
                );

                if total_pool < total_bonds_needed {
                    warn!(
                        "Pool has insufficient funds for all bonds ({} < {}), some producers may not bond",
                        total_pool, total_bonds_needed
                    );
                }

                // Consume all pool UTXOs (they'll be redistributed as bonds + remainder back to pool)
                let mut pool_consumed: u64 = 0;
                for (outpoint, entry) in &pool_utxos {
                    undo_spent_utxos.push((*outpoint, entry.clone()));
                    utxo.remove(outpoint);
                    let _ = batch.spend_utxo(outpoint);
                    pool_consumed += entry.output.amount;
                }

                // Create bonds for each producer from pool funds
                let mut bonds_created: u64 = 0;
                for pubkey in &genesis_producers {
                    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_bytes());

                    if pool_consumed - bonds_created < bond_unit {
                        warn!(
                            "Insufficient remaining pool for producer {} bond",
                            hex::encode(&pubkey.as_bytes()[..8])
                        );
                        continue;
                    }

                    // Create deterministic bond hash for tracking
                    let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());

                    // Create Bond UTXO
                    let bond_outpoint = storage::Outpoint::new(bond_hash, 0);
                    let bond_entry = storage::UtxoEntry {
                        output: doli_core::transaction::Output::bond(
                            bond_unit,
                            pubkey_hash,
                            u64::MAX, // locked until withdrawal
                            0,        // genesis bond — slot 0
                        ),
                        height,
                        is_coinbase: false,
                        is_epoch_reward: false,
                    };
                    undo_created_utxos.push(bond_outpoint);
                    utxo.insert(bond_outpoint, bond_entry.clone());
                    batch.add_utxo(bond_outpoint, bond_entry);
                    bonds_created += bond_unit;

                    // Register producer with real bond outpoint
                    // registered_at = 0: Genesis producers are exempt from ACTIVATION_DELAY
                    // (they've been producing since block 1, no propagation delay needed)
                    let mut producer_info = storage::ProducerInfo::new_with_bonds(
                        *pubkey,
                        0,
                        bond_unit,
                        (bond_hash, 0),
                        era,
                        1, // 1 bond
                    );
                    if let Some(bls) = genesis_bls.get(pubkey) {
                        producer_info.bls_pubkey = bls.clone();
                    }

                    match producers.register(producer_info, height) {
                        Ok(()) => {
                            info!(
                                "  Registered genesis producer {} with bond from pool ({} DOLI bonded)",
                                hex::encode(&pubkey.as_bytes()[..8]),
                                bond_unit / UNITS_PER_COIN,
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

                // Return remainder to pool (pool funds minus bonds)
                let remainder = pool_consumed.saturating_sub(bonds_created);
                if remainder > 0 {
                    let remainder_hash = hash_with_domain(b"genesis_pool_remainder", b"doli");
                    let remainder_outpoint = storage::Outpoint::new(remainder_hash, 0);
                    let remainder_entry = storage::UtxoEntry {
                        output: doli_core::transaction::Output::normal(remainder, pool_hash),
                        height,
                        is_coinbase: false,
                        is_epoch_reward: false,
                    };
                    undo_created_utxos.push(remainder_outpoint);
                    utxo.insert(remainder_outpoint, remainder_entry.clone());
                    batch.add_utxo(remainder_outpoint, remainder_entry);
                    info!(
                        "Genesis pool remainder: {} DOLI returned to pool",
                        remainder / UNITS_PER_COIN
                    );
                }

                // Full producer write needed — clear + rebuild happened
                needs_full_producer_write = true;

                info!(
                    "Genesis complete: {} producers active, {} DOLI bonded, {} DOLI in pool",
                    producers.active_producers().len(),
                    bonds_created / UNITS_PER_COIN,
                    remainder / UNITS_PER_COIN,
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

        // Store the block AFTER all transaction validation has passed.
        // This prevents block store poisoning: if UTXO validation fails, the block
        // is NOT in the store, so sync can retry it cleanly on the next attempt.
        // Previously this was before the tx loop, causing the N4 stuck-at-22888 bug.
        self.block_store.put_block(&block, height)?;
        self.block_store.set_canonical_chain(block_hash, height)?;

        // Atomic state persistence via StateDb WriteBatch.
        // All state changes (UTXOs, chain_state, producers) committed in one batch.
        // Crash between any two writes is impossible — it's all-or-nothing.
        {
            let state = self.chain_state.read().await;
            batch.put_chain_state(&state);
            batch.set_last_applied(height, block_hash, state.best_slot);

            let producers = self.producer_set.read().await;
            if needs_full_producer_write {
                batch.write_full_producer_set(&producers);
            } else {
                batch.write_dirty_producers(
                    &producers,
                    &dirty_producer_keys,
                    &removed_producer_keys,
                    &dirty_exit_keys,
                );
            }
        }
        batch
            .commit()
            .map_err(|e| anyhow::anyhow!("StateDb batch commit failed: {}", e))?;

        // Persist undo data for this block (enables O(depth) rollbacks)
        let undo = storage::UndoData {
            spent_utxos: undo_spent_utxos,
            created_utxos: undo_created_utxos,
            producer_snapshot: undo_producer_snapshot,
        };
        self.state_db.put_undo(height, &undo);

        // Prune old undo data (keep last MAX_REORG_DEPTH blocks)
        const UNDO_KEEP_DEPTH: u64 = 2000; // 2x MAX_REORG_DEPTH for safety margin
        if height > UNDO_KEEP_DEPTH {
            self.state_db.prune_undo_before(height - UNDO_KEEP_DEPTH);
        }

        // Recompute our tier at epoch boundaries and build EpochSnapshot
        // (moved after batch commit to avoid borrow conflict with BlockBatch)
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
            all_pks.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            drop(producers);

            let snapshot = doli_core::EpochSnapshot::new(epoch, tier1, &all_pks, total_w);
            info!(
                "EpochSnapshot built: epoch={}, producers={}, weight={}, merkle={}",
                epoch, snapshot.total_producers, snapshot.total_weight, snapshot.merkle_root
            );

            // Reset minute tracker for the new epoch
            self.minute_tracker.reset();
        }

        // Per-block attestation: sign chain tip for finality gadget + record in tracker.
        self.create_and_broadcast_attestation(block_hash, block.header.slot, height)
            .await;
        if let Some(ref kp) = self.producer_key {
            let minute = attestation_minute(block.header.slot);
            if let Some(ref bls_kp) = self.bls_key {
                let bls_msg = crypto::attestation_message(&block_hash, block.header.slot);
                let bls_sig = crypto::bls_sign(&bls_msg, bls_kp.secret_key())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                self.minute_tracker
                    .record_with_bls(*kp.public_key(), minute, bls_sig);
            } else {
                self.minute_tracker.record(*kp.public_key(), minute);
            }
        }

        // Buffer block for archiving (will be flushed when finalized)
        if self.archive_tx.is_some() {
            if let Ok(data) = bincode::serialize(&block) {
                self.pending_archive.push_back(ArchiveBlock {
                    height,
                    hash: block_hash,
                    data,
                });
            }
            // Flush any blocks that just reached finality
            self.flush_finalized_to_archive().await;
        }

        // Broadcast new block event to WebSocket subscribers
        if let Some(ref ws_tx) = *self.ws_sender.read().await {
            let _ = ws_tx.send(rpc::WsEvent::NewBlock {
                hash: block_hash.to_hex(),
                height,
                slot: block.header.slot,
                timestamp: block.header.timestamp,
                producer: hex::encode(block.header.producer.as_bytes()),
                tx_count: block.transactions.len(),
            });
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
                // Broadcast to WebSocket subscribers
                if let Some(ref ws_tx) = *self.ws_sender.read().await {
                    let tx_type = format!("{:?}", tx.tx_type).to_lowercase();
                    let _ = ws_tx.send(rpc::WsEvent::NewTx {
                        hash: tx_hash.to_hex(),
                        tx_type,
                        size: tx.size(),
                        fee: 0,
                    });
                }
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
                // Serve snapshot at current tip regardless of requested hash.
                // The requesting node verifies the state root against quorum votes.
                // Previously this rejected requests where best_hash != block_hash,
                // causing a race condition: the peer advances between vote and
                // download, making snap sync fail 100% of the time on active chains.
                if chain_state.best_hash != block_hash {
                    info!(
                        "[SNAP_SYNC] Requested hash {} differs from tip {} — serving current tip (client verifies root)",
                        block_hash, chain_state.best_hash
                    );
                }
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

        // Already produced for this slot — skip before any eligibility/scheduler work.
        // The signed_slots DB is the authoritative slashing guard; this is a fast path
        // that avoids wasted eligibility checks, VDF, and block building when the
        // production timer fires again within the same 10s slot.
        if self.last_produced_slot == Some(current_slot as u64) {
            return Ok(());
        }

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
            let mut sync_state = self.sync_manager.write().await;
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
                self.consecutive_forced_recoveries = 0;
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
            ProductionAuthorization::BlockedAwaitingCanonicalBlock => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Awaiting first canonical gossip block after snap sync",
                    current_slot
                );
                return Ok(());
            }
        }

        // Log slot info periodically (every ~60 seconds)
        if current_slot.is_multiple_of(60) {
            info!(
                "Production check: now={}, genesis={}, current_slot={}, last_produced={:?}",
                now, self.params.genesis_time, current_slot, self.last_produced_slot
            );
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

        // Get active producers with bond counts derived from UTXO set.
        // Per WHITEPAPER Section 7: each bond unit = one ticket in the rotation.
        // Use active_producers_at_height to ensure all nodes have the same view -
        // new producers must wait ACTIVATION_DELAY blocks before entering the scheduler.
        let producers = self.producer_set.read().await;
        let active_producers: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        let total_producers = producers.total_count();
        drop(producers);

        // Derive bond counts from UTXO set (source of truth for bonds)
        let utxo = self.utxo_set.read().await;
        let active_with_weights: Vec<(PublicKey, u64)> = active_producers
            .into_iter()
            .map(|pk| {
                let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes());
                let count = utxo
                    .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                    .max(1) as u64;
                (pk, count)
            })
            .collect();
        drop(utxo);

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
        // After a proper resync via recover_from_peers(), the producer_set
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
                    // Prefer on-chain ProducerSet (deterministic). Fall back to GSet
                    // when the on-chain set is empty (always the case during genesis
                    // blocks 1-360, since producers aren't registered until height 361).
                    let mut known_producers: Vec<PublicKey> = {
                        let producers = self.producer_set.read().await;
                        let on_chain: Vec<PublicKey> = producers
                            .active_producers_at_height(height)
                            .iter()
                            .map(|p| p.public_key)
                            .collect();
                        if !on_chain.is_empty() {
                            on_chain
                        } else {
                            // On-chain set empty (genesis phase). Use GSet as fallback.
                            drop(producers); // release lock before acquiring gset lock
                            let gset_producers = {
                                let gset = self.producer_gset.read().await;
                                gset.active_producers(7200)
                            };
                            if !gset_producers.is_empty() {
                                gset_producers
                            } else {
                                let known = self.known_producers.read().await;
                                known.clone()
                            }
                        }
                    };

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

        // Build the block — single transaction list used for both merkle root and block body.
        // All transactions MUST be added to the builder BEFORE build(), which computes the
        // merkle root from exactly the transactions that will be in the final block.
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let mut builder =
            BlockBuilder::new(prev_hash, prev_slot, our_pubkey).with_params(self.params.clone());

        // ALL blocks: coinbase goes to reward pool (built-in mining pool).
        // No producer can spend rewards early — only the consensus engine distributes
        // pool funds to qualified producers at epoch boundaries.
        builder.add_coinbase(height, pool_hash);

        if !self.config.network.is_in_genesis(height) {
            // POST-GENESIS: distribute pool at epoch boundaries.
            let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
            if height > 0 && reward_epoch::is_epoch_start_with(height, blocks_per_epoch) {
                let completed_epoch = (height / blocks_per_epoch) - 1;

                // Skip epoch 0: genesis bonds consumed pool funds, remainder carries to E1.
                // Distributing E0 would create coins from nothing (pool drained by bonds).
                if completed_epoch == 0 {
                    info!("Epoch 0 (genesis): pool remainder carries to E1, no distribution");
                } else {
                    info!(
                        "Epoch {} completed at height {}, distributing pool rewards...",
                        completed_epoch, height
                    );

                    let epoch_outputs = self.calculate_epoch_rewards(completed_epoch).await;

                    if !epoch_outputs.is_empty() {
                        let total_reward: u64 = epoch_outputs.iter().map(|(amt, _)| *amt).sum();
                        info!(
                            "Distributing {} total reward to {} qualified producers for epoch {}",
                            total_reward,
                            epoch_outputs.len(),
                            completed_epoch
                        );

                        let coinbase = Transaction::new_epoch_reward_coinbase(
                            epoch_outputs,
                            height,
                            completed_epoch,
                        );
                        builder.add_transaction(coinbase);
                    } else {
                        debug!(
                            "No qualified producers in epoch {} — pool burned",
                            completed_epoch
                        );
                    }
                } // else completed_epoch != 0
            }
        }

        // During genesis: include VDF proof Registration TX in EVERY block we produce.
        // Idempotent — derive_genesis_producers_from_chain() deduplicates by public key.
        // Must not use a one-shot flag: orphaned blocks would set it permanently,
        // preventing retry in the canonical chain.
        if let Some(vdf_output_bytes) = self.genesis_vdf_output {
            if self.config.network.is_in_genesis(height) {
                let (bls_pubkey_bytes, bls_pop_bytes) = if let Some(ref bls_kp) = self.bls_key {
                    let pop = bls_kp
                        .proof_of_possession()
                        .expect("PoP signing cannot fail");
                    (
                        bls_kp.public_key().as_bytes().to_vec(),
                        pop.as_bytes().to_vec(),
                    )
                } else {
                    (vec![], vec![])
                };
                let reg_data = RegistrationData {
                    public_key: our_pubkey,
                    epoch: 0,
                    vdf_output: vdf_output_bytes.to_vec(),
                    vdf_proof: vec![],
                    prev_registration_hash: Hash::ZERO,
                    sequence_number: 0,
                    bond_count: 0, // Zero bond — handled at genesis end
                    bls_pubkey: bls_pubkey_bytes,
                    bls_pop: bls_pop_bytes,
                };
                let extra_data = bincode::serialize(&reg_data)
                    .expect("RegistrationData serialization cannot fail");
                let reg_tx = Transaction {
                    version: 1,
                    tx_type: TxType::Registration,
                    inputs: vec![],
                    outputs: vec![],
                    extra_data,
                };
                builder.add_transaction(reg_tx);
                info!(
                    "Included genesis VDF proof Registration TX in block {} for {}",
                    height,
                    hex::encode(&our_pubkey.as_bytes()[..8])
                );
            }
        }

        // Add transactions from mempool (validate covenant conditions before inclusion)
        let mempool_txs: Vec<Transaction> = {
            let mempool = self.mempool.read().await;
            mempool.select_for_block(1_000_000) // Up to ~1MB of transactions per block
        };
        {
            let utxo = self.utxo_set.read().await;
            let utxo_ctx = validation::ValidationContext::new(
                ConsensusParams::for_network(self.config.network),
                self.config.network,
                0,
                height,
            );
            for tx in &mempool_txs {
                if let Err(e) = validation::validate_transaction_with_utxos(tx, &utxo_ctx, &*utxo) {
                    warn!(
                        "Skipping mempool tx {} — UTXO validation failed: {}",
                        tx.hash(),
                        e
                    );
                    continue;
                }
                builder.add_transaction(tx.clone());
            }
        }

        // Recapture timestamp just before building the block.
        // The original `now` (line ~4508) can be 1-2 seconds stale due to async
        // work between capture and here (GSet reads, liveness filtering, VDF, etc).
        // If the stale timestamp maps to a different 2s rank window than the one
        // we passed eligibility for, validation rejects our own block as
        // "invalid producer for slot". Using a fresh timestamp ensures the block's
        // header.timestamp matches the rank window we were actually eligible in.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Sanity: abort if we crossed into a different slot during production
        if self.params.timestamp_to_slot(now) != current_slot {
            warn!(
                "Slot boundary crossed during block production (started slot {}, now slot {}) - aborting",
                current_slot,
                self.params.timestamp_to_slot(now)
            );
            return Ok(());
        }

        // Build attestation bitfield for presence_root.
        // Records which producers attested the current minute (from gossip).
        // 6 blocks per minute from different producers → union mitigates censorship.
        let current_minute = attestation_minute(current_slot);
        let attested_pks = self.minute_tracker.attested_in_minute(current_minute);
        let presence_root = if attested_pks.is_empty() {
            Hash::ZERO
        } else {
            // Build sorted producer list (same ordering as DeterministicScheduler)
            let sorted_producers: Vec<storage::producer::ProducerInfo> = {
                let producers = self.producer_set.read().await;
                let mut ps: Vec<storage::producer::ProducerInfo> = producers
                    .active_producers()
                    .iter()
                    .map(|p| (*p).clone())
                    .collect();
                ps.sort_by(|a, b| a.public_key.as_bytes().cmp(b.public_key.as_bytes()));
                ps
            };
            // Map attesting pubkeys to sorted indices
            let mut attested_indices = Vec::new();
            for pk in &attested_pks {
                if let Some(idx) = sorted_producers.iter().position(|p| &p.public_key == *pk) {
                    attested_indices.push(idx);
                }
            }
            encode_attestation_bitfield(&attested_indices)
        };
        let builder = builder.with_presence_root(presence_root);

        // Build header + finalized transaction list. The merkle root is computed from
        // exactly these transactions, guaranteeing header-body consistency.
        let (header, transactions) = match builder.build(now) {
            Some(result) => result,
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
            genesis_hash: header.genesis_hash,
            timestamp: header.timestamp,
            slot: header.slot,
            producer: header.producer,
            vdf_output,
            vdf_proof,
        };

        // Use the transactions from the builder — same list used for merkle root computation.
        // No duplicate transaction assembly needed.
        // Aggregate BLS signatures from minute tracker for on-chain proof.
        // Only includes producers that have BLS sigs for this minute.
        let aggregate_bls_signature = {
            let bls_sigs = self.minute_tracker.bls_sigs_for_minute(current_minute);
            info!(
                "BLS aggregate: minute={} sigs_count={} tracker_bls_total={}",
                current_minute,
                bls_sigs.len(),
                self.minute_tracker.bls_sig_count()
            );
            if bls_sigs.is_empty() {
                Vec::new()
            } else {
                let sigs: Vec<crypto::BlsSignature> = bls_sigs
                    .iter()
                    .filter_map(|(_, raw)| crypto::BlsSignature::try_from_slice(raw).ok())
                    .collect();
                if sigs.is_empty() {
                    Vec::new()
                } else {
                    match crypto::bls_aggregate(&sigs) {
                        Ok(agg) => agg.as_bytes().to_vec(),
                        Err(e) => {
                            warn!("BLS aggregation failed: {}", e);
                            Vec::new()
                        }
                    }
                }
            }
        };

        let block = Block {
            header: final_header,
            transactions,
            aggregate_bls_signature,
        };

        let block_hash = block.hash();
        info!(
            "[BLOCK_PRODUCED] hash={} height={} slot={} parent={}",
            block_hash, height, current_slot, block.header.prev_hash
        );

        // Apply the block locally
        self.apply_block(block.clone(), ValidationMode::Full)
            .await?;

        // Reset stale chain timer and refresh peers — we just produced a block.
        // refresh_all_peers() prevents peers from going stale during long consecutive
        // production windows (e.g., 30 bonds = 300s without gossip = stale_timeout).
        {
            let mut sync = self.sync_manager.write().await;
            sync.refresh_all_peers();
            sync.note_block_received_via_gossip();
        }

        // Mark that we produced for this slot
        self.last_produced_slot = Some(current_slot as u64);

        // Broadcast the block to the network
        // This is only done for blocks we produce ourselves - received blocks
        // are already on the network and don't need to be re-broadcast.
        // Header is sent first so peers can pre-validate before the full block arrives.
        if let Some(ref network) = self.network {
            let _ = network.broadcast_header(block.header.clone()).await;
            let _ = network.broadcast_block(block).await;
        }

        // Attest our own block for finality gadget + record in minute tracker.
        self.create_and_broadcast_attestation(block_hash, current_slot, height)
            .await;
        if let Some(ref kp) = self.producer_key {
            let minute = attestation_minute(current_slot);
            if let Some(ref bls_kp) = self.bls_key {
                let bls_msg = crypto::attestation_message(&block_hash, current_slot);
                let bls_sig = crypto::bls_sign(&bls_msg, bls_kp.secret_key())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                self.minute_tracker
                    .record_with_bls(*kp.public_key(), minute, bls_sig);
            } else {
                self.minute_tracker.record(*kp.public_key(), minute);
            }
        }

        // Flush any blocks that just reached finality
        self.flush_finalized_to_archive().await;

        Ok(())
    }

    /// Calculate epoch rewards using on-chain attestation bitfield qualification.
    ///
    /// Scans blocks in the completed epoch, decodes each block's `presence_root`
    /// bitfield, and counts unique attestation minutes per producer. Producers
    /// with ≥54/60 minutes qualify. Pool is distributed bond-weighted among qualifiers.
    ///
    /// Formula: reward[i] = pool × bonds[i] / Σ(qualifying_bonds)
    /// Non-qualifiers' share is redistributed to qualifiers (not burned).
    ///
    /// Returns a vector of (amount, pubkey_hash) tuples for the EpochReward transaction.
    async fn calculate_epoch_rewards(&self, epoch: u64) -> Vec<(u64, Hash)> {
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let epoch_start_height = epoch * blocks_per_epoch;
        let epoch_end_height = (epoch + 1) * blocks_per_epoch;

        // Get active producers at epoch end, sorted by pubkey (same order as bitfield encoding)
        let sorted_producers: Vec<storage::producer::ProducerInfo> = {
            let producers = self.producer_set.read().await;
            let mut ps: Vec<storage::producer::ProducerInfo> = producers
                .active_producers_at_height(epoch_end_height)
                .iter()
                .map(|p| (*p).clone())
                .collect();
            ps.sort_by(|a, b| a.public_key.as_bytes().cmp(b.public_key.as_bytes()));
            ps
        };

        if sorted_producers.is_empty() {
            return Vec::new();
        }

        let producer_count = sorted_producers.len();

        // Scan all blocks in epoch, decode presence_root bitfield, track attested minutes per producer
        // Key: producer index → set of attestation minutes they were attested in
        let mut attested_minutes: HashMap<usize, HashSet<u32>> = HashMap::new();

        for h in epoch_start_height..epoch_end_height {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(h) {
                // Skip blocks with zero presence_root (no attestation data)
                if block.header.presence_root.is_zero() {
                    continue;
                }

                let minute = attestation_minute(block.header.slot);
                let indices =
                    decode_attestation_bitfield(&block.header.presence_root, producer_count);

                // Union: for each producer index attested in this block, add the minute
                for idx in indices {
                    attested_minutes.entry(idx).or_default().insert(minute);
                }
            }
        }

        // Qualify producers: attested in ≥ ATTESTATION_QUALIFICATION_THRESHOLD minutes
        // Genesis epoch (epoch 0): all active producers qualify — no attestation data exists yet
        let threshold = ATTESTATION_QUALIFICATION_THRESHOLD;
        let qualified: Vec<&storage::producer::ProducerInfo> = if epoch == 0 {
            info!("Epoch 0 (genesis): all active producers qualify for rewards");
            sorted_producers.iter().collect()
        } else {
            sorted_producers
                .iter()
                .enumerate()
                .filter(|(idx, _)| {
                    let minutes = attested_minutes.get(idx).map(|s| s.len()).unwrap_or(0);
                    minutes as u32 >= threshold
                })
                .map(|(_, p)| p)
                .collect()
        };

        if qualified.is_empty() {
            warn!(
                "Epoch {}: no producers qualified — all rewards burned",
                epoch
            );
            return Vec::new();
        }

        let disqualified_count = sorted_producers.len() - qualified.len();
        if disqualified_count > 0 {
            info!(
                "Epoch {}: {}/{} producers qualified, {} disqualified (rewards redistributed)",
                epoch,
                qualified.len(),
                sorted_producers.len(),
                disqualified_count
            );
        }

        // Sum qualifying bonds (includes delegated bonds via selection_weight)
        let qualifying_bonds: u64 = qualified.iter().map(|p| p.selection_weight()).sum();

        if qualifying_bonds == 0 {
            return Vec::new();
        }

        // Calculate total pool from accumulated coinbase UTXOs in the reward pool.
        // Include current block's coinbase (not yet in UTXO set during block production)
        // so the distributed amount matches what the consume step will remove.
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let pool = {
            let utxo = self.utxo_set.read().await;
            let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
            let utxo_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
            utxo_total + self.params.block_reward(epoch_end_height)
        };
        info!(
            "Epoch {} reward pool: {} units from accumulated coinbase",
            epoch, pool
        );

        // Bond-weighted distribution among qualifiers: pool × bonds[i] / qualifying_bonds
        // Uses u128 intermediates to prevent overflow
        let mut reward_outputs = Vec::new();
        let mut distributed: u64 = 0;

        for producer_info in &qualified {
            let bonds = producer_info.selection_weight();
            let reward = (pool as u128 * bonds as u128 / qualifying_bonds as u128) as u64;

            if reward == 0 {
                continue;
            }

            let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, producer_info.public_key.as_bytes());

            // Find this producer's index in sorted list for attestation minute count
            let att_minutes = sorted_producers
                .iter()
                .position(|p| p.public_key == producer_info.public_key)
                .and_then(|idx| attested_minutes.get(&idx))
                .map(|s| s.len())
                .unwrap_or(0);

            info!(
                "Producer {} earned {} in epoch {} (attested: {}/60 minutes, bonds: {})",
                producer_info.public_key, reward, epoch, att_minutes, bonds
            );

            // Split rewards if producer has received delegations
            if producer_info.received_delegations.is_empty() {
                reward_outputs.push((reward, pubkey_hash));
            } else {
                let utxo = self.utxo_set.read().await;
                let own_bonds = utxo
                    .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                    .max(1) as u64;
                drop(utxo);
                let delegated: u64 = producer_info
                    .received_delegations
                    .iter()
                    .map(|(_, c)| *c as u64)
                    .sum();
                let total_bonds = own_bonds + delegated;

                let own_share = reward * own_bonds / total_bonds;
                let delegated_share = reward - own_share;
                let delegate_fee = delegated_share * DELEGATE_REWARD_PCT as u64 / 100;
                let staker_pool = delegated_share - delegate_fee; // remainder to stakers, no dust

                // Distribute staker pool to delegators, last gets remainder
                let mut staker_distributed = 0u64;
                let delegators: Vec<_> = producer_info.received_delegations.iter().collect();
                for (i, (delegator_hash, bond_count)) in delegators.iter().enumerate() {
                    let delegator_reward = if i == delegators.len() - 1 {
                        staker_pool - staker_distributed // last delegator gets remainder
                    } else {
                        staker_pool * (*bond_count as u64) / delegated
                    };
                    staker_distributed += delegator_reward;
                    if delegator_reward > 0 {
                        reward_outputs.push((delegator_reward, *delegator_hash));
                    }
                }

                let producer_total = own_share + delegate_fee;
                if producer_total > 0 {
                    reward_outputs.push((producer_total, pubkey_hash));
                }
            }

            distributed += reward;
        }

        // Integer division remainder goes to first qualifier
        let remainder = pool.saturating_sub(distributed);
        if remainder > 0 && !reward_outputs.is_empty() {
            reward_outputs[0].0 += remainder;
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
        let epoch_len = SLOTS_PER_EPOCH as u64;

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
                            let bond_outputs: Vec<(usize, &doli_core::transaction::Output)> = tx
                                .outputs
                                .iter()
                                .enumerate()
                                .filter(|(_, o)| {
                                    o.output_type == doli_core::transaction::OutputType::Bond
                                })
                                .collect();

                            if let Some(&(bond_index, _)) = bond_outputs.first() {
                                let tx_hash = tx.hash();
                                let era = self.params.height_to_era(height);
                                let total_bond_amount: u64 =
                                    bond_outputs.iter().map(|(_, o)| o.amount).sum();
                                let mut producer_info = storage::ProducerInfo::new_with_bonds(
                                    reg_data.public_key,
                                    height,
                                    total_bond_amount,
                                    (tx_hash, bond_index as u32),
                                    era,
                                    reg_data.bond_count,
                                );
                                producer_info.bls_pubkey = reg_data.bls_pubkey.clone();
                                producers.queue_update(PendingProducerUpdate::Register {
                                    info: Box::new(producer_info),
                                    height,
                                });
                            }
                        }
                    }
                    TxType::Exit => {
                        if let Some(exit_data) = tx.exit_data() {
                            // Convert Exit to RequestWithdrawal for all bonds
                            if let Some(info) = producers.get_by_pubkey(&exit_data.public_key) {
                                let all_bonds = info.bond_count;
                                if all_bonds > 0 {
                                    if let Some(producer) =
                                        producers.get_by_pubkey_mut(&exit_data.public_key)
                                    {
                                        producer.withdrawal_pending_count += all_bonds;
                                    }
                                    producers.queue_update(
                                        PendingProducerUpdate::RequestWithdrawal {
                                            pubkey: exit_data.public_key,
                                            bond_count: all_bonds,
                                            bond_unit,
                                        },
                                    );
                                } else {
                                    producers.queue_update(PendingProducerUpdate::Exit {
                                        pubkey: exit_data.public_key,
                                        height,
                                    });
                                }
                            } else {
                                producers.queue_update(PendingProducerUpdate::Exit {
                                    pubkey: exit_data.public_key,
                                    height,
                                });
                            }
                        }
                    }
                    TxType::SlashProducer => {
                        if let Some(slash_data) = tx.slash_data() {
                            producers.queue_update(PendingProducerUpdate::Slash {
                                pubkey: slash_data.producer_pubkey,
                                height,
                            });
                        }
                    }
                    TxType::AddBond => {
                        if let Some(add_bond_data) = tx.add_bond_data() {
                            // Guard: skip if producer not registered (orphan Bond UTXO)
                            let pubkey = &add_bond_data.producer_pubkey;
                            let is_registered = producers.get_by_pubkey(pubkey).is_some();
                            let has_pending_reg = producers
                                .pending_updates_for(pubkey)
                                .iter()
                                .any(|u| matches!(u, PendingProducerUpdate::Register { .. }));

                            if is_registered || has_pending_reg {
                                let tx_hash = tx.hash();
                                // Lock/unlock: Bond output indices from actual TX outputs
                                let bond_outpoints: Vec<(crypto::Hash, u32)> = tx
                                    .outputs
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, o)| {
                                        o.output_type == doli_core::transaction::OutputType::Bond
                                    })
                                    .map(|(i, _)| (tx_hash, i as u32))
                                    .collect();
                                producers.queue_update(PendingProducerUpdate::AddBond {
                                    pubkey: add_bond_data.producer_pubkey,
                                    outpoints: bond_outpoints,
                                    bond_unit,
                                    creation_slot: block.header.slot,
                                });
                            }
                        }
                    }
                    TxType::RequestWithdrawal => {
                        if let Some(data) = tx.withdrawal_request_data() {
                            // During rebuild, validate and queue the withdrawal
                            if let Some(info) = producers.get_by_pubkey(&data.producer_pubkey) {
                                let available = info
                                    .bond_count
                                    .saturating_sub(info.withdrawal_pending_count);
                                if data.bond_count <= available {
                                    if let Some(producer) =
                                        producers.get_by_pubkey_mut(&data.producer_pubkey)
                                    {
                                        producer.withdrawal_pending_count += data.bond_count;
                                    }
                                    producers.queue_update(
                                        PendingProducerUpdate::RequestWithdrawal {
                                            pubkey: data.producer_pubkey,
                                            bond_count: data.bond_count,
                                            bond_unit,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    TxType::DelegateBond => {
                        if let Some(data) = tx.delegate_bond_data() {
                            producers.queue_update(PendingProducerUpdate::DelegateBond {
                                delegator: data.delegator,
                                delegate: data.delegate,
                                bond_count: data.bond_count,
                            });
                        }
                    }
                    TxType::RevokeDelegation => {
                        if let Some(data) = tx.revoke_delegation_data() {
                            producers.queue_update(PendingProducerUpdate::RevokeDelegation {
                                delegator: data.delegator,
                            });
                        }
                    }
                    // ProtocolActivation doesn't modify the producer set —
                    // it's processed in apply_block where chain_state is available.
                    _ => {}
                }
            }

            // Replicate GENESIS PHASE COMPLETE: register VDF-proven producers
            // when crossing the genesis boundary during rebuild.
            // Genesis producers are registered immediately (not deferred).
            if genesis_blocks > 0 && height == genesis_blocks + 1 {
                let genesis_producers = self.derive_genesis_producers_from_chain();
                let genesis_bls = self.genesis_bls_pubkeys();
                let era = self.params.height_to_era(height);
                for pubkey in &genesis_producers {
                    let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());
                    // registered_at = 0: Genesis producers exempt from ACTIVATION_DELAY
                    let mut producer_info = storage::ProducerInfo::new_with_bonds(
                        *pubkey,
                        0,
                        bond_unit,
                        (bond_hash, 0),
                        era,
                        1,
                    );
                    if let Some(bls) = genesis_bls.get(pubkey) {
                        producer_info.bls_pubkey = bls.clone();
                    }
                    let _ = producers.register(producer_info, height);
                }
            }

            // Apply deferred updates: every block in epoch 0, then at epoch boundaries
            let is_epoch_0 = height < epoch_len;
            let is_boundary = height > 0 && height.is_multiple_of(epoch_len);
            if is_epoch_0 || is_boundary {
                producers.apply_pending_updates();
            }

            // Process completed unbonding periods after each block
            producers.process_unbonding(height, UNBONDING_PERIOD);
        }

        // DO NOT apply remaining pending_updates here. If target_height doesn't
        // land on an epoch boundary, pending updates must stay deferred — they will
        // be applied when apply_block() processes the next epoch boundary. Applying
        // them early produces a producer set that never existed on-chain, causing
        // "invalid producer for slot" failures during reorg block validation.

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

        // Invalidate genesis producer cache if rollback crosses genesis boundary
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks > 0 && target_height <= genesis_blocks {
            info!("[ROLLBACK] Crossing genesis boundary — invalidating genesis producer cache");
            self.cached_genesis_producers = std::sync::OnceLock::new();
        }

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

        // Try undo-based rollback first (O(1) for single block)
        if let Some(undo) = self.state_db.get_undo(local_height) {
            info!(
                "Undo-based rollback: reverting block at height {}",
                local_height
            );
            {
                let mut utxo = self.utxo_set.write().await;

                // Remove UTXOs created by this block
                for outpoint in &undo.created_utxos {
                    utxo.remove(outpoint);
                }

                // Restore UTXOs spent by this block
                for (outpoint, entry) in &undo.spent_utxos {
                    utxo.insert(*outpoint, entry.clone());
                }
            }

            // Restore ProducerSet from undo snapshot
            if let Ok(restored_producers) =
                bincode::deserialize::<storage::ProducerSet>(&undo.producer_snapshot)
            {
                let mut producers = self.producer_set.write().await;
                *producers = restored_producers;
            } else {
                warn!("Failed to deserialize producer snapshot, rebuilding from blocks");
                let mut producers = self.producer_set.write().await;
                self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
            }
        } else {
            // Legacy fallback: rebuild from genesis (no undo data)
            warn!(
                "No undo data for height {} — falling back to rebuild from genesis",
                local_height
            );

            // Pre-check: snap-synced nodes can't rebuild from genesis — re-snap to recover
            if self.block_store.get_block_by_height(1)?.is_none() {
                warn!("Rollback: snap-synced node with no undo data. Re-snapping to recover.");
                self.reset_state_only().await?;
                return Ok(true);
            }

            let genesis_producers = if genesis_blocks > 0 && target_height > genesis_blocks {
                self.derive_genesis_producers_from_chain()
            } else {
                Vec::new()
            };
            let bond_unit = self.config.network.bond_unit();
            {
                let mut utxo = self.utxo_set.write().await;
                utxo.clear();
                for height in 1..=target_height {
                    let block = self
                        .block_store
                        .get_block_by_height(height)?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Rollback UTXO rebuild: missing block at height {}",
                                height
                            )
                        })?;
                    for (tx_index, tx) in block.transactions.iter().enumerate() {
                        let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                        if !is_reward_tx {
                            let _ = utxo.spend_transaction(tx);
                        }
                        utxo.add_transaction(tx, height, is_reward_tx, block.header.slot);
                    }
                    if genesis_blocks > 0 && height == genesis_blocks + 1 {
                        Self::consume_genesis_bond_utxos(
                            &mut utxo,
                            &genesis_producers,
                            bond_unit,
                            height,
                        );
                    }
                }
            }

            {
                let mut producers = self.producer_set.write().await;
                self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
            }
        }

        // Force scheduler rebuild after producer set reconstruction
        self.cached_scheduler = None;

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

        // Atomically persist the rolled-back state via StateDb.
        // Collects all UTXOs from in-memory set and writes everything in one WriteBatch.
        {
            let state = self.chain_state.read().await;
            let producers = self.producer_set.read().await;
            let utxo = self.utxo_set.read().await;
            let utxo_pairs: Vec<(storage::Outpoint, storage::UtxoEntry)> = match &*utxo {
                UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
                // RocksDb variant shouldn't occur (we load InMemory from StateDb), but handle it
                UtxoSet::RocksDb(_) => self.state_db.iter_utxos(),
            };
            self.state_db
                .atomic_replace(&state, &producers, utxo_pairs.into_iter())
                .map_err(|e| anyhow::anyhow!("StateDb atomic_replace failed: {}", e))?;
        }

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
                    if let Err(e) = self.apply_block(block, ValidationMode::Light).await {
                        warn!("Failed to apply pending sync block: {}", e);
                        self.sync_manager.write().await.block_apply_failed();
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

        // Archive catch-up: after sync completes, backfill archive from block_store.
        // Runs once — fills any missing .block files between 1 and tip.
        if !self.archive_caught_up {
            if let Some(ref archive_dir) = self.archive_dir {
                let tip = self.chain_state.read().await.best_height;
                if tip > 0 {
                    info!(
                        "[ARCHIVER] Catch-up: scanning for gaps up to height {}",
                        tip
                    );
                    match storage::archiver::BlockArchiver::catch_up(
                        archive_dir,
                        &self.block_store,
                        tip,
                    ) {
                        Ok(n) if n > 0 => info!("[ARCHIVER] Catch-up: filled {} missing blocks", n),
                        Ok(_) => info!("[ARCHIVER] Catch-up: archive complete, no gaps"),
                        Err(e) => warn!("[ARCHIVER] Catch-up error: {}", e),
                    }
                    self.archive_caught_up = true;
                }
            } else {
                self.archive_caught_up = true;
            }
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

        // SAFETY NET: If fork recovery exceeded max depth, the fork is too deep
        // for reorg. Recover from peers (snap sync).
        {
            let exceeded = self
                .sync_manager
                .write()
                .await
                .take_fork_exceeded_max_depth();
            if exceeded {
                warn!("Fork recovery exceeded max depth — recovering from peers");
                self.force_recover_from_peers().await?;
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
                let result = match our_hash {
                    Some(h) if h == peer_hash => network::sync::ProbeResult::Match,
                    Some(_) => network::sync::ProbeResult::Mismatch,
                    None => network::sync::ProbeResult::NotInStore,
                };
                self.sync_manager
                    .write()
                    .await
                    .fork_sync_handle_probe(result);
            }

            // Transition: search complete — provide ancestor hash from our block_store.
            //
            // Store-limited: search stopped because block store doesn't cover the range
            // (snap sync gap). This is NOT a deep fork — the node just doesn't have
            // the historical blocks. Re-snap to get back to tip and resume producing.
            if self.sync_manager.read().await.fork_sync_store_limited() {
                let floor = self.sync_manager.read().await.store_floor();
                warn!(
                    "Fork sync: search limited by block store floor (height {}). \
                     Re-snapping to restore state — NOT a deep fork.",
                    floor
                );
                self.sync_manager.write().await.fork_sync_clear();
                self.reset_state_only().await?;
                return Ok(());
            }

            // Bottomed out: genuine deep fork — no common ancestor within MAX_FORK_SYNC_DEPTH
            // and the block store covers the full range. Full resync required.
            if self.sync_manager.read().await.fork_sync_bottomed_out() {
                warn!("Fork sync: binary search hit floor without finding common ancestor — full resync required");
                self.sync_manager.write().await.fork_sync_clear();
                self.force_recover_from_peers().await?;
                return Ok(());
            }
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
        {
            let needs_resync = self.sync_manager.read().await.needs_genesis_resync();
            if needs_resync {
                warn!("Persistent chain rejection: peers reject our tip. Recovering from peers.");
                self.force_recover_from_peers().await?;
                return Ok(());
            }
        }

        // DEEP FORK ESCALATION: If peers consistently reject our chain tip (10+ empty
        // header responses), normal sync and fork recovery can't bridge the gap.
        {
            let is_deep_fork = self.sync_manager.read().await.is_deep_fork_detected();
            if is_deep_fork {
                warn!("Deep fork detected: peers consistently reject our chain tip. Recovering from peers.");
                self.force_recover_from_peers().await?;
                return Ok(());
            }
        }

        // Check if we need to request sync
        {
            let mut sm = self.sync_manager.write().await;
            // Snap sync: batch-send GetStateRoot to ALL peers simultaneously.
            // Responses cluster in ~1-2s so peers report the same height/root.
            let snap_batch = sm.next_snap_requests();
            if !snap_batch.is_empty() {
                if let Some(ref network) = self.network {
                    for (peer_id, request) in snap_batch {
                        let _ = network.request_sync(peer_id, request).await;
                    }
                }
            }
            // Normal sync: one request per tick
            if let Some((peer_id, request)) = sm.next_request() {
                if let Some(ref network) = self.network {
                    let _ = network.request_sync(peer_id, request).await;
                }
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

        // PORT REACHABILITY WARNING (one-shot, mainnet producers only)
        // After 60s of running, if we have zero peers it likely means the P2P
        // port is not reachable from the internet (firewall/NAT misconfiguration).
        if !self.port_check_done
            && self.config.network == Network::Mainnet
            && self.producer_key.is_some()
        {
            let uptime = self
                .first_peer_connected
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            // Wait at least 60s after first peer, or 120s total if no peer ever connected
            let threshold = if self.first_peer_connected.is_some() {
                60
            } else {
                120
            };
            if uptime >= threshold
                || (self.first_peer_connected.is_none()
                    && now_secs.is_multiple_of(120)
                    && now_secs > 0)
            {
                let peer_count = self.sync_manager.read().await.peer_count();
                if peer_count == 0 {
                    let p2p_port = self
                        .config
                        .listen_addr
                        .split(':')
                        .next_back()
                        .unwrap_or("30300");
                    warn!("════════════════════════════════════════════════════════════════");
                    warn!(
                        "  WARNING: 0 peers after {}s — P2P port {} may be unreachable",
                        threshold, p2p_port
                    );
                    warn!("  Blocks you produce will NOT propagate to the network.");
                    warn!(
                        "  Fix: ensure TCP port {} is open (inbound) on your firewall.",
                        p2p_port
                    );
                    warn!("════════════════════════════════════════════════════════════════");
                } else {
                    info!(
                        "Port check: {} peers connected after {}s — OK",
                        peer_count, threshold
                    );
                }
                self.port_check_done = true;
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

    /// Derive genesis producers from on-chain VDF proof Registration TXs.
    ///
    /// Scans genesis blocks for Registration TXs and returns producers
    /// with valid registration data. VDF proofs were already verified
    /// at block acceptance time (validation.rs:validate_registration).
    ///
    /// Cached via OnceLock for the common path. Invalidated by
    /// execute_reorg/rollback_one_block when a reorg crosses the genesis
    /// boundary (target_height <= genesis_blocks).
    fn derive_genesis_producers_from_chain(&self) -> Vec<PublicKey> {
        self.cached_genesis_producers
            .get_or_init(|| {
                let genesis_blocks = self.config.network.genesis_blocks();
                if genesis_blocks == 0 {
                    return Vec::new();
                }

                let mut seen = std::collections::HashSet::new();
                let mut proven_producers = Vec::new();

                for height in 1..=genesis_blocks {
                    if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                        for tx in &block.transactions {
                            if tx.tx_type == TxType::Registration {
                                if let Some(reg_data) = tx.registration_data() {
                                    // VDF proof already verified at block acceptance
                                    // (validate_registration in validation.rs).
                                    // Just check format and deduplicate.
                                    if reg_data.vdf_output.len() == 32
                                        && seen.insert(reg_data.public_key)
                                    {
                                        info!(
                                            "  Genesis producer {} (VDF pre-verified)",
                                            hex::encode(&reg_data.public_key.as_bytes()[..8])
                                        );
                                        proven_producers.push(reg_data.public_key);
                                    }
                                }
                            }
                        }
                    }
                }

                if proven_producers.is_empty() {
                    // Snap-synced nodes don't have genesis blocks — fall back to
                    // hardcoded chainspec producers to avoid breaking the scheduler.
                    let fallback: Vec<PublicKey> = match self.config.network {
                        Network::Mainnet => {
                            use doli_core::genesis::mainnet_genesis_producers;
                            mainnet_genesis_producers()
                                .into_iter()
                                .map(|(pk, _)| pk)
                                .collect()
                        }
                        Network::Testnet => {
                            use doli_core::genesis::testnet_genesis_producers;
                            testnet_genesis_producers()
                                .into_iter()
                                .map(|(pk, _)| pk)
                                .collect()
                        }
                        Network::Devnet => Vec::new(),
                    };
                    if !fallback.is_empty() {
                        warn!(
                            "[GENESIS] Blocks 1..={} missing (snap sync). Using {} chainspec \
                             genesis producers as fallback.",
                            genesis_blocks,
                            fallback.len()
                        );
                    }
                    return fallback;
                }

                info!(
                    "Derived {} genesis producers with valid VDF proofs (from {} genesis blocks)",
                    proven_producers.len(),
                    genesis_blocks
                );

                proven_producers
            })
            .clone()
    }

    /// Extract BLS pubkeys from genesis registration TXs.
    /// Returns a map from producer PublicKey → BLS pubkey bytes (48 bytes).
    /// Only includes producers that included a BLS pubkey in their registration.
    fn genesis_bls_pubkeys(&self) -> std::collections::HashMap<PublicKey, Vec<u8>> {
        let genesis_blocks = self.config.network.genesis_blocks();
        let mut bls_keys = std::collections::HashMap::new();

        for height in 1..=genesis_blocks {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                for tx in &block.transactions {
                    if tx.tx_type == TxType::Registration {
                        if let Some(reg_data) = tx.registration_data() {
                            if !reg_data.bls_pubkey.is_empty()
                                && !bls_keys.contains_key(&reg_data.public_key)
                            {
                                bls_keys.insert(reg_data.public_key, reg_data.bls_pubkey.clone());
                            }
                        }
                    }
                }
            }
        }

        bls_keys
    }

    /// Consume pool UTXOs for genesis bond migration during UTXO rebuild.
    ///
    /// Must be called at height == genesis_blocks + 1 in any UTXO rebuild loop
    /// to match the apply_block() behavior. All per-block coinbase goes to the
    /// reward pool — this consumes pool UTXOs to create bonds for each producer
    /// and returns the remainder to the pool.
    fn consume_genesis_bond_utxos(
        utxo: &mut storage::UtxoSet,
        genesis_producers: &[PublicKey],
        bond_unit: u64,
        height: u64,
    ) {
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();

        // Collect and sort all pool UTXOs (deterministic order)
        let mut pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
        pool_utxos.sort_by(|(a_op, a_entry), (b_op, b_entry)| {
            a_entry
                .height
                .cmp(&b_entry.height)
                .then_with(|| a_op.tx_hash.cmp(&b_op.tx_hash))
                .then_with(|| a_op.index.cmp(&b_op.index))
        });

        // Consume all pool UTXOs
        let mut pool_consumed: u64 = 0;
        for (outpoint, entry) in &pool_utxos {
            utxo.remove(outpoint);
            pool_consumed += entry.output.amount;
        }

        // Create bonds for each producer from pool funds
        let mut bonds_created: u64 = 0;
        for pubkey in genesis_producers {
            let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_bytes());

            if pool_consumed - bonds_created < bond_unit {
                continue;
            }

            let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());
            let bond_outpoint = storage::Outpoint::new(bond_hash, 0);
            let bond_entry = storage::UtxoEntry {
                output: doli_core::transaction::Output::bond(
                    bond_unit,
                    pubkey_hash,
                    u64::MAX, // locked until withdrawal
                    0,        // genesis bond — slot 0
                ),
                height,
                is_coinbase: false,
                is_epoch_reward: false,
            };
            utxo.insert(bond_outpoint, bond_entry);
            bonds_created += bond_unit;
        }

        // Return remainder to pool
        let remainder = pool_consumed.saturating_sub(bonds_created);
        if remainder > 0 {
            let remainder_hash = hash_with_domain(b"genesis_pool_remainder", b"doli");
            let remainder_outpoint = storage::Outpoint::new(remainder_hash, 0);
            let remainder_entry = storage::UtxoEntry {
                output: doli_core::transaction::Output::normal(remainder, pool_hash),
                height,
                is_coinbase: false,
                is_epoch_reward: false,
            };
            utxo.insert(remainder_outpoint, remainder_entry);
        }
    }

    /// Save all node state — now a no-op.
    ///
    /// All state persistence happens atomically via StateDb WriteBatch.
    /// apply_block() commits chain_state + producers + UTXOs in one batch.
    /// Reorg/rollback/snap_sync use atomic_replace().
    async fn save_state(&self) -> Result<()> {
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
