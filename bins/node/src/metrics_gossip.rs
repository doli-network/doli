//! Gossip receive/verdict metrics (INC-I-237 M0). Observation only: nothing
//! here feeds a relay, validation or consensus decision.

use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use lazy_static::lazy_static;
use network::gossip::staleness::GossipTopic;
use network::gossip::tx_batch_stats::{TxBatchSnapshot, TX_BATCH_BUCKETS, TX_GOSSIP_BATCH_STATS};
use network::GossipHandle;
use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Opts, Registry,
};

/// 2 s = SLO, 3.5 s = R5 deadline, 5 s = mcache window.
const VERDICT_BUCKETS: [f64; 14] = [
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 3.5, 5.0, 10.0, 30.0,
];
const BLOCK_RECV_BUCKETS: [f64; 14] = [
    0.1, 0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 7.5, 10.0, 15.0, 20.0, 30.0, 60.0,
];
const COVERAGE_BUCKETS: [f64; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 1.0];

/// Every topic `NetworkEvent::gossip_topic` can return.
const EVENT_TOPICS: [GossipTopic; 7] = [
    GossipTopic::Blocks,
    GossipTopic::Transactions,
    GossipTopic::Attestations,
    GossipTopic::Votes,
    GossipTopic::Producers,
    GossipTopic::Headers,
    GossipTopic::Heartbeats,
];

lazy_static! {
    static ref VERDICT_LATENCY: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "doli_gossip_verdict_latency_seconds",
            "Receive to handler-done latency per gossip topic"
        )
        .buckets(VERDICT_BUCKETS.to_vec()),
        &["topic"]
    )
    .unwrap();
    static ref EVENTS_RECEIVED: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_gossip_events_received_total",
            "Gossip events handled by the node"
        ),
        &["topic"]
    )
    .unwrap();
    static ref BLOCK_RECV_LATENCY: Histogram = Histogram::with_opts(
        HistogramOpts::new(
            "doli_gossip_block_recv_latency_seconds",
            "Wall receive time minus block timestamp"
        )
        .buckets(BLOCK_RECV_BUCKETS.to_vec())
    )
    .unwrap();
    static ref ATTESTATION_COVERAGE: Histogram = Histogram::with_opts(
        HistogramOpts::new(
            "doli_attestation_coverage_ratio",
            "Attesting producers over the decode universe per applied block"
        )
        .buckets(COVERAGE_BUCKETS.to_vec())
    )
    .unwrap();
    static ref TX_BATCH_MESSAGES: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_gossip_tx_batch_messages_total",
            "Received tx batch messages by size"
        ),
        &["size"]
    )
    .unwrap();
    static ref TX_BATCH_TXS: IntCounter = IntCounter::new(
        "doli_gossip_tx_batch_txs_total",
        "Transactions received in tx batches"
    )
    .unwrap();
}

/// Per-topic children, resolved once so the per-event path does no label lookup.
static RECEIVED_CHILDREN: [OnceLock<IntCounter>; 8] = [const { OnceLock::new() }; 8];
static VERDICT_CHILDREN: [OnceLock<Histogram>; 8] = [const { OnceLock::new() }; 8];

/// Scrape state of the `/metrics` handler.
static EXPOSITION_SCRAPE: Mutex<TxBatchScrapeState> = Mutex::new(TxBatchScrapeState::new());

fn slot(topic: GossipTopic) -> usize {
    match topic {
        GossipTopic::Blocks => 0,
        GossipTopic::Tier1Blocks => 1,
        GossipTopic::Producers => 2,
        GossipTopic::Attestations => 3,
        GossipTopic::Heartbeats => 4,
        GossipTopic::Headers => 5,
        GossipTopic::Votes => 6,
        GossipTopic::Transactions => 7,
    }
}

fn received(topic: GossipTopic) -> &'static IntCounter {
    RECEIVED_CHILDREN[slot(topic)]
        .get_or_init(|| EVENTS_RECEIVED.with_label_values(&[topic.metric_label()]))
}

fn verdict(topic: GossipTopic) -> &'static Histogram {
    VERDICT_CHILDREN[slot(topic)]
        .get_or_init(|| VERDICT_LATENCY.with_label_values(&[topic.metric_label()]))
}

/// Register the gossip families and zero-initialise their label sets.
pub fn register_gossip_metrics(registry: &Registry) {
    let _ = registry.register(Box::new(VERDICT_LATENCY.clone()));
    let _ = registry.register(Box::new(EVENTS_RECEIVED.clone()));
    let _ = registry.register(Box::new(BLOCK_RECV_LATENCY.clone()));
    let _ = registry.register(Box::new(ATTESTATION_COVERAGE.clone()));
    let _ = registry.register(Box::new(TX_BATCH_MESSAGES.clone()));
    let _ = registry.register(Box::new(TX_BATCH_TXS.clone()));
    for topic in EVENT_TOPICS {
        received(topic);
    }
    for topic in EVENT_TOPICS.into_iter().filter(|t| t.latency_tracked()) {
        verdict(topic);
    }
    for size in TX_BATCH_BUCKETS {
        TX_BATCH_MESSAGES.with_label_values(&[size]);
    }
}

/// Count one handled gossip event; time it from `recv_at` iff the topic is tracked.
pub fn observe_gossip_handled(topic: GossipTopic, handle: &GossipHandle) {
    received(topic).inc();
    if topic.latency_tracked() {
        verdict(topic).observe(handle.recv_at.elapsed().as_secs_f64());
    }
}

/// Wall-clock Unix ms at which the swarm received the message.
pub fn recv_unix_ms(handle: &GossipHandle) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.saturating_sub(handle.recv_at.elapsed()).as_millis() as u64
}

/// Observe wall receive time minus block timestamp, saturating at 0.
pub fn observe_block_recv_latency(recv_unix_ms: u64, block_ts_secs: u64) {
    let delay_ms = recv_unix_ms.saturating_sub(block_ts_secs.saturating_mul(1_000));
    BLOCK_RECV_LATENCY.observe(delay_ms as f64 / 1_000.0);
}

/// Observe attesting / universe for one applied block; skipped for an empty universe.
pub fn observe_attestation_coverage(attesting: usize, universe: usize) {
    if universe > 0 {
        ATTESTATION_COVERAGE.observe(attesting as f64 / universe as f64);
    }
}

/// Last scraped `TX_GOSSIP_BATCH_STATS`: counters only take deltas.
pub struct TxBatchScrapeState {
    last: TxBatchSnapshot,
}

impl TxBatchScrapeState {
    pub const fn new() -> Self {
        Self {
            last: TxBatchSnapshot {
                msgs: 0,
                txs: 0,
                buckets: [0; 7],
            },
        }
    }
}

impl Default for TxBatchScrapeState {
    fn default() -> Self {
        Self::new()
    }
}

/// Add the batch stats recorded since the last scrape of `state`.
pub fn scrape_tx_batch_stats(state: &mut TxBatchScrapeState) {
    let now = TX_GOSSIP_BATCH_STATS.snapshot();
    for (i, size) in TX_BATCH_BUCKETS.iter().enumerate() {
        TX_BATCH_MESSAGES
            .with_label_values(&[size])
            .inc_by(now.buckets[i].saturating_sub(state.last.buckets[i]));
    }
    TX_BATCH_TXS.inc_by(now.txs.saturating_sub(state.last.txs));
    state.last = now;
}

/// Scrape the batch stats into the registry before a `/metrics` exposition.
pub fn scrape_for_exposition() {
    scrape_tx_batch_stats(&mut EXPOSITION_SCRAPE.lock().unwrap_or_else(|e| e.into_inner()));
}
