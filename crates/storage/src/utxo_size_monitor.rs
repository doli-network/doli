//! F1 monitor: cached UTXO canonical serialization size gauge.
//!
//! Snap sync silently fails when the canonical UTXO set exceeds 16 MB
//! (MAX_SYNC_SIZE in `crates/network/src/protocols/sync.rs`). This module
//! provides a cached size measurement so Prometheus can alert at 12 MB
//! (75% of the wire limit) before the wall is hit.
//!
//! Design:
//! - `UtxoSizeMonitor` holds an `Arc<StateDb>` and caches the last
//!   computed size + timestamp.
//! - `get_cached_size()` returns the cached value if < 60s old;
//!   otherwise recomputes via `serialize_canonical_utxo().len()`.
//! - `compute_size()` always recomputes (for tests / one-shot use).
//! - `computation_count()` exposes how many times the full computation
//!   has run (for cache-hit testing).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::StateDb;

/// How often (in seconds) the canonical size is recomputed.
const CACHE_TTL_SECS: u64 = 60;

/// Cached UTXO canonical serialization size for the F1 snap-sync monitor.
pub struct UtxoSizeMonitor {
    state_db: Arc<StateDb>,
    /// Cached (size_bytes, computed_at).
    cache: Mutex<Option<(u64, Instant)>>,
    /// Number of times `compute_size()` has been called (for test assertions).
    computations: AtomicU64,
}

impl UtxoSizeMonitor {
    /// Create a new monitor backed by the given state_db.
    pub fn new(state_db: Arc<StateDb>) -> Self {
        Self {
            state_db,
            cache: Mutex::new(None),
            computations: AtomicU64::new(0),
        }
    }

    /// Compute the canonical UTXO serialization size (always recomputes).
    pub fn compute_size(&self) -> u64 {
        self.computations.fetch_add(1, Ordering::Relaxed);
        self.state_db.serialize_canonical_utxo().len() as u64
    }

    /// Return the cached size, recomputing only if the cache is stale (> 60s).
    pub fn get_cached_size(&self) -> u64 {
        let mut cache = self.cache.lock().unwrap();
        if let Some((size, at)) = *cache {
            if at.elapsed().as_secs() < CACHE_TTL_SECS {
                return size;
            }
        }
        let size = self.compute_size();
        *cache = Some((size, Instant::now()));
        size
    }

    /// How many times the full computation has run (for test assertions).
    pub fn computation_count(&self) -> u64 {
        self.computations.load(Ordering::Relaxed)
    }
}
