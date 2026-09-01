//! Prometheus metrics for DOLI node
//!
//! This module provides metrics for monitoring node health and performance.

use std::net::SocketAddr;

use axum::{routing::get, Router};
use lazy_static::lazy_static;
use prometheus::{
    Encoder, Gauge, Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGauge, IntGaugeVec,
    Opts, Registry, TextEncoder,
};
use tracing::{info, warn};

lazy_static! {
    /// Global metrics registry
    pub static ref REGISTRY: Registry = Registry::new();

    // ===================
    // Block Metrics
    // ===================

    /// Total blocks processed
    pub static ref BLOCKS_PROCESSED: IntCounter = IntCounter::new(
        "doli_blocks_processed_total",
        "Total number of blocks processed"
    ).unwrap();

    /// Blocks by status (valid, invalid, orphan)
    pub static ref BLOCKS_BY_STATUS: IntCounterVec = IntCounterVec::new(
        Opts::new("doli_blocks_by_status_total", "Blocks by validation status"),
        &["status"]
    ).unwrap();

    /// Current chain height
    pub static ref CHAIN_HEIGHT: IntGauge = IntGauge::new(
        "doli_chain_height",
        "Current blockchain height"
    ).unwrap();

    /// Current slot number
    pub static ref CURRENT_SLOT: IntGauge = IntGauge::new(
        "doli_current_slot",
        "Current slot number"
    ).unwrap();

    /// Block processing time
    pub static ref BLOCK_PROCESSING_TIME: Histogram = Histogram::with_opts(
        HistogramOpts::new("doli_block_processing_seconds", "Block processing time in seconds")
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0])
    ).unwrap();

    // ===================
    // Transaction Metrics
    // ===================

    /// Total transactions validated
    pub static ref TRANSACTIONS_VALIDATED: IntCounter = IntCounter::new(
        "doli_transactions_validated_total",
        "Total number of transactions validated"
    ).unwrap();

    /// Transactions by type (transfer, registration, exit, coinbase)
    pub static ref TRANSACTIONS_BY_TYPE: IntCounterVec = IntCounterVec::new(
        Opts::new("doli_transactions_by_type_total", "Transactions by type"),
        &["type"]
    ).unwrap();

    /// Transactions by validation result
    pub static ref TRANSACTIONS_BY_RESULT: IntCounterVec = IntCounterVec::new(
        Opts::new("doli_transactions_by_result_total", "Transactions by validation result"),
        &["result"]
    ).unwrap();

    // ===================
    // Mempool Metrics
    // ===================

    /// Current mempool size (transaction count)
    pub static ref MEMPOOL_SIZE: IntGauge = IntGauge::new(
        "doli_mempool_size",
        "Current number of transactions in mempool"
    ).unwrap();

    /// Mempool size in bytes
    pub static ref MEMPOOL_BYTES: IntGauge = IntGauge::new(
        "doli_mempool_bytes",
        "Current mempool size in bytes"
    ).unwrap();

    // ===================
    // Network Metrics
    // ===================

    /// Number of connected peers
    pub static ref PEERS_CONNECTED: IntGauge = IntGauge::new(
        "doli_peers_connected",
        "Number of currently connected peers"
    ).unwrap();

    /// Total peers seen
    pub static ref PEERS_SEEN_TOTAL: IntCounter = IntCounter::new(
        "doli_peers_seen_total",
        "Total number of peers ever connected"
    ).unwrap();

    /// Peers by connection status
    pub static ref PEERS_BY_STATUS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_peers_by_status", "Peers by connection status"),
        &["status"]
    ).unwrap();

    /// Messages received by type
    pub static ref MESSAGES_RECEIVED: IntCounterVec = IntCounterVec::new(
        Opts::new("doli_messages_received_total", "Messages received by type"),
        &["type"]
    ).unwrap();

    /// Messages sent by type
    pub static ref MESSAGES_SENT: IntCounterVec = IntCounterVec::new(
        Opts::new("doli_messages_sent_total", "Messages sent by type"),
        &["type"]
    ).unwrap();

    /// Network bandwidth (bytes received)
    pub static ref BYTES_RECEIVED: IntCounter = IntCounter::new(
        "doli_bytes_received_total",
        "Total bytes received from network"
    ).unwrap();

    /// Network bandwidth (bytes sent)
    pub static ref BYTES_SENT: IntCounter = IntCounter::new(
        "doli_bytes_sent_total",
        "Total bytes sent to network"
    ).unwrap();

    // ===================
    // Sync Metrics
    // ===================

    /// Sync progress (0.0 to 1.0)
    pub static ref SYNC_PROGRESS: Gauge = Gauge::new(
        "doli_sync_progress",
        "Synchronization progress (0.0 to 1.0)"
    ).unwrap();

    /// Is node syncing
    pub static ref IS_SYNCING: IntGauge = IntGauge::new(
        "doli_is_syncing",
        "Whether the node is currently syncing (1) or synced (0)"
    ).unwrap();

    /// Blocks behind
    pub static ref BLOCKS_BEHIND: IntGauge = IntGauge::new(
        "doli_blocks_behind",
        "Number of blocks behind the network"
    ).unwrap();

    // ===================
    // VDF Metrics
    // ===================

    /// VDF computation time
    pub static ref VDF_COMPUTE_SECONDS: Histogram = Histogram::with_opts(
        HistogramOpts::new("doli_vdf_compute_seconds", "VDF computation time in seconds")
            .buckets(vec![10.0, 20.0, 30.0, 40.0, 50.0, 55.0, 60.0, 70.0, 80.0])
    ).unwrap();

    /// VDF verification time
    pub static ref VDF_VERIFY_SECONDS: Histogram = Histogram::with_opts(
        HistogramOpts::new("doli_vdf_verify_seconds", "VDF verification time in seconds")
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0])
    ).unwrap();

    // ===================
    // Producer Metrics
    // ===================

    /// Total active producers
    pub static ref ACTIVE_PRODUCERS: IntGauge = IntGauge::new(
        "doli_active_producers",
        "Number of active block producers"
    ).unwrap();

    /// Blocks produced by this node
    pub static ref BLOCKS_PRODUCED: IntCounter = IntCounter::new(
        "doli_blocks_produced_total",
        "Total blocks produced by this node"
    ).unwrap();

    /// INC-I-154 F1: producers that did NOT attest in the most recent block carrying
    /// attestation data. `segment="scheduled"` = member of the epoch's producer_list;
    /// `segment="unscheduled"` = active producer the epoch-boundary attestation filter
    /// left out of the schedule. Before this metric existed, a persistently absent
    /// producer was observable only by reading logs by eye — and an unscheduled one was
    /// not observable at all.
    pub static ref ATTESTATION_MISSING_CURRENT: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "doli_attestation_missing_current",
            "Producers that did not attest in the most recent block with attestation data, by segment"
        ),
        &["segment"]
    ).unwrap();

    /// INC-I-154 F1: cumulative producer-block attestation misses, by segment.
    pub static ref ATTESTATION_MISSES_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_attestation_misses_total",
            "Cumulative producer-block attestation misses, by segment"
        ),
        &["segment"]
    ).unwrap();

    /// Slot latency (time from slot start to block)
    pub static ref SLOT_LATENCY: Histogram = Histogram::with_opts(
        HistogramOpts::new("doli_slot_latency_seconds", "Time from slot start to block production")
            .buckets(vec![1.0, 5.0, 10.0, 20.0, 30.0, 40.0, 50.0, 55.0, 60.0])
    ).unwrap();

    // ===================
    // Storage Metrics
    // ===================

    /// UTXO set size
    pub static ref UTXO_SET_SIZE: IntGauge = IntGauge::new(
        "doli_utxo_set_size",
        "Number of unspent transaction outputs"
    ).unwrap();

    /// Storage size in bytes
    pub static ref STORAGE_BYTES: IntGauge = IntGauge::new(
        "doli_storage_bytes",
        "Total storage size in bytes"
    ).unwrap();

    // ===================
    // F1 Snap-Sync Size Monitor (Phase 5)
    // ===================
    // Snap sync silently fails when the canonical UTXO set exceeds
    // MAX_SYNC_SIZE (16 MB) in crates/network/src/protocols/sync.rs.
    // These gauges provide early warning.
    //
    // Recommended Prometheus alert rule:
    //   alert: UtxoCanonicalSizeApproachingLimit
    //   expr: doli_utxo_canonical_size_bytes > 12582912
    //   for: 5m
    //   labels: { severity: warning }
    //   annotations:
    //     summary: "UTXO canonical size > 12 MB (75% of 16 MB snap sync limit)"
    //     description: "New nodes will fail snap sync when size exceeds 16 MB.
    //                   Start Tier 3-A chunked snap sync work."

    /// Approximate byte size of the canonical UTXO set serialization.
    /// Cached; recomputed at most once per 60 seconds.
    /// Snap sync silently fails when this exceeds MAX_SYNC_SIZE (16 MB).
    pub static ref UTXO_CANONICAL_SIZE_BYTES: IntGauge = IntGauge::new(
        "doli_utxo_canonical_size_bytes",
        "Approximate byte size of the canonical UTXO set serialization. \
         Snap sync silently fails when this exceeds MAX_SYNC_SIZE (16 MB). \
         Alert: > 12582912 (12 MB = 75% of limit) for 5m."
    ).unwrap();

    /// The snap sync wire limit (MAX_SYNC_SIZE) so dashboards can render
    /// a relative bar without hardcoding. Set once at startup, never changes.
    pub static ref UTXO_CANONICAL_SIZE_THRESHOLD_BYTES: IntGauge = IntGauge::new(
        "doli_utxo_canonical_size_threshold_bytes",
        "Snap sync wire limit (MAX_SYNC_SIZE = 16 MB). Static reference for dashboard \
         ratio computations: doli_utxo_canonical_size_bytes / this."
    ).unwrap();

    // ===================
    // Fork-Guard Backfill Refusals (INC-I-156 M2 / AUDIT-P2-206)
    // ===================
    // Incremented when a state-rebuild path REFUSES to run because the local
    // block_store is not dense over the range it would have to replay, and
    // logs `[FORK_GUARD_BACKFILL_REQUIRED]`. The refusal is the safe outcome
    // (no state is mutated), but a node that keeps hitting it cannot rebuild
    // its ProducerSet and makes no progress until sync backfills the gap —
    // previously detectable only by watching chain height stall, the same
    // detection lag that produced the INC-I-034 cascade.
    //
    // Recommended Prometheus alert rule:
    //   alert: ForkGuardBackfillRequired
    //   expr: increase(doli_fork_guard_refusals_total[15m]) > 0
    //   for: 5m
    //   labels: { severity: warning }
    //   annotations:
    //     summary: "Node refused a state rebuild — local block_store has a gap"
    //     description: "Let header-first sync backfill the missing heights. If
    //                   the blocks are present but the height index is stale,
    //                   stop the node and run `doli-node reindex --data-dir <DIR>`."
    //
    // INSTRUMENT SCOPE — every FORK_GUARD refusal path increments (INC-I-204 M0):
    //   producer_rebuild  rewards.rs        rebuild_producer_set_from_blocks
    //   rollback_rebuild  rollback.rs       rollback_one_block, no-undo fallback
    //   reorg_execute     block_handling.rs execute_reorg, both preconditions
    // All three series are zero-initialised in register_metrics(), so a zero is a
    // measured "no refusals" and an absent series means the exporter is broken.
    pub static ref FORK_GUARD_REFUSALS: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_fork_guard_refusals_total",
            "State rebuilds and reorgs refused because block_store is not dense over the \
             replay range ([FORK_GUARD_BACKFILL_REQUIRED]). Every refusal path increments: \
             site=producer_rebuild, rollback_rebuild, reorg_execute."
        ),
        &["site"]
    ).unwrap();

    // ===================
    // Fork lifecycle (INC-I-204 M0 — observability only)
    // ===================

    /// Distinct chain tips across our tip plus connected peers, from
    /// `SyncManager::checkpoint_health()`. 1 = the fleet agrees; 0 = no peers,
    /// i.e. no evidence either way. Sustained > 1 is the INC-I-204 wedge witness.
    pub static ref UNIQUE_CHAIN_TIPS: IntGauge = IntGauge::new(
        "doli_unique_chain_tips",
        "Distinct chain tips across our tip plus connected peers. 1 = the fleet agrees, \
         0 = no connected peers (no evidence)."
    ).unwrap();

    /// FORK_GUARD wedge-escape decisions, one series per reason.
    pub static ref WEDGE_ESCAPE_OUTCOMES: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_wedge_escape_outcomes_total",
            "FORK_GUARD wedge-escape decisions, by reason: cannot_outweigh, plan_refused, \
             not_heavier, reorg, reorg_did_not_land."
        ),
        &["reason"]
    ).unwrap();

    /// Executions of a pre-activation branch of an activation-height gate — the
    /// dormant-window canary REQ-FORK-014 asks for.
    pub static ref PRE_ACTIVATION_BRANCH: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_pre_activation_branch_total",
            "Executions of a pre-activation (known-legacy) branch of an activation-height \
             gate. Non-zero on a network past that height means the gate is misconfigured."
        ),
        &["gate"]
    ).unwrap();

    /// Finality-guard comparisons reached and refused, by site. Measures the
    /// INV-SYNC-012 reject rate M5's deletion decision depends on.
    pub static ref REORG_FINALITY_PROBE: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_reorg_finality_probe_total",
            "Finality-guard comparisons reached (outcome=entry) and refused (outcome=reject), \
             by site: check_reorg_weighted, plan_reorg."
        ),
        &["site", "outcome"]
    ).unwrap();

    // ===================
    // DeFi Economic Security (D4 / AC-6)
    // ===================

    /// Sum of every active Bond UTXO `amount`, base units. Saturated at u64::MAX.
    /// Spec: `specs/defi-subsystem-architecture.md` AC block (AC-6).
    pub static ref DEFI_TOTAL_ACTIVE_BONDS: IntGauge = IntGauge::new(
        "doli_defi_total_active_bonds",
        "Sum of every active Bond UTXO amount (DOLI base units)."
    ).unwrap();

    /// Largest single Pool UTXO TVL in DOLI base units (Phase-1 pre-oracle:
    /// tvl = 2 * reserve_a using the pool's own spot price).
    /// Spec: `specs/defi-subsystem-architecture.md` AC block (AC-6).
    pub static ref DEFI_MAX_POOL_TVL: IntGauge = IntGauge::new(
        "doli_defi_max_pool_tvl",
        "Largest single Pool UTXO TVL in DOLI base units (pre-oracle: 2 * reserve_a)."
    ).unwrap();

    /// R = total_active_bonds / max_pool_TVL. R < 1.0 → economic security
    /// against single-pool capture is degraded (disclosure only, no TX
    /// rejection). NaN when max_pool_TVL == 0.
    /// Spec: `specs/defi-subsystem-architecture.md` AC block (AC-6, ACCEPTED 2026-05-29).
    pub static ref DEFI_BOND_TO_TVL_RATIO: Gauge = Gauge::new(
        "doli_defi_bond_to_tvl_ratio",
        "AC-6 monitoring metric: total_active_bonds / max_pool_TVL. < 1.0 == degraded."
    ).unwrap();

    // ===================
    // System Metrics
    // ===================

    /// Node uptime in seconds
    pub static ref UPTIME_SECONDS: IntGauge = IntGauge::new(
        "doli_uptime_seconds",
        "Node uptime in seconds"
    ).unwrap();

    /// Build info
    pub static ref BUILD_INFO: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_build_info", "Build information"),
        &["version", "commit"]
    ).unwrap();

    // ===================
    // RocksDB Metrics (per-instance, labeled by instance="block_store|state_db")
    // ===================
    // Properties read via `db.property_int_value(...)`. See storage::metrics for property names.

    /// Current memtable bytes (sum across all named CFs). Compare against
    /// doli_rocksdb_memtable_max_bytes to detect approach-to-cap.
    /// Alert: `doli_rocksdb_memtable_bytes / doli_rocksdb_memtable_max_bytes > 0.9 for 5m`.
    pub static ref ROCKSDB_MEMTABLE_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_memtable_bytes",
            "Current memtable bytes summed across named CFs (cur-size-all-mem-tables). \
             Alert when ratio to _max_bytes > 0.9 sustained 5m — memtable near cap, flush stall risk."),
        &["instance"]
    ).unwrap();

    /// Current memtable bytes summed across named CFs, *including pinned
    /// immutable memtables*. Slightly larger than `memtable_bytes` when
    /// pinned memtables exist. This is CURRENT usage, NOT the configured cap.
    /// For the cap, see `doli_rocksdb_memtable_cap_bytes`.
    pub static ref ROCKSDB_MEMTABLE_MAX_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_memtable_max_bytes",
            "Current memtable bytes incl. pinned immutables (size-all-mem-tables). \
             CURRENT usage, not the cap — use _memtable_cap_bytes for cap comparisons."),
        &["instance"]
    ).unwrap();

    /// Configured `db_write_buffer_size` — the hard cap. INC-I-104 M0 values:
    /// block_store=48 MB, state_db=64 MB.
    /// Per Failure Analyst C-002, C-007, C-011 the spec requires these caps.
    /// Use for approach-to-cap alerts: `memtable_bytes / memtable_cap_bytes > 0.9`.
    pub static ref ROCKSDB_MEMTABLE_CAP_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_memtable_cap_bytes",
            "Configured db_write_buffer_size — the hard cap on total memtable allocation. \
             Per INC-I-104 M0 spec: block_store=50331648, state_db=67108864. \
             Alert: doli_rocksdb_memtable_bytes / doli_rocksdb_memtable_cap_bytes > 0.9 sustained 5m."),
        &["instance"]
    ).unwrap();

    /// Block cache resident bytes. INC-I-106: queried directly from
    /// `rocksdb::Cache::get_usage()`, not summed across CFs. Shared cache per
    /// instance: block_store=32 MB, state_db=48 MB. Compare against the matching
    /// `doli_rocksdb_block_cache_capacity_bytes{instance="…"}` for headroom.
    pub static ref ROCKSDB_BLOCK_CACHE_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_block_cache_bytes",
            "Block cache resident bytes per instance, from rocksdb::Cache::get_usage() (INC-I-106). \
             Configured caps: block_store=32MB, state_db=48MB. \
             Compare against doli_rocksdb_block_cache_capacity_bytes for headroom."),
        &["instance"]
    ).unwrap();

    /// Block cache pinned bytes — subset of cache memory that can't be evicted.
    /// INC-I-106: queried directly from `rocksdb::Cache::get_pinned_usage()`.
    pub static ref ROCKSDB_BLOCK_CACHE_PINNED_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_block_cache_pinned_bytes",
            "Block cache pinned bytes (cannot be evicted), from rocksdb::Cache::get_pinned_usage() (INC-I-106). \
             High ratio to _block_cache_bytes means little room for new reads."),
        &["instance"]
    ).unwrap();

    /// Configured block-cache capacity per instance. INC-I-107: exposes the
    /// `Cache::new_lru_cache(N)` size that the matching `_block_cache_bytes`
    /// gauge should stay below. Use `_block_cache_bytes / _block_cache_capacity_bytes`
    /// as the approach-to-cap signal.
    pub static ref ROCKSDB_BLOCK_CACHE_CAPACITY_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_block_cache_capacity_bytes",
            "Configured shared LRU block cache capacity per instance, in bytes (INC-I-107). \
             Set at storage open(): block_store=33554432 (32MB), state_db=50331648 (48MB). \
             Alert: doli_rocksdb_block_cache_bytes / this > 0.9 sustained 5m."),
        &["instance"]
    ).unwrap();

    /// SST index + bloom filter memory. Counts against per-node RSS.
    pub static ref ROCKSDB_TABLE_READERS_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_table_readers_bytes",
            "Memory used by SST index + bloom filter blocks (estimate-table-readers-mem). \
             Grows with SST count; counts against process RSS."),
        &["instance"]
    ).unwrap();

    /// Estimated live key count per instance.
    pub static ref ROCKSDB_ESTIMATE_KEYS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_estimate_keys",
            "Approximate live key count summed across named CFs (estimate-num-keys)."),
        &["instance"]
    ).unwrap();

    /// Live data bytes (post-compaction estimate).
    pub static ref ROCKSDB_LIVE_DATA_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_live_data_bytes",
            "Approximate live data bytes after compaction (estimate-live-data-size)."),
        &["instance"]
    ).unwrap();

    /// Total SST bytes on disk (includes obsolete pending deletion).
    pub static ref ROCKSDB_SST_TOTAL_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_sst_total_bytes",
            "Total SST file bytes on disk including obsolete (total-sst-files-size)."),
        &["instance"]
    ).unwrap();

    /// Live SST bytes (excludes obsolete pending deletion).
    pub static ref ROCKSDB_SST_LIVE_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_sst_live_bytes",
            "Live SST file bytes (live-sst-files-size). \
             Big gap to _sst_total_bytes means obsolete files queued for delete."),
        &["instance"]
    ).unwrap();

    /// Flush jobs currently executing (DB-scoped).
    pub static ref ROCKSDB_RUNNING_FLUSHES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_running_flushes",
            "Flush jobs currently executing (DB-scoped). \
             Sustained > 0 with high _memtable_bytes indicates flush throughput bottleneck."),
        &["instance"]
    ).unwrap();

    /// Compaction jobs currently executing (DB-scoped).
    pub static ref ROCKSDB_RUNNING_COMPACTIONS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_running_compactions",
            "Compaction jobs currently executing (DB-scoped). Capped by max_background_jobs."),
        &["instance"]
    ).unwrap();

    /// Compaction pending: > 0 means a compaction is queued / scheduled.
    pub static ref ROCKSDB_COMPACTION_PENDING: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_compaction_pending",
            "Compaction queued/pending (summed across named CFs). \
             Sustained > 1 with rising L0 files = falling behind."),
        &["instance"]
    ).unwrap();

    /// Memtable flush pending: > 0 means a flush is queued / scheduled.
    pub static ref ROCKSDB_MEMTABLE_FLUSH_PENDING: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_memtable_flush_pending",
            "Memtable flush queued/pending (summed across named CFs). \
             Sustained > 0 means write rate exceeds flush rate."),
        &["instance"]
    ).unwrap();

    /// Immutable memtables awaiting flush.
    pub static ref ROCKSDB_NUM_IMMUTABLE_MEMTABLE: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_num_immutable_memtable",
            "Immutable memtables awaiting flush (summed across named CFs). \
             Approaching max_write_buffer_number means stall imminent."),
        &["instance"]
    ).unwrap();

    /// Write throttle rate. Non-zero means RocksDB is delaying writes
    /// (level0_slowdown_writes_trigger reached).
    pub static ref ROCKSDB_ACTUAL_DELAYED_WRITE_RATE: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_actual_delayed_write_rate",
            "RocksDB write throttle rate in bytes/sec (DB-scoped). \
             0 = no throttling; > 0 = level0_slowdown_writes_trigger hit. \
             Alert when > 0 sustained 30s — consensus hot path is being throttled."),
        &["instance"]
    ).unwrap();

    /// Writes stopped (1/0). Critical: 1 means level0_stop_writes_trigger
    /// hit and writes are completely blocked.
    pub static ref ROCKSDB_IS_WRITE_STOPPED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_is_write_stopped",
            "Writes stopped 1/0 (DB-scoped). 1 = level0_stop_writes_trigger hit, all writes BLOCKED. \
             CRITICAL: alert immediately if > 0. Matches Failure Analyst FM-02. \
             For consensus-critical instances (state_db, block_store) this directly causes missed slots."),
        &["instance"]
    ).unwrap();

    /// SST file count per LSM level. Labels: instance, level (0..6).
    /// Level 0 file count is the C-003 write-stall predictor —
    /// slowdown=40, stop=60 on hot CFs per the INC-I-104 spec.
    pub static ref ROCKSDB_FILES_AT_LEVEL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("doli_rocksdb_files_at_level",
            "SST file count per LSM level (summed across named CFs). \
             Level 0 count near level0_slowdown_writes_trigger (40 on hot CFs per INC-I-104) = stall imminent. \
             Alert: sum by (instance) (doli_rocksdb_files_at_level{level=\"0\"}) > 30."),
        &["instance", "level"]
    ).unwrap();

    /// Cumulative background errors (compaction/flush failures). Resets on
    /// process restart — Prometheus rate()/increase() handle this.
    pub static ref ROCKSDB_BACKGROUND_ERRORS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("doli_rocksdb_background_errors_total",
            "Cumulative background errors (compaction/flush failures). \
             Counter: use rate() or increase() in PromQL. \
             Alert: increase(doli_rocksdb_background_errors_total[5m]) > 0 — new error since 5 min ago."),
        &["instance"]
    ).unwrap();
}

/// Every `site` value `FORK_GUARD_REFUSALS` is written with.
pub const FORK_GUARD_SITES: [&str; 3] = ["producer_rebuild", "rollback_rebuild", "reorg_execute"];

/// Every `gate` value `PRE_ACTIVATION_BRANCH` is written with.
const PRE_ACTIVATION_GATES: [&str; 2] =
    ["inc_i_147_record_height", "inc_i_147_plan_reorg_finality"];

/// Every `site` value `REORG_FINALITY_PROBE` is written with.
const FINALITY_PROBE_SITES: [&str; 2] = ["check_reorg_weighted", "plan_reorg"];

/// Register all metrics with the registry
pub fn register_metrics() {
    let _ = REGISTRY.register(Box::new(BLOCKS_PROCESSED.clone()));
    let _ = REGISTRY.register(Box::new(BLOCKS_BY_STATUS.clone()));
    let _ = REGISTRY.register(Box::new(CHAIN_HEIGHT.clone()));
    let _ = REGISTRY.register(Box::new(CURRENT_SLOT.clone()));
    let _ = REGISTRY.register(Box::new(BLOCK_PROCESSING_TIME.clone()));

    let _ = REGISTRY.register(Box::new(TRANSACTIONS_VALIDATED.clone()));
    let _ = REGISTRY.register(Box::new(TRANSACTIONS_BY_TYPE.clone()));
    let _ = REGISTRY.register(Box::new(TRANSACTIONS_BY_RESULT.clone()));

    let _ = REGISTRY.register(Box::new(MEMPOOL_SIZE.clone()));
    let _ = REGISTRY.register(Box::new(MEMPOOL_BYTES.clone()));

    let _ = REGISTRY.register(Box::new(PEERS_CONNECTED.clone()));
    let _ = REGISTRY.register(Box::new(PEERS_SEEN_TOTAL.clone()));
    let _ = REGISTRY.register(Box::new(PEERS_BY_STATUS.clone()));
    let _ = REGISTRY.register(Box::new(MESSAGES_RECEIVED.clone()));
    let _ = REGISTRY.register(Box::new(MESSAGES_SENT.clone()));
    let _ = REGISTRY.register(Box::new(BYTES_RECEIVED.clone()));
    let _ = REGISTRY.register(Box::new(BYTES_SENT.clone()));

    let _ = REGISTRY.register(Box::new(SYNC_PROGRESS.clone()));
    let _ = REGISTRY.register(Box::new(IS_SYNCING.clone()));
    let _ = REGISTRY.register(Box::new(BLOCKS_BEHIND.clone()));

    let _ = REGISTRY.register(Box::new(VDF_COMPUTE_SECONDS.clone()));
    let _ = REGISTRY.register(Box::new(VDF_VERIFY_SECONDS.clone()));

    let _ = REGISTRY.register(Box::new(ACTIVE_PRODUCERS.clone()));
    let _ = REGISTRY.register(Box::new(BLOCKS_PRODUCED.clone()));
    let _ = REGISTRY.register(Box::new(ATTESTATION_MISSING_CURRENT.clone()));
    let _ = REGISTRY.register(Box::new(ATTESTATION_MISSES_TOTAL.clone()));
    // INC-I-154: a labelled metric publishes NO series until a label value is first
    // touched. Initialise both segments to 0 so the series exist from process start —
    // otherwise a "missing > 0 for 30m" alert has nothing to evaluate on a node that
    // has not yet seen a miss, and `absent()` handling would be required instead.
    for segment in ["scheduled", "unscheduled"] {
        ATTESTATION_MISSING_CURRENT
            .with_label_values(&[segment])
            .set(0);
        ATTESTATION_MISSES_TOTAL
            .with_label_values(&[segment])
            .inc_by(0);
    }
    let _ = REGISTRY.register(Box::new(SLOT_LATENCY.clone()));

    let _ = REGISTRY.register(Box::new(UTXO_SET_SIZE.clone()));
    let _ = REGISTRY.register(Box::new(STORAGE_BYTES.clone()));

    // F1 snap-sync size monitor (Phase 5)
    let _ = REGISTRY.register(Box::new(UTXO_CANONICAL_SIZE_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(UTXO_CANONICAL_SIZE_THRESHOLD_BYTES.clone()));
    // Set the static threshold once (MAX_SYNC_SIZE = 16 MB).
    UTXO_CANONICAL_SIZE_THRESHOLD_BYTES.set(16 * 1024 * 1024);

    // Fork-guard backfill refusals (INC-I-156 M2 / AUDIT-P2-206)
    let _ = REGISTRY.register(Box::new(FORK_GUARD_REFUSALS.clone()));

    // Fork lifecycle (INC-I-204 M0)
    let _ = REGISTRY.register(Box::new(UNIQUE_CHAIN_TIPS.clone()));
    let _ = REGISTRY.register(Box::new(WEDGE_ESCAPE_OUTCOMES.clone()));
    let _ = REGISTRY.register(Box::new(PRE_ACTIVATION_BRANCH.clone()));
    let _ = REGISTRY.register(Box::new(REORG_FINALITY_PROBE.clone()));
    // INC-I-154 again: zero-initialise every label value, or the family publishes
    // no series and an alert on an untouched site has nothing to evaluate.
    for site in FORK_GUARD_SITES {
        FORK_GUARD_REFUSALS.with_label_values(&[site]).inc_by(0);
    }
    for outcome in crate::node::wedge_outcome::ALL_WEDGE_OUTCOMES {
        WEDGE_ESCAPE_OUTCOMES
            .with_label_values(&[outcome.reason()])
            .inc_by(0);
    }
    for gate in PRE_ACTIVATION_GATES {
        PRE_ACTIVATION_BRANCH.with_label_values(&[gate]).inc_by(0);
    }
    for site in FINALITY_PROBE_SITES {
        for outcome in ["entry", "reject"] {
            REORG_FINALITY_PROBE
                .with_label_values(&[site, outcome])
                .inc_by(0);
        }
    }

    let _ = REGISTRY.register(Box::new(DEFI_TOTAL_ACTIVE_BONDS.clone()));
    let _ = REGISTRY.register(Box::new(DEFI_MAX_POOL_TVL.clone()));
    let _ = REGISTRY.register(Box::new(DEFI_BOND_TO_TVL_RATIO.clone()));

    let _ = REGISTRY.register(Box::new(UPTIME_SECONDS.clone()));
    let _ = REGISTRY.register(Box::new(BUILD_INFO.clone()));

    // RocksDB metrics
    let _ = REGISTRY.register(Box::new(ROCKSDB_MEMTABLE_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_MEMTABLE_MAX_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_MEMTABLE_CAP_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_BLOCK_CACHE_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_BLOCK_CACHE_PINNED_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_BLOCK_CACHE_CAPACITY_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_TABLE_READERS_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_ESTIMATE_KEYS.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_LIVE_DATA_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_SST_TOTAL_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_SST_LIVE_BYTES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_RUNNING_FLUSHES.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_RUNNING_COMPACTIONS.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_COMPACTION_PENDING.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_MEMTABLE_FLUSH_PENDING.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_NUM_IMMUTABLE_MEMTABLE.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_ACTUAL_DELAYED_WRITE_RATE.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_IS_WRITE_STOPPED.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_BACKGROUND_ERRORS_TOTAL.clone()));
    let _ = REGISTRY.register(Box::new(ROCKSDB_FILES_AT_LEVEL.clone()));

    // Set build info
    BUILD_INFO
        .with_label_values(&[env!("CARGO_PKG_VERSION"), "unknown"])
        .set(1);
}

/// Update the D4 / AC-6 DeFi economic-security gauges from a UTXO snapshot.
///
/// `max_pool` is `None` when no Pool UTXOs exist — the ratio gauge is set to
/// `f64::NAN` in that case so Prometheus surfaces "no value" rather than 0
/// (avoids implying R=0 / fully-degraded when the metric is simply undefined).
pub fn update_defi_health_metric(total_active_bonds: u64, max_pool: Option<(crypto::Hash, u64)>) {
    DEFI_TOTAL_ACTIVE_BONDS.set(total_active_bonds as i64);
    match max_pool {
        Some((_pool_id, tvl)) => {
            DEFI_MAX_POOL_TVL.set(tvl as i64);
            let ratio = if tvl == 0 {
                0.0
            } else {
                (total_active_bonds as f64) / (tvl as f64)
            };
            DEFI_BOND_TO_TVL_RATIO.set(ratio);
        }
        None => {
            DEFI_MAX_POOL_TVL.set(0);
            DEFI_BOND_TO_TVL_RATIO.set(f64::NAN);
        }
    }
}

/// Publish the fleet-divergence gauge. `tips == 0` means no connected peers.
pub fn update_unique_chain_tips(tips: usize) {
    UNIQUE_CHAIN_TIPS.set(tips as i64);
}

/// Count one wedge-escape decision. `reason` comes from `WedgeOutcome::reason()`.
pub fn record_wedge_escape_outcome(reason: &str) {
    WEDGE_ESCAPE_OUTCOMES.with_label_values(&[reason]).inc();
}

/// Cumulative FORK_GUARD refusals across every instrumented site.
///
/// Feeds the observability-only `WedgeAlarm`; no decision path reads it.
pub fn fork_guard_refusals_total() -> u64 {
    FORK_GUARD_SITES
        .iter()
        .map(|site| FORK_GUARD_REFUSALS.with_label_values(&[site]).get())
        .sum()
}

/// Last-seen `ReorgObservations`, retained across scrapes: a Prometheus counter
/// can only `inc_by(delta)`, never `set()`. Same shape as `RocksDbScrapeState`.
#[derive(Default)]
pub struct ReorgScrapeState {
    last: network::sync::ReorgObservations,
}

impl ReorgScrapeState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Carry one `ReorgHandler` observation snapshot into Prometheus.
///
/// `crates/network` has no `prometheus` dependency, so the handler keeps plain
/// atomics and this is the scrape seam — the `apply_rocksdb_metrics` pattern.
pub fn apply_reorg_observations(
    state: &mut ReorgScrapeState,
    o: &network::sync::ReorgObservations,
) {
    let last = state.last;
    inc_delta(
        &REORG_FINALITY_PROBE,
        &["check_reorg_weighted", "entry"],
        o.check_reorg_finality_entries,
        last.check_reorg_finality_entries,
    );
    inc_delta(
        &REORG_FINALITY_PROBE,
        &["check_reorg_weighted", "reject"],
        o.check_reorg_finality_rejects,
        last.check_reorg_finality_rejects,
    );
    inc_delta(
        &REORG_FINALITY_PROBE,
        &["plan_reorg", "entry"],
        o.plan_reorg_finality_entries,
        last.plan_reorg_finality_entries,
    );
    inc_delta(
        &REORG_FINALITY_PROBE,
        &["plan_reorg", "reject"],
        o.plan_reorg_finality_rejects,
        last.plan_reorg_finality_rejects,
    );
    inc_delta(
        &PRE_ACTIVATION_BRANCH,
        &["inc_i_147_record_height"],
        o.pre_activation_record_height,
        last.pre_activation_record_height,
    );
    inc_delta(
        &PRE_ACTIVATION_BRANCH,
        &["inc_i_147_plan_reorg_finality"],
        o.pre_activation_plan_reorg_finality,
        last.pre_activation_plan_reorg_finality,
    );
    state.last = *o;
}

/// Advance a counter series by the positive delta only; a handler replaced at
/// runtime restarts from 0 and Prometheus handles the reset itself.
fn inc_delta(family: &IntCounterVec, labels: &[&str], now: u64, last: u64) {
    if now > last {
        family.with_label_values(labels).inc_by(now - last);
    }
}

/// Per-instance state retained across scrapes so the cumulative
/// `background_errors` RocksDB property can be exposed as a proper Prometheus
/// counter: we increment by the positive delta vs the last-seen value.
/// On process restart the registry is empty AND last_seen resets, so the
/// counter naturally starts at 0 — Prometheus rate()/increase() handle the
/// reset correctly without us having to do anything special.
#[derive(Default)]
pub struct RocksDbScrapeState {
    last_background_errors: std::collections::HashMap<&'static str, u64>,
}

impl RocksDbScrapeState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Apply a single `RocksDbMetrics` snapshot to the per-instance gauges and
/// counters. The `state` parameter retains cross-scrape state for counter
/// delta computation.
pub fn apply_rocksdb_metrics(state: &mut RocksDbScrapeState, m: &storage::RocksDbMetrics) {
    let inst = m.instance;
    ROCKSDB_MEMTABLE_BYTES
        .with_label_values(&[inst])
        .set(m.memtable_bytes as i64);
    ROCKSDB_MEMTABLE_MAX_BYTES
        .with_label_values(&[inst])
        .set(m.memtable_max_bytes as i64);
    ROCKSDB_MEMTABLE_CAP_BYTES
        .with_label_values(&[inst])
        .set(m.memtable_cap_bytes as i64);
    ROCKSDB_BLOCK_CACHE_BYTES
        .with_label_values(&[inst])
        .set(m.block_cache_bytes as i64);
    ROCKSDB_BLOCK_CACHE_PINNED_BYTES
        .with_label_values(&[inst])
        .set(m.block_cache_pinned_bytes as i64);
    ROCKSDB_BLOCK_CACHE_CAPACITY_BYTES
        .with_label_values(&[inst])
        .set(m.block_cache_capacity as i64);
    ROCKSDB_TABLE_READERS_BYTES
        .with_label_values(&[inst])
        .set(m.table_readers_bytes as i64);
    ROCKSDB_ESTIMATE_KEYS
        .with_label_values(&[inst])
        .set(m.estimate_keys as i64);
    ROCKSDB_LIVE_DATA_BYTES
        .with_label_values(&[inst])
        .set(m.live_data_bytes as i64);
    ROCKSDB_SST_TOTAL_BYTES
        .with_label_values(&[inst])
        .set(m.sst_total_bytes as i64);
    ROCKSDB_SST_LIVE_BYTES
        .with_label_values(&[inst])
        .set(m.sst_live_bytes as i64);
    ROCKSDB_RUNNING_FLUSHES
        .with_label_values(&[inst])
        .set(m.running_flushes as i64);
    ROCKSDB_RUNNING_COMPACTIONS
        .with_label_values(&[inst])
        .set(m.running_compactions as i64);
    ROCKSDB_COMPACTION_PENDING
        .with_label_values(&[inst])
        .set(m.compaction_pending as i64);
    ROCKSDB_MEMTABLE_FLUSH_PENDING
        .with_label_values(&[inst])
        .set(m.mem_table_flush_pending as i64);
    ROCKSDB_NUM_IMMUTABLE_MEMTABLE
        .with_label_values(&[inst])
        .set(m.num_immutable_memtable as i64);
    ROCKSDB_ACTUAL_DELAYED_WRITE_RATE
        .with_label_values(&[inst])
        .set(m.actual_delayed_write_rate as i64);
    ROCKSDB_IS_WRITE_STOPPED
        .with_label_values(&[inst])
        .set(m.is_write_stopped as i64);

    // Counter: increment by positive delta vs last-seen. Negative deltas
    // (RocksDB property "decremented" somehow — shouldn't happen but be safe)
    // are no-ops; Prometheus counters can only go up.
    let last = state
        .last_background_errors
        .get(&inst)
        .copied()
        .unwrap_or(0);
    if m.background_errors > last {
        let delta = m.background_errors - last;
        ROCKSDB_BACKGROUND_ERRORS_TOTAL
            .with_label_values(&[inst])
            .inc_by(delta);
    }
    state
        .last_background_errors
        .insert(inst, m.background_errors);

    for (level, count) in &m.files_per_level {
        ROCKSDB_FILES_AT_LEVEL
            .with_label_values(&[inst, &level.to_string()])
            .set(*count as i64);
    }
}

/// Spawn a periodic task that scrapes RocksDB runtime properties from
/// block_store and state_db every 15 seconds and updates the Prometheus gauges.
///
/// Phase 4: utxo_store was deleted; state_db is the sole UTXO store.
pub fn spawn_rocksdb_metrics_scraper(
    block_store: std::sync::Arc<storage::BlockStore>,
    state_db: std::sync::Arc<storage::StateDb>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Counter delta state retained across ticks. Task-local, so a process
        // restart resets it; Prometheus counter-reset detection handles that.
        let mut state = RocksDbScrapeState::new();
        loop {
            ticker.tick().await;
            apply_rocksdb_metrics(&mut state, &block_store.metrics());
            apply_rocksdb_metrics(&mut state, &state_db.metrics());
        }
    });
}

/// Spawn the F1 snap-sync UTXO size monitor as a background task.
///
/// Phase 5: computes `serialize_canonical_utxo().len()` at most once per
/// 60 seconds and updates the `doli_utxo_canonical_size_bytes` gauge.
/// The computation is O(UTXO count) but amortized by the 60s cache.
///
/// Alert rule (for Prometheus/Alertmanager — NOT enforced in code):
///   `doli_utxo_canonical_size_bytes > 12582912` for 5m → warning
///   (12 MB = 75% of MAX_SYNC_SIZE = 16 MB).
pub fn spawn_utxo_size_monitor(state_db: std::sync::Arc<storage::StateDb>) {
    let monitor = std::sync::Arc::new(storage::UtxoSizeMonitor::new(state_db));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let size = monitor.get_cached_size();
            UTXO_CANONICAL_SIZE_BYTES.set(size as i64);
        }
    });
}

/// HTTP handler for metrics endpoint
async fn metrics_handler() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();

    encoder
        .encode(&metric_families, &mut buffer)
        .expect("Failed to encode metrics");

    String::from_utf8(buffer).expect("Metrics are valid UTF-8")
}

/// Start the metrics HTTP server
pub async fn start_metrics_server(addr: SocketAddr) {
    register_metrics();

    let app = Router::new().route("/metrics", get(metrics_handler));

    info!("Starting metrics server on {}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!(
                "Failed to bind metrics server on {}: {} — continuing without metrics",
                addr, e
            );
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        warn!("Metrics server stopped: {}", e);
    }
}

/// Spawn the metrics server in a background task
pub fn spawn_metrics_server(addr: SocketAddr) {
    tokio::spawn(async move {
        start_metrics_server(addr).await;
    });
}

/// Record a block being processed
#[allow(dead_code)]
pub fn record_block_processed(valid: bool) {
    BLOCKS_PROCESSED.inc();
    if valid {
        BLOCKS_BY_STATUS.with_label_values(&["valid"]).inc();
    } else {
        BLOCKS_BY_STATUS.with_label_values(&["invalid"]).inc();
    }
}

/// Record a transaction being validated
#[allow(dead_code)]
pub fn record_transaction_validated(tx_type: &str, valid: bool) {
    TRANSACTIONS_VALIDATED.inc();
    TRANSACTIONS_BY_TYPE.with_label_values(&[tx_type]).inc();
    if valid {
        TRANSACTIONS_BY_RESULT.with_label_values(&["valid"]).inc();
    } else {
        TRANSACTIONS_BY_RESULT.with_label_values(&["invalid"]).inc();
    }
}

/// Update sync metrics
#[allow(dead_code)]
pub fn update_sync_metrics(progress: f64, syncing: bool, behind: u64) {
    SYNC_PROGRESS.set(progress);
    IS_SYNCING.set(if syncing { 1 } else { 0 });
    BLOCKS_BEHIND.set(behind as i64);
}

/// Update chain metrics
#[allow(dead_code)]
pub fn update_chain_metrics(height: u64, slot: u64) {
    CHAIN_HEIGHT.set(height as i64);
    CURRENT_SLOT.set(slot as i64);
}

/// Update network metrics
#[allow(dead_code)]
pub fn update_network_metrics(connected: usize) {
    PEERS_CONNECTED.set(connected as i64);
}

/// Update mempool metrics
#[allow(dead_code)]
pub fn update_mempool_metrics(count: usize, bytes: usize) {
    MEMPOOL_SIZE.set(count as i64);
    MEMPOOL_BYTES.set(bytes as i64);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_block_processed() {
        register_metrics();
        let before = BLOCKS_PROCESSED.get();
        record_block_processed(true);
        assert_eq!(BLOCKS_PROCESSED.get(), before + 1);
    }

    #[test]
    fn test_update_chain_metrics() {
        register_metrics();
        update_chain_metrics(12345, 67890);
        assert_eq!(CHAIN_HEIGHT.get(), 12345);
        assert_eq!(CURRENT_SLOT.get(), 67890);
    }

    #[test]
    fn test_metrics_handler() {
        register_metrics();
        CHAIN_HEIGHT.set(100);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let output = rt.block_on(metrics_handler());

        assert!(output.contains("doli_chain_height"));
    }

    /// INC-I-154 F1: attestation misses must be exported as Prometheus series, and both
    /// segment labels must exist from process start — a `*Vec` publishes nothing until a
    /// label value is touched, which would leave a "for 30m" alert with no series.
    #[test]
    fn inc_i_154_attestation_metrics_are_exported_with_both_segments() {
        register_metrics();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let output = rt.block_on(metrics_handler());

        for series in [
            "doli_attestation_missing_current{segment=\"scheduled\"}",
            "doli_attestation_missing_current{segment=\"unscheduled\"}",
            "doli_attestation_misses_total{segment=\"scheduled\"}",
            "doli_attestation_misses_total{segment=\"unscheduled\"}",
        ] {
            assert!(
                output.contains(series),
                "missing series {series} in /metrics output:\n{output}"
            );
        }
    }
}
