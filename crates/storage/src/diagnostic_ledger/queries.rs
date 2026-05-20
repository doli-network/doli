//! Efficient query implementations for the diagnostic ledger.
//!
//! - `query_range` — prefix-scan by kind + height range, with limit cap at 10,000.
//! - `query_recent` — time-window scan, ordered oldest-first.
//! - `query_causal_chain` — follows `caused_by_event_id` links with cycle detection.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::{DiagnosticEvent, EventKind};
use super::{DiagnosticLedger, CF_EVENTS};
use crate::StorageError;

/// Maximum events any single query can return (REQ-FORKOBS-SEC-003).
const MAX_QUERY_LIMIT: usize = 10_000;

impl DiagnosticLedger {
    /// Query events by kind and height range.
    ///
    /// When `kind` is `Some`, only events matching that kind are returned.
    /// When `kind` is `None`, all events in the height range are returned.
    /// The `limit` is clamped to [`MAX_QUERY_LIMIT`] (10,000).
    /// Results are ordered oldest-first (timestamp, then event_id).
    pub fn query_range_impl(
        &self,
        kind: Option<EventKind>,
        min_height: u64,
        max_height: u64,
        limit: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        let cf = self
            .cf_handle()
            .ok_or_else(|| StorageError::Database("cf_events not found".into()))?;

        let capped_limit = limit.min(MAX_QUERY_LIMIT);

        let mut results = Vec::new();

        if let Some(k) = kind {
            // Prefix scan: seek to [kind_byte, min_height BE, 0..0]
            let mut prefix = Vec::with_capacity(9);
            prefix.push(k.as_u8());
            prefix.extend_from_slice(&min_height.to_be_bytes());

            let iter = self.db.iterator_cf(
                cf,
                rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
            );

            for item in iter {
                let (key, value) = item.map_err(|e| StorageError::Database(e.to_string()))?;
                // Stop if kind byte changed
                if key.is_empty() || key[0] != k.as_u8() {
                    break;
                }
                // Extract height from key bytes [1..9]
                if key.len() >= 9 {
                    let height = u64::from_be_bytes(key[1..9].try_into().unwrap_or([0u8; 8]));
                    if height > max_height {
                        break;
                    }
                    // height >= min_height is guaranteed by seek start
                }
                let event = Self::deserialize_event(&value)?;
                results.push(event);
                if results.len() >= capped_limit {
                    break;
                }
            }
        } else {
            // No kind filter: full scan, filter by height
            let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
            for item in iter {
                let (_, value) = item.map_err(|e| StorageError::Database(e.to_string()))?;
                let event = Self::deserialize_event(&value)?;
                let h = event.height.unwrap_or(0);
                if h >= min_height && h <= max_height {
                    results.push(event);
                    if results.len() >= capped_limit {
                        break;
                    }
                }
            }
        }

        // Sort oldest-first by (timestamp, event_id) for determinism
        results.sort_by(|a, b| {
            a.timestamp_ms
                .cmp(&b.timestamp_ms)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        results.truncate(capped_limit);
        Ok(results)
    }

    /// Query recent events within a time window.
    ///
    /// Returns events whose `timestamp_ms >= now - window_secs*1000`, ordered
    /// oldest-first (timestamp, then event_id), capped by `limit`.
    pub fn query_recent_impl(
        &self,
        window_secs: u64,
        limit: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        let cf = self
            .cf_handle()
            .ok_or_else(|| StorageError::Database("cf_events not found".into()))?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let cutoff_ms = now_ms.saturating_sub(window_secs * 1000);

        let capped_limit = limit.min(MAX_QUERY_LIMIT);

        let mut results = Vec::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (_, value) = item.map_err(|e| StorageError::Database(e.to_string()))?;
            let event = Self::deserialize_event(&value)?;
            if event.timestamp_ms >= cutoff_ms {
                results.push(event);
            }
        }

        // Sort oldest-first for determinism
        results.sort_by(|a, b| {
            a.timestamp_ms
                .cmp(&b.timestamp_ms)
                .then_with(|| a.event_id.cmp(&b.event_id))
        });
        results.truncate(capped_limit);
        Ok(results)
    }

    /// Follow `caused_by_event_id` links from `start_event_id` up to
    /// `max_depth` hops, returning the causal chain oldest-first.
    ///
    /// Cycle detection: a `HashSet<event_id>` is used to avoid infinite loops
    /// if an event points to itself or forms a cycle.
    pub fn query_causal_chain_impl(
        &self,
        start_event_id: &str,
        max_depth: usize,
    ) -> Result<Vec<DiagnosticEvent>, StorageError> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        let mut current_id = start_event_id.to_string();

        for _ in 0..max_depth {
            if visited.contains(&current_id) {
                break;
            }
            visited.insert(current_id.clone());

            // Look up the event by scanning (no secondary index by event_id)
            if let Some(event) = self.lookup_by_event_id(&current_id)? {
                let next = event.caused_by_event_id.clone();
                chain.push(event);
                match next {
                    Some(ref parent_id) if !parent_id.is_empty() => {
                        current_id = parent_id.clone();
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }

        // Reverse to oldest-first
        chain.reverse();
        Ok(chain)
    }

    /// Look up a single event by its `event_id` (full CF scan).
    fn lookup_by_event_id(&self, event_id: &str) -> Result<Option<DiagnosticEvent>, StorageError> {
        let cf = self
            .cf_handle()
            .ok_or_else(|| StorageError::Database("cf_events not found".into()))?;

        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (_, value) = item.map_err(|e| StorageError::Database(e.to_string()))?;
            let event = Self::deserialize_event(&value)?;
            if event.event_id == event_id {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    /// Return a handle to the `cf_events` column family.
    fn cf_handle(&self) -> Option<&rocksdb::ColumnFamily> {
        self.db.cf_handle(CF_EVENTS)
    }
}
