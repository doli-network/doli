//! State database types, constants, and struct definitions

use std::collections::HashMap;
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
}

// Column family names
pub(super) const CF_UTXO: &str = "cf_utxo";
pub(super) const CF_UTXO_BY_PUBKEY: &str = "cf_utxo_by_pubkey";
pub(super) const CF_PRODUCERS: &str = "cf_producers";
pub(super) const CF_EXIT_HISTORY: &str = "cf_exit_history";
pub(super) const CF_META: &str = "cf_meta";
pub(super) const CF_UNDO: &str = "cf_undo";

// Meta keys
pub(super) const META_CHAIN_STATE: &[u8] = b"chain_state";
pub(super) const META_PENDING_UPDATES: &[u8] = b"pending_updates";
pub(super) const META_LAST_APPLIED: &[u8] = b"last_applied";

/// Unified state database wrapping a single RocksDB instance.
pub struct StateDb {
    pub(super) db: rocksdb::DB,
    pub(super) utxo_count: AtomicU64,
}

/// Atomic write batch for a single block application.
///
/// All mutations within a block go into this batch. On `commit()`,
/// the entire batch is written atomically. If the batch is dropped
/// without committing, no changes are persisted.
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
