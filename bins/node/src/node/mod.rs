//! Node implementation
//!
//! Integrates all DOLI components: storage, networking, RPC, mempool, and sync.
// v0.2.1-test: upgrade pipeline validation

mod apply_block;
#[cfg(test)]
mod attestation_authority_tests;
#[allow(unused_imports)]
pub use apply_block::state_fp_sr_field;
mod block_handling;
pub mod checkpoint_health;
mod event_loop;
mod floor_window;
mod force_reorg;
pub use force_reorg::ForceReorgOutcome;
mod fork_recovery;
mod genesis;
mod holdings;
use holdings::holdings_of_every_producer;
mod init;
mod maintainer_rewind;
mod network_events;

// Re-export Phase 4 disk cleanup helper so integration tests can verify
// orphan-dir removal behavior. Used only through the lib target.
#[allow(unused_imports)]
pub use init::cleanup_orphan_utxo_store;
#[allow(unused_imports)]
pub use init::recover_body_gaps;
mod periodic;
mod production;
mod rewards;
#[allow(unused_imports)]
pub use rewards::IncompleteEpochStoreError;
mod rollback;
pub use rollback::RollbackOutcome;
mod rollback_authority;
pub use rollback_authority::RollbackAuthority;
mod startup;
mod state_root_serve;
mod state_snapshot_serve;
mod tx_announcements;
mod validation_checks;
pub mod wedge_alarm;
mod wedge_escape;
pub mod wedge_outcome;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crypto::hash::{hash as crypto_hash, hash_with_domain};
use crypto::{Hash, KeyPair, PublicKey, ADDRESS_DOMAIN};
use doli_core::block::BlockBuilder;
use doli_core::consensus::{
    self, construct_vdf_input, reward_epoch, ConsensusParams, DELEGATE_REWARD_PCT, UNBONDING_PERIOD,
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
    ProducerGSet, Transaction,
};
use network::protocols::{
    SyncRequest, SyncResponse, CURRENT_PROTOCOL_VERSION, EPOCH_STATE_FORMAT_VERSION,
};
use network::{
    EquivocationDetector, EquivocationProof, NetworkCommand, NetworkConfig, NetworkEvent,
    NetworkEvidence, NetworkService, PeerId, ProductionAuthorization, ReorgResult, SyncConfig,
    SyncManager,
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
    pub config: NodeConfig,
    /// Consensus parameters
    pub params: ConsensusParams,
    /// Block storage
    pub block_store: Arc<BlockStore>,
    /// Unified state database (UTXO + producers + chain state, atomic WriteBatch per block)
    pub state_db: Arc<StateDb>,
    /// UTXO set (in-memory working copy, populated from state_db on startup)
    pub utxo_set: Arc<RwLock<UtxoSet>>,
    /// Chain state
    pub chain_state: Arc<RwLock<ChainState>>,
    /// Producer set
    pub producer_set: Arc<RwLock<ProducerSet>>,
    /// Mempool
    pub mempool: Arc<RwLock<Mempool>>,
    /// Network service
    pub network: Option<NetworkService>,
    /// PeerIds of bootstrap/seed nodes — disconnected after DHT bootstrap + gossip verified.
    /// Frees seed slots so the network scales without seed becoming a bottleneck.
    pub seed_peer_ids: Vec<network::PeerId>,
    /// Whether we've already disconnected from seeds (one-shot, don't reconnect)
    pub seeds_released: bool,
    /// Sync manager
    pub sync_manager: Arc<RwLock<SyncManager>>,
    /// Shutdown flag
    pub shutdown: Arc<RwLock<bool>>,
    /// Producer key (if producing blocks)
    pub producer_key: Option<KeyPair>,
    /// BLS key pair for aggregate attestation signatures
    pub bls_key: Option<crypto::BlsKeyPair>,
    /// Last slot we successfully produced a block for (to avoid double-producing).
    /// Set after successful broadcast — checked at the top of try_produce_block()
    /// before any eligibility/scheduler work to save CPU and silence fallback-rank noise.
    pub last_produced_slot: Option<u64>,
    /// All known producers (persists across epochs for round-robin)
    pub known_producers: Arc<RwLock<Vec<PublicKey>>>,
    /// Time when we first connected to a peer (for discovery grace period)
    pub first_peer_connected: Option<Instant>,
    /// Equivocation detector for slashing double-signers
    pub equivocation_detector: Arc<RwLock<EquivocationDetector>>,
    /// VDF calibrator for dynamic iteration adjustment
    pub vdf_calibrator: Arc<RwLock<VdfCalibrator>>,
    /// Cache of blocks received during forks (for reorg execution)
    pub fork_block_cache: Arc<RwLock<HashMap<Hash, Block>>>,
    /// Time when the bootstrap producer list last changed (for stability check)
    pub last_producer_list_change: Option<Instant>,
    /// Producer discovery CRDT with cryptographic announcements
    pub producer_gset: Arc<RwLock<ProducerGSet>>,
    /// Adaptive gossip controller for smart interval management
    pub adaptive_gossip: Arc<RwLock<AdaptiveGossip>>,
    /// Our current producer announcement (if we are a producer)
    pub our_announcement: Arc<RwLock<Option<ProducerAnnouncement>>>,
    /// Sequence number for our announcements (monotonically increasing)
    pub announcement_sequence: Arc<AtomicU64>,
    /// GSet size at last broadcast. Suppresses redundant gossip when unchanged.
    /// Reset to 0 on new peer connection so late joiners receive the full GSet.
    pub last_broadcast_gset_len: usize,
    /// Signed slots database (prevents double-signing after restart)
    pub signed_slots_db: Option<SignedSlotsDb>,
    /// Number of shallow fork rollbacks performed since last successful sync.
    /// Capped at MAX_SHALLOW_ROLLBACKS to prevent rolling back the entire chain.
    pub shallow_rollback_count: u32,
    /// Cumulative rollback depth since last successful block application.
    /// Tracks how far we've rolled back in total. Capped at MAX_CUMULATIVE_ROLLBACK (50)
    /// to prevent cascading rollbacks from reaching genesis.
    pub cumulative_rollback_depth: u32,
    /// INC-I-174 (REQ-174-010). Semantics AND the REQ-174-010 metrics-surface status:
    /// `maintainer_rewind/mod.rs`, "Counter semantics". In-process only.
    pub maintainer_rewind_count: u64,
    /// INC-I-174 (REQ-174-005/010). Semantics: `maintainer_rewind/mod.rs`, "Counter
    /// semantics". In-process only — see the note on `maintainer_rewind_count`.
    pub maintainer_rewind_unrestored_count: u64,
    /// Slots for which we've seen a block via gossip (not yet applied to block_store).
    /// Used by rank 1 to avoid producing a competing block when rank 0 already produced
    /// but the block hasn't been applied to disk yet. Cleaned periodically.
    pub seen_blocks_for_slot: std::collections::HashSet<u32>,
    /// Producers excluded from round-robin for missing their slot.
    /// Unified epoch scheduler state: producer lists, bond snapshot, attestation
    /// accumulators. Single source of truth for all consensus-derived scheduler inputs.
    pub epoch_state: doli_core::EpochState,
    /// Whether this node is in the active production list. Recomputed at each epoch boundary.
    pub is_active_producer: bool,
    /// Last epoch for which we computed our active status (to detect epoch boundaries).
    pub last_active_status_epoch: Option<u64>,
    /// Channel to forward gossip votes to the UpdateService
    pub vote_tx: Option<tokio::sync::mpsc::Sender<node_updater::VoteMessage>>,
    /// Shared pending update state from UpdateService (for RPC to read live)
    pub pending_update: Option<Arc<RwLock<Option<node_updater::PendingUpdate>>>>,
    /// Last time we attempted to redial bootstrap nodes (rate limiter)
    pub last_peer_redial: Option<Instant>,
    /// REQ-NET-001: Exponential backoff tracking for bootstrap node reconnection.
    /// Maps bootstrap address → (failure_count, last_attempt_time).
    /// Reset when any peer connects (peer_count > 0).
    pub bootstrap_backoff: HashMap<String, (u32, Instant)>,
    /// Last height at which each producer produced a block (for liveness filter).
    /// Populated from chain data in apply_block(), rebuilt from block_store on startup.
    /// Used by bootstrap scheduling to exclude stale producers from primary rotation.
    pub producer_liveness: HashMap<PublicKey, u64>,
    /// Cached genesis VDF proof output (computed in background at startup during genesis).
    /// Used to create a zero-bond Registration TX that proves VDF work on-chain.
    pub genesis_vdf_output: Option<[u8; 32]>,
    /// Cached state root, updated atomically after each block application.
    /// Avoids race conditions when GetStateRoot reads during apply_block.
    /// Tuple: (state_root, block_hash, block_height)
    pub cached_state_root: Arc<RwLock<Option<(Hash, Hash, u64)>>>,
    /// Cached genesis producers. Invalidated on reorgs crossing genesis boundary.
    pub cached_genesis_producers: std::sync::OnceLock<Vec<PublicKey>>,
    /// Whether we've already checked inbound peer connectivity (one-shot after 60s)
    pub port_check_done: bool,
    /// On-chain maintainer set (3-5 members, persisted, bootstrapped from first 5 producers).
    /// Used by the auto-update system for release signature verification.
    pub maintainer_state: Option<Arc<RwLock<storage::MaintainerState>>>,
    /// Channel to send blocks to the archiver (if --archive-to is set)
    pub archive_tx: Option<tokio::sync::mpsc::Sender<ArchiveBlock>>,
    /// Blocks waiting for finality before being archived
    pub pending_archive: std::collections::VecDeque<ArchiveBlock>,
    /// Archive directory path (for catch-up after sync)
    pub archive_dir: Option<PathBuf>,
    /// Whether archive catch-up has been performed after sync
    pub archive_caught_up: bool,
    /// WebSocket broadcast sender for real-time events (new blocks, new txs)
    pub ws_sender: Arc<RwLock<Option<tokio::sync::broadcast::Sender<rpc::WsEvent>>>>,
    /// In-memory tracker for minute attestations received via gossip.
    /// Used by block producer to build the presence_root bitfield.
    pub minute_tracker: MinuteAttestationTracker,
    // NOTE: epoch_attestation_accum, epoch_blocks_produced_accum, and
    // epoch_attested_set are now inside epoch_state: EpochState.
    /// INC-I-014: Fork tips we've already rejected (prevents re-requesting them).
    /// Bounded to prevent memory growth: entries removed after 1000 blocks.
    pub rejected_fork_tips: HashSet<Hash>,
    /// INC-I-143 F2 (security-hardening): hashes of sibling/descendant fork blocks
    /// that PASSED the producer-eligibility gate and were retained by the
    /// wedge-escape path. ONLY a block whose parent is in this set may be rerouted
    /// from the Orphan arm into the reorg graph — this prevents arbitrary
    /// cache-parented (unvalidated) blocks from seeding multi-block fabricated
    /// forks (AUDIT-P2-001/P2-002). Bounded by WEDGE_RETAINED_CAP to cap
    /// gossip-sourced descendant chain depth well below MAX_REORG_DEPTH.
    pub wedge_retained_tips: HashSet<Hash>,
    /// Height at which snap sync was applied (for validation mode selection).
    /// Blocks at or below this height use Light validation (no full tx verification)
    /// since the state was verified by state root quorum, not replayed.
    pub snap_sync_height: Option<u64>,
    /// INC-I-190 M4 (AUDIT-P1-501): a rebuild or snap derivation took a floor
    /// fallback, so our `producer_list` may differ from the fleet's. While set,
    /// gossip blocks are validated in `Light` mode. Lifetime is decided per
    /// boundary by `floor_window::on_boundary`. Node-local, never persisted,
    /// never a consensus input.
    pub floor_fallback_window: bool,
    /// Consecutive floor-pinned boundaries the open window has survived. Bounded
    /// by `FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES`. Node-local, never persisted.
    pub floor_fallback_boundaries: u8,
    /// INC-I-012: Rate limiter for sync requests processed per interval.
    /// Reset each production timer tick. Prevents sync I/O from starving production.
    pub sync_requests_this_interval: u32,
    /// Last height at which an auto-checkpoint was created.
    /// Used to determine when the next checkpoint is due.
    pub last_checkpoint_height: u64,
    /// TX announce-request: pending transaction hash announcements from peers.
    /// Batched and fetched periodically to amortize round-trips.
    pub pending_tx_announcements: HashMap<PeerId, Vec<Hash>>,
    /// Hard fork schedule — stops production when binary is too old for an activated fork.
    /// Built at compile time from known forks; checked every production tick.
    pub hardfork_schedule: updater::HardForkSchedule,

    /// Per-peer connection churn tracker (Fix 5a, 2026-04-15).
    /// Rolling window of recent connect/disconnect timestamps per peer. If a peer
    /// exceeds PEER_CHURN_MAX events in PEER_CHURN_WINDOW, we stop doing expensive
    /// work (status request, sync_manager mutation, bootstrap redial) for its
    /// connection events. Prevents flapping peers from starving the event loop and
    /// triggering spurious snap sync cascades. See 2026-04-15 incident.
    pub peer_churn: HashMap<PeerId, std::collections::VecDeque<Instant>>,

    /// Last tip height at which we ran the periodic block-store integrity check.
    /// `None` before first scan. Updated after each scan (Ok or Err).
    /// Phase 1 observability-only (INC-I-034 / M-Choice2).
    pub last_integrity_check_tip: Option<u64>,

    /// Recovery mode: when true, apply_block() and apply_snap_snapshot() silently
    /// drop all inbound state mutations (anti-poisoning gate for seed recovery).
    /// Toggled via enterRecoveryMode/exitRecoveryMode RPC. Non-persistent: cleared on restart.
    pub recovery_mode: Arc<AtomicBool>,

    /// Phase 2.1 Oracle M8 sunset flag — non-persistent, recomputed
    /// at each epoch boundary from `bond_snapshot` +
    /// `STRUCTURAL_PUBKEY_HASHES_HEX`. Wired into every
    /// `ValidationContext` construction via
    /// `ctx.oracle_sunset_triggered`. Pre-activation
    /// (`oracle_activation_height = u64::MAX`) this stays `false`
    /// because no attestations exist to drive a metric in the first
    /// place — sunset is only reachable once the activation height
    /// has been crossed (M4 admission opens) and producer migration
    /// drops structural share below 55%.
    ///
    /// Cleared on node restart. The next epoch boundary recomputes
    /// the metric and re-sets the flag if the chain is post-sunset
    /// — so a restart cannot bypass sunset for more than one epoch.
    pub oracle_sunset_triggered: Arc<AtomicBool>,

    /// AUDIT-P1-001: live snapshot of `(PublicKey, bond_weight)` for the
    /// currently active producer set. Shared with the mempool via
    /// `share_active_producers_weighted`; refreshed after every block
    /// apply. Wired into every mempool-side `ValidationContext` so the
    /// auth check at `validation/transaction.rs:242` sees the live set
    /// instead of an empty Vec — pre-fix that broke `PriceAttestation`
    /// admission at oracle activation.
    pub mempool_active_producers_snapshot: std::sync::Arc<std::sync::RwLock<Vec<(PublicKey, u64)>>>,

    /// INC-I-147 (INV-VALIDATION-001): live snapshot of
    /// `ProducerSet::pending_registration_keys()` — producers whose
    /// registration has been MINED but not yet flushed at the epoch
    /// boundary. Shared with the mempool via `share_pending_producer_keys`;
    /// refreshed alongside the active snapshot after every block apply.
    ///
    /// This is the exact value block validation passes to
    /// `with_pending_producer_keys` (`validation_checks.rs:291`). Pre-fix the
    /// mempool built every `ValidationContext` with
    /// `pending_producer_keys = Vec::new()`, so the duplicate check at
    /// `validation/registration.rs:173` was guaranteed-false and a second
    /// registration for a still-pending producer was admitted by every node
    /// and rejected by every node's block validation — poisoning the block of
    /// whichever producer selected it.
    pub mempool_pending_producer_keys: std::sync::Arc<std::sync::RwLock<Vec<PublicKey>>>,

    pub mempool_producer_holdings:
        std::sync::Arc<std::sync::RwLock<Vec<(PublicKey, mempool::ProducerHoldings)>>>,

    /// INC-I-055: Rolling health window for auto-checkpoint tagging.
    /// Tracks the last CHECKPOINT_HEALTH_WINDOW_SIZE health samples (true=healthy).
    /// A checkpoint is tagged healthy if ANY sample in the window was healthy,
    /// preventing all checkpoints from being marked unhealthy during transient
    /// peer disconnections. Updated every 30s in the periodic health diagnostic.
    pub health_window: std::collections::VecDeque<bool>,

    /// INC-I-204 M0: rolling wedge detector, fed by the 30s health tick. Its
    /// verdict is logged for the operator and read by no decision path.
    pub wedge_alarm: wedge_alarm::WedgeAlarm,

    /// INC-I-204 M0: last-seen `ReorgObservations`, so the plain network-side
    /// counters can be carried into Prometheus as counter deltas.
    pub reorg_scrape_state: crate::metrics::ReorgScrapeState,

    /// INC-I-049: Deferred attestation-triggered block fetch.
    /// Maps block_hash → (record_time, peers_asked, source_peer).
    /// When an attestation references an unknown block, we record it here
    /// but DON'T send the request immediately. run_periodic_tasks() checks
    /// entries >500ms old: if block_store still missing → send GetBlockByHash
    /// (genuine recovery); if block arrived via gossip → silently clear.
    /// Max 3 peers per hash, 30s TTL. Eliminates ~94% false-positive fetches
    /// where gossip delivers the block 1-2ms after the attestation.
    pub attest_fetch_tracker: HashMap<Hash, (Instant, u8, PeerId)>,

    /// INC-I-111: Counter for how many times the defi_health_inputs() scan
    /// has actually run (cache miss). Used by integration tests to verify
    /// the 30-second cache prevents redundant full UTXO scans.
    pub defi_health_refresh_counter: AtomicU64,

    /// INC-I-111: Cached defi_health_inputs result with TTL.
    /// `None` = never computed. `Some((value, computed_at))` = cached.
    /// Cache is checked at the top of the defi_health block in
    /// `run_periodic_tasks()`; refreshed only when stale (>30s).
    #[allow(clippy::type_complexity)]
    pub defi_health_cache: std::sync::Mutex<Option<((u64, Option<(Hash, u64)>), Instant)>>,
}

/// INC-I-055: Number of health samples to track in the rolling window.
/// At 30s per sample, 20 samples = 10 minutes of history.
pub const CHECKPOINT_HEALTH_WINDOW_SIZE: usize = 20;

/// Max connect+disconnect events per peer within PEER_CHURN_WINDOW before rate-limit kicks in.
pub const PEER_CHURN_MAX: usize = 5;
/// Rolling window for peer churn tracking.
pub const PEER_CHURN_WINDOW: Duration = Duration::from_secs(30);

impl Node {
    /// INC-I-111: How many times the defi_health_inputs() scan has actually
    /// run (cache misses). Zero at startup; incremented each time the cache
    /// is stale and a real scan executes.
    #[allow(dead_code)] // Used by integration tests (inc_i_111_defi_health_cache)
    pub fn defi_health_refresh_count(&self) -> u64 {
        self.defi_health_refresh_counter.load(Ordering::Relaxed)
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

    /// Compute the current fork_id from the hardfork schedule and genesis hash.
    pub fn current_fork_id(&self) -> crypto::Hash {
        let genesis_hash = self.params.genesis_hash;
        // Use a height of u64::MAX to include ALL scheduled forks in the fork_id.
        // This ensures that a node running the latest binary always produces the
        // same fork_id regardless of current chain height — the fork_id represents
        // "what forks does this binary know about", not "what forks are active now".
        // At the activation boundary, all upgraded nodes will have the same fork_id.
        self.hardfork_schedule.fork_id(&genesis_hash, u64::MAX)
    }

    /// Detect if there's a gap in historical blocks (e.g., from snap sync).
    /// Get the current chain tip height
    pub async fn best_height(&self) -> u64 {
        self.chain_state.read().await.best_height
    }

    /// Get the current chain tip hash
    #[allow(dead_code)]
    pub async fn best_hash(&self) -> crypto::Hash {
        self.chain_state.read().await.best_hash
    }

    /// Derive an attester's authority from the LOCAL ProducerSet (Seam A, [F1]).
    /// `None` = not a producer-set member (reject; never trust the wire's
    /// self-declared weight). `Some(w)` = member with locally-derived selection
    /// weight, which may be 0 for a fully-delegated active producer (INV-ATTEST-001,
    /// so attendance admission uses `.is_some()`, not `w > 0`).
    pub(crate) fn derive_attester_weight(
        &self,
        producers: &ProducerSet,
        attester: &PublicKey,
        height: u64,
    ) -> Option<u64> {
        producers.get_by_pubkey(attester).map(|p| {
            p.selection_weight_at(
                height,
                self.config
                    .network
                    .params()
                    .security_audit_activation_height,
            )
        })
    }

    /// Compute bond weights for scheduling from epoch snapshot (or UTXO fallback for epoch 0).
    ///
    /// Single source of truth for bond weights used by the scheduler.
    /// Called by production, validation, and gossip eligibility checks.
    /// Using the epoch-locked snapshot prevents mid-epoch add-bond TXs from
    /// changing total_bonds and causing scheduler divergence across nodes.
    pub async fn bond_weights_for_scheduling(
        &self,
        active_producers: Vec<PublicKey>,
    ) -> Vec<(PublicKey, u64)> {
        if self.epoch_state.bond_snapshot.is_empty() {
            // No snapshot yet (first epoch) — fall back to UTXO
            let utxo = self.utxo_set.read().await;
            active_producers
                .into_iter()
                .map(|pk| {
                    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes());
                    let count = utxo
                        .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                        .max(1) as u64;
                    (pk, count)
                })
                .collect()
        } else {
            // Use epoch snapshot — deterministic across all nodes
            active_producers
                .into_iter()
                .map(|pk| {
                    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes());
                    let count = self
                        .epoch_state
                        .bond_snapshot
                        .get(&pubkey_hash)
                        .copied()
                        .unwrap_or(1);
                    (pk, count)
                })
                .collect()
        }
    }

    /// AUDIT-P1-001: refresh the mempool's shared active-producer
    /// weighted snapshot from the live ProducerSet + epoch_state bond
    /// snapshot. Called after every successful apply_block commit so
    /// the mempool admission path sees the same producer set the
    /// block-validation path sees.
    ///
    /// Pre-fix the mempool built every ValidationContext with
    /// `active_producers = Vec::new()`, so the auth check at
    /// `validation/transaction.rs:242`
    /// (`ctx.active_producers.contains(&signer_pubkey)`) was
    /// guaranteed-false for every PriceAttestation. With
    /// oracle_activation_height=u64::MAX today this is masked, but
    /// becomes a liveness blocker the instant a real height is pinned.
    /// INC-I-147: the same refresh also republishes
    /// `pending_registration_keys()` — read from the SAME `producer_set` guard
    /// as the active list, so the two snapshots can never be taken from
    /// different states of the ProducerSet.
    pub async fn refresh_mempool_producer_snapshot(&self, height: u64) {
        let producers = self.producer_set.read().await;
        let active: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        let pending = producers.pending_registration_keys();
        let holdings = holdings_of_every_producer(&producers);
        drop(producers);
        let weighted = self.bond_weights_for_scheduling(active).await;
        if let Ok(mut guard) = self.mempool_active_producers_snapshot.write() {
            *guard = weighted;
        }
        if let Ok(mut guard) = self.mempool_pending_producer_keys.write() {
            *guard = pending;
        }
        if let Ok(mut guard) = self.mempool_producer_holdings.write() {
            *guard = holdings;
        }
    }

    // Checkpoint methods removed — snap sync is handled internally by SyncManager.

    pub fn set_maintainer_state(&mut self, state: Arc<RwLock<storage::MaintainerState>>) {
        self.maintainer_state = Some(state);
    }

    /// Shutdown the node
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down node...");

        // Set shutdown flag
        *self.shutdown.write().await = true;

        info!("Node shutdown complete");
        Ok(())
    }
}

// Note: The weighted presence reward system uses automatic EpochReward
// transactions distributed at epoch boundaries. Validation is in
// crates/core/src/validation.rs and tests in crates/core/src/rewards.rs.
