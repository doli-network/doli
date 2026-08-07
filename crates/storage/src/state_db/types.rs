//! State database types, constants, and struct definitions

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;

use crypto::Hash;
use serde::{Deserialize, Serialize};

use crate::utxo::{Outpoint, UtxoEntry};

/// Reverse diff for a single block — enough to undo all state changes.
///
/// Stored in `cf_undo` keyed by block height. Enables O(rollback_depth) reorgs
/// instead of O(chain_height) rebuild-from-genesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoData {
    /// UTXOs that were spent by this block (restore on rollback).
    pub spent_utxos: Vec<(Outpoint, UtxoEntry)>,
    /// UTXOs that were created by this block (delete on rollback).
    pub created_utxos: Vec<Outpoint>,
    /// Serialized ProducerSet snapshot BEFORE this block was applied.
    /// Producer state is complex (bonds, pending updates, epoch boundaries)
    /// so we snapshot instead of tracking individual deltas.
    pub producer_snapshot: Vec<u8>,
    /// Serialized EpochState snapshot BEFORE this block was applied.
    /// Enables O(1) rollback of scheduler state instead of rebuilding from blocks.
    /// None for blocks created before this field was added (backward compat).
    #[serde(default)]
    pub epoch_state_snapshot: Option<Vec<u8>>,
    /// Legacy field — kept for backward compat deserialization of existing undo data.
    /// Chain commitment is now computed via periodic full scan, not incrementally.
    #[serde(default)]
    pub chain_commitment: Option<[u8; 32]>,
}

/// INC-I-104 M0: hard cap on total memtable budget across all CFs.
/// Shared between `open()` (sets `db_write_buffer_size`) and `metrics()`.
/// Per Failure Analyst C-002: must be >= 32 MB for snap-sync atomic_replace.
pub(super) const DB_WRITE_BUFFER_SIZE_BYTES: u64 = 64 * 1024 * 1024;

// Column family names
pub(super) const CF_UTXO: &str = "cf_utxo";
pub(super) const CF_UTXO_BY_PUBKEY: &str = "cf_utxo_by_pubkey";
pub(super) const CF_PRODUCERS: &str = "cf_producers";
pub(super) const CF_EXIT_HISTORY: &str = "cf_exit_history";
pub(super) const CF_META: &str = "cf_meta";
pub(super) const CF_UNDO: &str = "cf_undo";
/// Phase 1 of UTXO storage consolidation: unique ID index for NFT/Pool/Asset
/// uniqueness checks. Mirrors utxo_store's `unique_id` CF.
/// Key: prefix(1B) + id(32B) -> empty. See `specs/utxo-storage-architecture.md`.
pub(super) const CF_UNIQUE_ID: &str = "cf_unique_id";

// Meta keys
pub(super) const META_CHAIN_STATE: &[u8] = b"chain_state";
pub(super) const META_PENDING_UPDATES: &[u8] = b"pending_updates";
pub(super) const META_LAST_APPLIED: &[u8] = b"last_applied";
pub(super) const META_EPOCH_PRODUCER_LIST: &[u8] = b"epoch_producer_list";
pub(super) const META_ACTIVE_PRODUCTION_LIST: &[u8] = b"active_production_list";
pub(super) const META_EPOCH_ATTESTED_SET: &[u8] = b"epoch_attested_set";
pub(super) const META_EPOCH_ATTESTATION_ACCUM: &[u8] = b"epoch_attestation_accum";
pub(super) const META_EPOCH_BLOCKS_PRODUCED: &[u8] = b"epoch_blocks_produced";
pub(super) const META_EPOCH_BOND_SNAPSHOT: &[u8] = b"epoch_bond_snapshot";
pub(super) const META_EPOCH_STATE: &[u8] = b"epoch_state";
pub(super) const META_EPOCH_STATE_VERSION: &[u8] = b"epoch_state_version";
pub(super) const META_CHAIN_COMMITMENT: &[u8] = b"chain_commitment";
pub(super) const META_CHAIN_COMMITMENT_TIP: &[u8] = b"chain_commitment_tip";
/// D.3 oracle sunset gradient state (warning/halt epoch tracking).
/// Persisted as bincode-serialized `OracleSunsetState`. Local
/// bookkeeping — NOT part of the consensus state root.
pub(super) const META_ORACLE_SUNSET_STATE: &[u8] = b"oracle_sunset_state";
/// AUDIT-P2-001: cached `last_update_height` for the oracle status RPC.
/// Written at every successful `OraclePrice` UTXO insert in the
/// aggregator; read by `getOracleStatus` to avoid an unbounded full-
/// UTXO-set scan on every unauthenticated RPC call. Stored as 8-byte
/// little-endian u64. NOT part of the consensus state root.
pub(super) const META_ORACLE_LAST_UPDATE_HEIGHT: &[u8] = b"oracle_last_update_height";
/// AUDIT-P1-001 (INC-I-156): set immediately BEFORE a destructive
/// rebuild-from-genesis wipes `cf_utxo`, deleted only after the trailing
/// `atomic_replace` succeeds. Its presence at any later moment means the wipe
/// committed but the replay did not finish — the durable ledger is a truncated
/// subset of the chain the persisted `chain_state` claims.
///
/// Stored as 16 bytes: target height (8B LE) ‖ unix start time (8B LE).
/// NOT part of the consensus state root. Deliberately in `CF_META`, which
/// `atomic_replace` does not iterate-delete (`writes.rs:181-186`), so the
/// marker survives the very operation that clears it explicitly — and survives
/// a `systemctl restart` in the middle of the replay window.
pub(super) const META_REBUILD_IN_PROGRESS: &[u8] = b"rebuild_in_progress";

/// Unified state database wrapping a single RocksDB instance.
pub struct StateDb {
    pub(super) db: rocksdb::DB,
    pub(super) utxo_count: AtomicU64,
    /// Shared LRU block cache referenced by every CF. Held on the struct so
    /// `metrics()` can query its real usage via `Cache::get_usage()` instead
    /// of summing per-CF property reads (INC-I-106 root-cause fix).
    pub(super) block_cache: rocksdb::Cache,
    /// Configured capacity of `block_cache` in bytes.
    pub(super) block_cache_capacity_bytes: u64,
}

/// Atomic write batch for a single block application.
///
/// All mutations within a block go into this batch. On `commit()`,
/// the entire batch is written atomically. If the batch is dropped
/// without committing, no changes are persisted.
///
/// Phase 3: BlockBatch is now the **sole** UTXO mutation path during
/// `apply_block`. All reads during block application use the overlay
/// methods (`get_utxo`, `contains_utxo`, `get_utxos_by_pubkey`,
/// `has_unique_id_check`) which check pending state first, then fall
/// through to committed state_db.
pub struct BlockBatch<'a> {
    pub(super) db: &'a StateDb,
    pub(super) batch: rocksdb::WriteBatch,
    pub(super) utxo_delta: i64,
    /// UTXOs added in this batch but not yet committed to DB.
    /// Needed for same-block-spend: TX2 spending an output created by TX1
    /// in the same block won't find it via db.get() (not committed yet).
    pub(super) pending_utxos: HashMap<Outpoint, UtxoEntry>,
    /// Outpoints removed in this batch (to avoid returning spent UTXOs from pending).
    pub(super) spent_in_batch: Vec<Outpoint>,
    /// Unique IDs added in this batch but not yet committed to disk.
    /// Enables same-block NFT/Pool/Asset uniqueness checks without
    /// reading from cf_unique_id (which hasn't been committed yet).
    /// Key: (output_type_discriminant, unique_id_hash).
    /// Phase 1 of UTXO storage consolidation (specs/utxo-storage-architecture.md).
    pub(super) pending_unique_ids: HashSet<(u8, [u8; 32])>,
    /// Unique IDs removed in this batch but not yet committed to disk.
    /// Phase 3: `has_unique_id_check` must return false for IDs that were
    /// spent in the current block, even if they still exist on disk.
    pub(super) removed_unique_ids: HashSet<(u8, [u8; 32])>,
}

/// The consistency canary — stored inside the same WriteBatch as state.
/// If this key exists and matches the chain_state, the DB is consistent.
#[derive(Debug, Clone)]
pub struct LastApplied {
    pub height: u64,
    pub hash: Hash,
    pub slot: u32,
}

impl LastApplied {
    pub(super) const SIZE: usize = 44; // 8 + 32 + 4

    pub(super) fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..8].copy_from_slice(&self.height.to_le_bytes());
        buf[8..40].copy_from_slice(self.hash.as_bytes());
        buf[40..44].copy_from_slice(&self.slot.to_le_bytes());
        buf
    }

    pub(super) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::SIZE {
            return None;
        }
        let height = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let hash = Hash::from_bytes(bytes[8..40].try_into().ok()?);
        let slot = u32::from_le_bytes(bytes[40..44].try_into().ok()?);
        Some(Self { height, hash, slot })
    }
}
