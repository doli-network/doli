//! # doli-storage
//!
//! Storage layer for the DOLI protocol.
//!
//! This crate provides persistent storage for all blockchain data using
//! RocksDB as the underlying key-value store. It manages blocks, the UTXO
//! set, chain state, and producer registrations.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                      Storage Layer                           │
//! │                                                              │
//! │  ┌─────────────┐  ┌──────────────────────────────────────┐   │
//! │  │ BlockStore  │  │            StateDb                    │   │
//! │  │  (RocksDB)  │  │         (RocksDB)                    │   │
//! │  │             │  │                                      │   │
//! │  │ - headers   │  │ cf_utxo          Outpoint→UtxoEntry  │   │
//! │  │ - bodies    │  │ cf_utxo_by_pubkey  secondary index   │   │
//! │  │ - height_idx│  │ cf_producers     pubkey→ProducerInfo │   │
//! │  │ - slot_idx  │  │ cf_exit_history  pubkey→exit_height  │   │
//! │  │             │  │ cf_meta          chain_state, etc.    │   │
//! │  └─────────────┘  └──────────────────────────────────────┘   │
//! │                                                              │
//! │  One atomic WriteBatch per block — all state changes         │
//! │  (UTXOs, producers, chain_state) committed together.         │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Components
//!
//! ### BlockStore
//!
//! Stores complete blocks with efficient indexing:
//!
//! - Block headers indexed by hash and height
//! - Block bodies (transactions) stored separately
//! - Orphan block handling for reorganizations
//!
//! ### UtxoSet
//!
//! Manages the set of unspent transaction outputs:
//!
//! - O(1) lookup by `(txid, output_index)`
//! - Atomic batch updates during block processing
//! - Rollback support for chain reorganizations
//!
//! ### ChainState
//!
//! Tracks the current consensus state:
//!
//! - Current chain tip (best block hash)
//! - Block height
//! - Active producer set
//! - Epoch and era boundaries
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use storage::{BlockStore, UtxoSet, ChainState};
//! use crypto::Hash;
//! use std::path::Path;
//!
//! // Initialize stores
//! let blocks = BlockStore::open(Path::new("/var/lib/doli/blocks")).unwrap();
//! let utxos = UtxoSet::new();
//! let state = ChainState::new(Hash::ZERO);
//!
//! // Query the current chain tip
//! println!("Chain height: {}", state.best_height);
//! ```
//!
//! ## Performance
//!
//! - Uses RocksDB column families for data isolation
//! - Write batches for atomic multi-key updates
//! - LRU caching for frequently accessed data
//! - Bloom filters for negative lookup optimization
//!
//! ## Data Durability
//!
//! - WAL (Write-Ahead Log) ensures crash recovery
//! - Checksums detect data corruption
//! - Periodic compaction maintains read performance

pub mod archiver;
pub mod block_store;
pub mod chain_state;
pub mod maintainer;
pub mod producer;
pub mod snapshot;
pub mod state_db;
pub mod update;
pub mod utxo;
pub mod utxo_rocks;

pub use block_store::BlockStore;
pub use chain_state::ChainState;
pub use maintainer::MaintainerState;
#[allow(deprecated)]
pub use producer::{
    producer_weight, producer_weight_for_network, producer_weight_precise,
    producer_weight_precise_for_network, total_weight, total_weight_for_network,
    weighted_veto_threshold, weighted_veto_threshold_for_network, ActivityStatus,
    PendingProducerUpdate, ProducerInfo, ProducerSet, ProducerStatus, StoredBondEntry,
    ACTIVATION_DELAY, BLOCKS_PER_MONTH, BLOCKS_PER_YEAR, EXIT_HISTORY_RETENTION,
    INACTIVITY_THRESHOLD, MAX_WEIGHT, MIN_WEIGHT, REACTIVATION_THRESHOLD, VETO_BOND_AMOUNT,
    VETO_THRESHOLD_PERCENT,
};
pub use snapshot::{compute_state_root, StateSnapshot};
pub use state_db::{BlockBatch, LastApplied, StateDb, UndoData};
pub use update::UpdateState;
pub use utxo::{InMemoryUtxoStore, Outpoint, UtxoEntry, UtxoSet};
pub use utxo_rocks::RocksDbUtxoStore;

use std::path::Path;
use thiserror::Error;

/// Storage layer errors.
///
/// All storage operations return `Result<T, StorageError>` to handle
/// database failures, serialization issues, and missing data gracefully.
#[derive(Debug, Error)]
pub enum StorageError {
    /// RocksDB internal error (corruption, I/O failure, etc.)
    #[error("database error: {0}")]
    Database(String),

    /// Failed to serialize or deserialize data
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Requested item does not exist in the database
    #[error("not found: {0}")]
    NotFound(String),

    /// Attempted to insert a duplicate entry
    #[error("already exists: {0}")]
    AlreadyExists(String),

    /// File system I/O error
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<rocksdb::Error> for StorageError {
    fn from(e: rocksdb::Error) -> Self {
        StorageError::Database(e.to_string())
    }
}

/// Open a RocksDB database at the given path.
///
/// Creates the database directory if it doesn't exist. The database
/// is configured with sensible defaults for blockchain storage:
///
/// - Automatic creation if missing
/// - Limited open file handles (256) to avoid resource exhaustion
///
/// # Errors
///
/// Returns `StorageError::Database` if the database cannot be opened
/// (permissions, corruption, or lock contention).
///
/// # Example
///
/// ```rust,no_run
/// use storage::open_db;
/// use std::path::Path;
///
/// let db = open_db(Path::new("/var/lib/doli/data"))?;
/// # Ok::<(), storage::StorageError>(())
/// ```
pub fn open_db(path: &Path) -> Result<rocksdb::DB, StorageError> {
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.set_max_open_files(256);
    rocksdb::DB::open(&opts, path).map_err(StorageError::from)
}
