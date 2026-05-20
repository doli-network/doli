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

pub mod emitter;
pub mod types;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use types::{decode_event, encode_event, DiagnosticEvent, EventKind};

use crate::StorageError;

const CF_EVENTS: &str = "cf_events";

/// Diagnostic event ledger backed by a separate RocksDB instance.
pub struct DiagnosticLedger {
    db: rocksdb::DB,
}

impl DiagnosticLedger {
    /// Open (or create) the diagnostic ledger at `<data_dir>/diagnostics/`.
    ///
    /// Mirrors the `BlockStore::open()` pattern: `create_if_missing`, Lz4
    /// compression, single column family.
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let diag_path = data_dir.join("diagnostics");

        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_max_open_files(64);

        let db = rocksdb::DB::open_cf(&opts, &diag_path, vec![CF_EVENTS])?;
        Ok(Self { db })
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
        self.db.put_cf(cf, &key, &value)?;
        Ok(())
    }

    /// Query recent events within a time window.
    ///
    /// Returns events whose `timestamp_ms` falls within `[now - window_secs*1000, now]`,
    /// ordered oldest-first, capped by `limit`.
    ///
    /// // TODO(M3): Replace full-table scan with time-indexed prefix scan.
    pub fn query_recent(
        &self,
        window_secs: u64,
        limit: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        let cf = self
            .db
            .cf_handle(CF_EVENTS)
            .ok_or_else(|| StorageError::Database("cf_events not found".into()))?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let cutoff_ms = now_ms.saturating_sub(window_secs * 1000);

        let mut results = Vec::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (_, value) = item.map_err(|e| StorageError::Database(e.to_string()))?;
            let event = Self::deserialize_event(&value)?;
            if event.timestamp_ms >= cutoff_ms {
                results.push(event);
            }
            if results.len() >= limit {
                break;
            }
        }
        // Sort by timestamp then event_id for stable oldest-first ordering
        results.sort_by(|a, b| {
            a.timestamp_ms
                .cmp(&b.timestamp_ms)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        if results.len() > limit {
            results.truncate(limit);
        }
        Ok(results)
    }

    /// Query events by kind and height range.
    ///
    /// When `kind` is `Some`, only events matching that kind are returned.
    /// When `kind` is `None`, all events in the height range are returned.
    /// Results are ordered oldest-first, capped by `limit`.
    ///
    /// // TODO(M3): Replace full-table scan with prefix-seek by kind+height.
    pub fn query_range(
        &self,
        kind: Option<EventKind>,
        min_height: u64,
        max_height: u64,
        limit: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        let cf = self
            .db
            .cf_handle(CF_EVENTS)
            .ok_or_else(|| StorageError::Database("cf_events not found".into()))?;

        let mut results = Vec::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (_, value) = item.map_err(|e| StorageError::Database(e.to_string()))?;
            let event = Self::deserialize_event(&value)?;

            if let Some(k) = kind {
                if event.kind != k {
                    continue;
                }
            }
            let h = event.height.unwrap_or(0);
            if h < min_height || h > max_height {
                continue;
            }
            results.push(event);
            if results.len() >= limit {
                break;
            }
        }
        results.sort_by(|a, b| {
            a.timestamp_ms
                .cmp(&b.timestamp_ms)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        if results.len() > limit {
            results.truncate(limit);
        }
        Ok(results)
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
        let total_pruned = stale_keys.len() + excess_keys.len();
        let mut batch = rocksdb::WriteBatch::default();
        for key in stale_keys.iter().chain(excess_keys.iter()) {
            batch.delete_cf(cf, key);
        }
        if total_pruned > 0 {
            self.db
                .write(batch)
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        Ok(total_pruned)
    }
}
