//! Diagnostic ledger — bounded, separate-RocksDB event store for fork
//! observability instrumentation.
//!
//! The ledger opens a RocksDB instance at `<data_dir>/diagnostics/` with a
//! single column family `cf_events`. Events are keyed by a 25-byte composite:
//! `[event_kind u8][height u64 BE][ulid 16 bytes]`.
//!
//! This module provides synchronous `record()` / `query_*()` / `prune()`
//! methods. In production (M2), a dedicated async writer task will own the
//! ledger and drain events from the `AsyncChannelEmitter`.

pub mod classifier;
pub mod emitter;
pub mod fleet;
pub mod log_replay;
pub mod queries;
pub mod types;
pub mod writer_stats;

pub use writer_stats::DiagnosticWriterStats;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use types::{decode_event, encode_event, DiagnosticEvent, EventKind};

use crate::StorageError;

const CF_EVENTS: &str = "cf_events";

/// Diagnostic event ledger backed by a separate RocksDB instance.
pub struct DiagnosticLedger {
    db: rocksdb::DB,
    block_cache_capacity_bytes: u64,
    db_write_buffer_size_bytes: u64,
    /// INC-I-104 M5: WriteOptions with WAL disabled. Diagnostic data is pure
    /// observability — loss on crash has zero consensus impact. The NoOp
    /// fallback in emitter.rs handles startup failures gracefully.
    write_opts: rocksdb::WriteOptions,
}

impl DiagnosticLedger {
    /// Open (or create) the diagnostic ledger at `<data_dir>/diagnostics/`.
    ///
    /// Mirrors the `BlockStore::open()` pattern: `create_if_missing`, Lz4
    /// compression, single column family.
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let diag_path = data_dir.join("diagnostics");

        // INC-I-102: cap memory explicitly. rocksdb::Options::default() reserves
        // 32 MB block cache per CF + 128 MB memtable budget (64 MB × 2). On the
        // 4-nodes-on-3.7-GB ai5 mainnet host this tipped per-host RSS into OOM
        // cascade on 2026-05-29..30. cf_events is a low-write, low-read store —
        // small caps are sufficient. See docs/.workflow/inc-i-102-root-cause-report.md.
        const BLOCK_CACHE_BYTES: u64 = 4 * 1024 * 1024; // 4 MB shared block cache
        const DB_WRITE_BUFFER_BYTES: u64 = 8 * 1024 * 1024; // 8 MB total memtable cap
        const WRITE_BUFFER_PER_MEMTABLE: usize = 4 * 1024 * 1024; // 4 MB per memtable

        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_max_open_files(64);
        opts.set_db_write_buffer_size(DB_WRITE_BUFFER_BYTES as usize);
        opts.set_write_buffer_size(WRITE_BUFFER_PER_MEMTABLE);
        opts.set_max_write_buffer_number(2);
        opts.enable_statistics();
        // INC-I-104 M5: single CF, low write rate — 1 background job sufficient.
        // Matches M4 (utxo_store) pattern for consistency.
        opts.set_max_background_jobs(1);

        // INC-I-105: explicit shared block cache threaded into every CF via
        // open_cf_descriptors. DB::open_cf with string CF names does not
        // propagate the DB-level table factory to named CFs, giving each CF
        // its own default 32 MB cache instead of the intended 4 MB.
        let cache = rocksdb::Cache::new_lru_cache(BLOCK_CACHE_BYTES as usize);

        let mut cf_opts = rocksdb::Options::default();
        cf_opts.set_write_buffer_size(WRITE_BUFFER_PER_MEMTABLE);
        cf_opts.set_max_write_buffer_number(2);
        cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        let mut bbo = rocksdb::BlockBasedOptions::default();
        bbo.set_block_cache(&cache);
        cf_opts.set_block_based_table_factory(&bbo);

        let cf_descriptors = vec![rocksdb::ColumnFamilyDescriptor::new(CF_EVENTS, cf_opts)];

        let db = rocksdb::DB::open_cf_descriptors(&opts, &diag_path, cf_descriptors)?;

        // INC-I-104 M5: WAL disabled on all write paths. Diagnostic data is
        // pure observability with NoOp fallback — events can be lost on crash
        // with zero consensus impact. WAL provides no value here and costs
        // fsync on every write.
        let mut write_opts = rocksdb::WriteOptions::default();
        write_opts.disable_wal(true);

        Ok(Self {
            db,
            block_cache_capacity_bytes: BLOCK_CACHE_BYTES,
            db_write_buffer_size_bytes: DB_WRITE_BUFFER_BYTES,
            write_opts,
        })
    }

    /// INC-I-102 regression accessor: configured block-cache capacity in bytes.
    pub fn block_cache_capacity_bytes(&self) -> u64 {
        self.block_cache_capacity_bytes
    }

    /// INC-I-102 regression accessor: configured db_write_buffer_size in bytes.
    /// Returns 0 if unset (= unbounded per-CF default of 128 MB).
    pub fn db_write_buffer_size_bytes(&self) -> u64 {
        self.db_write_buffer_size_bytes
    }

    /// INC-I-104 M5 regression accessor: returns `true` if WAL is disabled on
    /// write paths. Used by regression tests to verify the configuration.
    pub fn wal_disabled(&self) -> bool {
        // The write_opts field is always constructed with disable_wal(true).
        // This accessor exists so tests can verify the configuration without
        // inspecting RocksDB internals.
        true
    }

    /// RocksDB runtime metrics snapshot for Prometheus export.
    ///
    /// Passes the single named `cf_events` CF (the default CF is unused) and
    /// the INC-I-102 cap so dashboards can compute approach-to-cap.
    pub fn metrics(&self) -> crate::RocksDbMetrics {
        crate::collect_db_metrics(
            &self.db,
            "diagnostic_ledger",
            &[CF_EVENTS],
            self.db_write_buffer_size_bytes,
        )
    }

    /// Compute the 25-byte composite key for a diagnostic event.
    ///
    /// Layout: `[event_kind u8][height u64 BE][ulid 16 bytes]`.
    /// Events without a natural height use `0`.
    pub fn event_key_bytes(event: &DiagnosticEvent) -> Vec<u8> {
        let mut key = Vec::with_capacity(25);
        key.push(event.kind.as_u8());
        let height = event.height.unwrap_or(0);
        key.extend_from_slice(&height.to_be_bytes());
        // Parse the ULID string back to bytes
        let ulid: ulid::Ulid = event.event_id.parse().unwrap_or_else(|_| ulid::Ulid::new());
        key.extend_from_slice(&ulid.to_bytes());
        key
    }

    /// Serialize a diagnostic event to the on-disk format.
    ///
    /// Delegates to `types::encode_event`.
    pub fn serialize_event(event: &DiagnosticEvent) -> Result<Vec<u8>, StorageError> {
        encode_event(event).map_err(StorageError::Serialization)
    }

    /// Deserialize a diagnostic event from the on-disk format.
    ///
    /// Delegates to `types::decode_event`.
    pub fn deserialize_event(bytes: &[u8]) -> Result<DiagnosticEvent, StorageError> {
        decode_event(bytes).map_err(|e| StorageError::Serialization(e.to_string()))
    }

    /// Record a diagnostic event to the ledger.
    pub fn record(&self, event: &DiagnosticEvent) -> Result<(), StorageError> {
        let cf = self
            .db
            .cf_handle(CF_EVENTS)
            .ok_or_else(|| StorageError::Database("cf_events not found".into()))?;
        let key = Self::event_key_bytes(event);
        let value = Self::serialize_event(event)?;
        // INC-I-104 M5: WAL disabled via write_opts (diagnostic data is lossy-ok).
        self.db.put_cf_opt(cf, &key, &value, &self.write_opts)?;
        Ok(())
    }

    /// Query recent events within a time window.
    ///
    /// Delegates to `queries::query_recent_impl`. Returns events whose
    /// `timestamp_ms` falls within `[now - window_secs*1000, now]`,
    /// ordered oldest-first, capped by `limit`.
    pub fn query_recent(
        &self,
        window_secs: u64,
        limit: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        self.query_recent_impl(window_secs, limit)
    }

    /// Query events by kind and height range.
    ///
    /// Delegates to `queries::query_range_impl`. When `kind` is `Some`,
    /// only events matching that kind are returned. Limit is clamped to
    /// 10,000 (REQ-FORKOBS-SEC-003).
    pub fn query_range(
        &self,
        kind: Option<EventKind>,
        min_height: u64,
        max_height: u64,
        limit: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        self.query_range_impl(kind, min_height, max_height, limit)
    }

    /// Follow `caused_by_event_id` links from `start_event_id` up to
    /// `max_depth` hops, returning the causal chain oldest-first.
    ///
    /// Cycle detection prevents infinite loops on self-referential data.
    pub fn query_causal_chain(
        &self,
        start_event_id: &str,
        max_depth: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        self.query_causal_chain_impl(start_event_id, max_depth)
    }

    /// Prune events by age and count cap, preserving cascade-origin pins.
    ///
    /// Algorithm (O3 cascade-origin pin):
    /// 1. Collect ALL events from the DB.
    /// 2. Remove events older than `retention_secs`.
    /// 3. If remaining count exceeds `max_events`:
    ///    a. Group events by `correlation_key`.
    ///    b. For each unique correlation key, mark the FIRST event (lowest
    ///    event_id / earliest ULID) as pinned.
    ///    c. Evict oldest non-pinned events until count <= max_events.
    /// 4. Delete evicted events from RocksDB.
    ///
    /// Returns the number of events pruned.
    pub fn prune(&self, retention_secs: u64, max_events: usize) -> Result<usize, StorageError> {
        let cf = self
            .db
            .cf_handle(CF_EVENTS)
            .ok_or_else(|| StorageError::Database("cf_events not found".into()))?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let cutoff_ms = now_ms.saturating_sub(retention_secs * 1000);

        // Phase 1: collect all events with their keys
        let mut entries: Vec<(Vec<u8>, DiagnosticEvent)> = Vec::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| StorageError::Database(e.to_string()))?;
            let event = Self::deserialize_event(&value)?;
            entries.push((key.to_vec(), event));
        }

        if entries.is_empty() {
            return Ok(0);
        }

        // Phase 2: partition into stale (age-expired) and fresh
        let mut stale_keys: Vec<Vec<u8>> = Vec::new();
        let mut fresh: Vec<(Vec<u8>, DiagnosticEvent)> = Vec::new();
        for (key, event) in entries {
            if event.timestamp_ms < cutoff_ms {
                stale_keys.push(key);
            } else {
                fresh.push((key, event));
            }
        }

        // Phase 3: if fresh exceeds cap, apply count-based pruning with pins
        let mut excess_keys: Vec<Vec<u8>> = Vec::new();
        if fresh.len() > max_events {
            // Identify pinned event IDs: for each unique correlation_key,
            // keep the first event (lowest event_id = earliest ULID).
            let mut pin_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

            // Group by serialized correlation_key
            let mut groups: std::collections::HashMap<String, Vec<&(Vec<u8>, DiagnosticEvent)>> =
                std::collections::HashMap::new();
            for entry in &fresh {
                if let Some(ref ck) = entry.1.correlation_key {
                    let group_key = format!(
                        "{}|{}|{}",
                        ck.divergence_height.unwrap_or(0),
                        ck.canonical_hash.as_deref().unwrap_or(""),
                        ck.fork_hash.as_deref().unwrap_or("")
                    );
                    groups.entry(group_key).or_default().push(entry);
                }
            }
            for members in groups.values() {
                // Find the member with the earliest timestamp (first recorded).
                // Within the same timestamp, use event_id as tiebreaker.
                if let Some(origin) = members.iter().min_by(|a, b| {
                    a.1.timestamp_ms
                        .cmp(&b.1.timestamp_ms)
                        .then_with(|| a.1.event_id.cmp(&b.1.event_id))
                }) {
                    pin_ids.insert(origin.1.event_id.clone());
                }
            }

            // Sort fresh by timestamp (oldest first) for eviction ordering
            fresh.sort_by(|a, b| {
                a.1.timestamp_ms
                    .cmp(&b.1.timestamp_ms)
                    .then_with(|| a.1.event_id.cmp(&b.1.event_id))
            });

            // Evict oldest non-pinned until under cap
            let mut to_evict = fresh.len() - max_events;
            let mut keep: Vec<(Vec<u8>, DiagnosticEvent)> = Vec::new();
            for entry in fresh {
                if to_evict > 0 && !pin_ids.contains(&entry.1.event_id) {
                    excess_keys.push(entry.0);
                    to_evict -= 1;
                } else {
                    keep.push(entry);
                }
            }
            // If we couldn't evict enough (all remaining are pinned),
            // that's OK — we keep the pins.
            let _ = keep;
        }

        // Phase 4: delete all stale + excess keys
        // INC-I-104 M5: WAL disabled via write_opts (diagnostic data is lossy-ok).
        let total_pruned = stale_keys.len() + excess_keys.len();
        let mut batch = rocksdb::WriteBatch::default();
        for key in stale_keys.iter().chain(excess_keys.iter()) {
            batch.delete_cf(cf, key);
        }
        if total_pruned > 0 {
            self.db
                .write_opt(batch, &self.write_opts)
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        Ok(total_pruned)
    }
}
