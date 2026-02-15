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
}

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
    let _ = REGISTRY.register(Box::new(SLOT_LATENCY.clone()));

    let _ = REGISTRY.register(Box::new(UTXO_SET_SIZE.clone()));
    let _ = REGISTRY.register(Box::new(STORAGE_BYTES.clone()));

    let _ = REGISTRY.register(Box::new(UPTIME_SECONDS.clone()));
    let _ = REGISTRY.register(Box::new(BUILD_INFO.clone()));

    // Set build info
    BUILD_INFO
        .with_label_values(&[env!("CARGO_PKG_VERSION"), "unknown"])
        .set(1);
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
}
