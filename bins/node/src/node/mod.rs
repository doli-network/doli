//! Node implementation
//!
//! Integrates all DOLI components: storage, networking, RPC, mempool, and sync.
// v0.2.1-test: upgrade pipeline validation

mod apply_block;
mod block_handling;
mod event_loop;
mod fork_recovery;
mod genesis;
mod init;
mod network_events;
mod periodic;
mod production;
mod rewards;
mod rollback;
mod startup;
mod tx_announcements;
mod validation_checks;

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
    DELEGATE_REWARD_PCT, UNBONDING_PERIOD,
};
// WeightedRewardCalculator removed — replaced by attestation-qualified bond-weighted distribution
use doli_core::tpop::calibration::VdfCalibrator;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{RegistrationData, TxType};
use doli_core::types::UNITS_PER_COIN;
use doli_core::validation;
use doli_core::validation::ValidationMode;
use doli_core::DeterministicScheduler;
use doli_core::{
    attestation_minute, decode_attestation_bitfield, encode_attestation_bitfield, AdaptiveGossip,
    Attestation, Block, BlockHeader, MinuteAttestationTracker, Network, ProducerAnnouncement,
    ProducerGSet, Transaction,
};
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
    /// Last time we checked for production opportunity
    pub _last_production_check: Instant,
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
    /// Last time we triggered a forced resync (cooldown to prevent loops)
    pub last_resync_time: Option<Instant>,
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
    /// Consecutive slots where production was blocked due to fork detection
    /// (AheadOfPeers, SyncFailures, ChainMismatch). Triggers auto-resync when threshold exceeded.
    pub consecutive_fork_blocks: u32,
    /// Number of shallow fork rollbacks performed since last successful sync.
    /// Capped at MAX_SHALLOW_ROLLBACKS to prevent rolling back the entire chain.
    pub shallow_rollback_count: u32,
    /// Cumulative rollback depth since last successful block application.
    /// Tracks how far we've rolled back in total. Capped at MAX_CUMULATIVE_ROLLBACK (50)
    /// to prevent cascading rollbacks from reaching genesis.
    pub cumulative_rollback_depth: u32,
    /// Slots for which we've seen a block via gossip (not yet applied to block_store).
    /// Used by rank 1 to avoid producing a competing block when rank 0 already produced
    /// but the block hasn't been applied to disk yet. Cleaned periodically.
    pub seen_blocks_for_slot: std::collections::HashSet<u32>,
    /// Producers excluded from round-robin for missing their slot.
    /// Re-included when they attest. Rebuilt from block store at startup.
    /// Updated live in post_commit_actions on every block.
    pub excluded_producers: HashSet<PublicKey>,
    /// Epoch-locked bond snapshot: {pubkey_hash → bond_count}.
    /// Computed once at each epoch boundary from the UTXO set.
    /// Used by scheduler (validation + production) for the entire epoch.
    /// Prevents mid-epoch add-bond from changing total_bonds and diverging schedulers.
    pub epoch_bond_snapshot: HashMap<Hash, u64>,
    /// The epoch number for which the bond snapshot was taken.
    pub epoch_bond_snapshot_epoch: u64,
    /// Cached DeterministicScheduler (epoch, producer_count, total_bonds, scheduler)
    /// Rebuilt when epoch changes OR active producer set changes (new registrations, exits, slashing).
    pub cached_scheduler: Option<(u64, usize, u64, DeterministicScheduler)>,
    /// Our computed tier (1, 2, or 3). Recomputed at each epoch boundary.
    pub our_tier: u8,
    /// Last epoch for which we computed our tier (to detect epoch boundaries).
    pub last_tier_epoch: Option<u64>,
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
    /// INC-I-014: Fork tips we've already rejected (prevents re-requesting them).
    /// Bounded to prevent memory growth: entries removed after 1000 blocks.
    pub rejected_fork_tips: HashSet<Hash>,
    /// Height at which snap sync was applied (for validation mode selection).
    /// Blocks at or below this height use Light validation (no full tx verification)
    /// since the state was verified by state root quorum, not replayed.
    pub snap_sync_height: Option<u64>,
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
}

impl Node {
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

    /// Detect if there's a gap in historical blocks (e.g., from snap sync).
    /// Get the current chain tip height
    pub async fn best_height(&self) -> u64 {
        self.chain_state.read().await.best_height
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
        if self.epoch_bond_snapshot.is_empty() {
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
                        .epoch_bond_snapshot
                        .get(&pubkey_hash)
                        .copied()
                        .unwrap_or(1);
                    (pk, count)
                })
                .collect()
        }
    }

    // Checkpoint methods removed — snap sync is handled internally by SyncManager.

    pub fn set_maintainer_state(&mut self, state: Arc<RwLock<storage::MaintainerState>>) {
        self.maintainer_state = Some(state);
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

    /// Save all node state — now a no-op.
    ///
    /// All state persistence happens atomically via StateDb WriteBatch.
    /// apply_block() commits chain_state + producers + UTXOs in one batch.
    /// Reorg/rollback/snap_sync use atomic_replace().
    pub async fn save_state(&self) -> Result<()> {
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
