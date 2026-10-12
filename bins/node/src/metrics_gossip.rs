//! Gossip receive/verdict metrics (INC-I-237 M0). Observation only: nothing
//! here feeds a relay, validation or consensus decision.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use lazy_static::lazy_static;
use network::gossip::staleness::GossipTopic;
use network::gossip::tx_batch_stats::{TxBatchSnapshot, TX_BATCH_BUCKETS, TX_GOSSIP_BATCH_STATS};
use network::service::{LaneBudgets, LaneSnapshot, LaneStats, LATE_VERDICTS};
use network::GossipHandle;
use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, Opts, Registry,
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
const EVENT_TOPICS: [GossipTopic; 5] = [
    GossipTopic::Blocks,
    GossipTopic::Transactions,
    GossipTopic::Attestations,
    GossipTopic::Votes,
    GossipTopic::Producers,
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
    static ref LANE_BUDGET_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "doli_event_lane_budget_bytes",
            "Byte budget of each swarm-to-node event lane"
        ),
        &["lane"]
    )
    .unwrap();
    static ref LANE_INFLIGHT_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "doli_event_lane_inflight_bytes",
            "Bytes queued in each swarm-to-node event lane"
        ),
        &["lane"]
    )
    .unwrap();
    static ref VERDICT_LATE: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_gossip_verdict_late_total",
            "Node gossip verdicts that missed the 3.5 s window (counted, never reported), per topic."
        ),
        &["topic"]
    )
    .unwrap();
    static ref LANE_SHED: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "doli_event_lane_shed_total",
            "Events dropped by each event lane since start"
        ),
        &["lane"]
    )
    .unwrap();
    static ref LANE_DEPTH: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "doli_event_lane_depth",
            "Events queued in each swarm-to-node event lane"
        ),
        &["lane"]
    )
    .unwrap();
    static ref SHED_BY_TOPIC: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "doli_event_shed_by_topic_total",
            "Gossip events refused by a full event lane, per topic"
        ),
        &["topic"]
    )
    .unwrap();
}

/// Lane counters of the running network service, read at scrape time.
static EVENT_LANES: OnceLock<(Arc<LaneStats>, LaneBudgets)> = OnceLock::new();

/// Per-topic children, resolved once so the per-event path does no label lookup.
static RECEIVED_CHILDREN: [OnceLock<IntCounter>; 5] = [const { OnceLock::new() }; 5];
static VERDICT_CHILDREN: [OnceLock<Histogram>; 5] = [const { OnceLock::new() }; 5];

/// Scrape state of the `/metrics` handler.
static EXPOSITION_SCRAPE: Mutex<TxBatchScrapeState> = Mutex::new(TxBatchScrapeState::new());

fn slot(topic: GossipTopic) -> usize {
    match topic {
        GossipTopic::Blocks => 0,
        GossipTopic::Producers => 1,
        GossipTopic::Attestations => 2,
        GossipTopic::Votes => 3,
        GossipTopic::Transactions => 4,
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
    let _ = registry.register(Box::new(LANE_BUDGET_BYTES.clone()));
    let _ = registry.register(Box::new(LANE_INFLIGHT_BYTES.clone()));
    let _ = registry.register(Box::new(LANE_SHED.clone()));
    let _ = registry.register(Box::new(VERDICT_LATE.clone()));
    let _ = registry.register(Box::new(LANE_DEPTH.clone()));
    let _ = registry.register(Box::new(SHED_BY_TOPIC.clone()));
    for topic in EVENT_TOPICS {
        received(topic);
        verdict(topic);
        SHED_BY_TOPIC.with_label_values(&[topic.metric_label()]);
    }
    scrape_late_verdicts();
    for size in TX_BATCH_BUCKETS {
        TX_BATCH_MESSAGES.with_label_values(&[size]);
    }
}

/// Count one handled gossip event and time it from `recv_at`.
pub fn observe_gossip_handled(topic: GossipTopic, handle: &GossipHandle) {
    received(topic).inc();
    verdict(topic).observe(handle.recv_at.elapsed().as_secs_f64());
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

/// Publish lane budgets, in-flight bytes, depth and drop counts (gauges, replaced
/// each call); raise each per-topic shed counter to the lanes' monotone count.
pub fn observe_event_lanes(snap: &LaneSnapshot, budgets: &LaneBudgets) {
    let rows = [
        (
            "consensus",
            budgets.consensus_bytes,
            snap.consensus_bytes,
            snap.consensus_shed,
            snap.consensus_len,
        ),
        (
            "bulk",
            budgets.bulk_bytes,
            snap.bulk_bytes,
            snap.bulk_shed,
            snap.bulk_len,
        ),
        (
            "backlog",
            budgets.backlog_bytes,
            snap.backlog_bytes,
            snap.control_dropped,
            snap.backlog_len,
        ),
    ];
    for (lane, budget, inflight, shed, depth) in rows {
        LANE_BUDGET_BYTES
            .with_label_values(&[lane])
            .set(budget as i64);
        LANE_INFLIGHT_BYTES
            .with_label_values(&[lane])
            .set(inflight as i64);
        LANE_SHED.with_label_values(&[lane]).set(shed as i64);
        LANE_DEPTH.with_label_values(&[lane]).set(depth as i64);
    }
    for topic in EVENT_TOPICS {
        let c = SHED_BY_TOPIC.with_label_values(&[topic.metric_label()]);
        c.inc_by(snap.shed_by_topic.get(topic).saturating_sub(c.get()));
    }
}

/// Register the network service's lane counters for scrape-time refresh.
pub fn watch_event_lanes(stats: Arc<LaneStats>, budgets: LaneBudgets) {
    let _ = EVENT_LANES.set((stats, budgets));
}

/// Raise each per-topic late counter to the swarm's monotone count.
fn scrape_late_verdicts() {
    for topic in EVENT_TOPICS {
        let c = VERDICT_LATE.with_label_values(&[topic.metric_label()]);
        c.inc_by(LATE_VERDICTS.get(topic).saturating_sub(c.get()));
    }
}

/// Scrape the batch stats, late verdicts and lane counters into the registry before a `/metrics` exposition.
pub fn scrape_for_exposition() {
    let mut state = EXPOSITION_SCRAPE.lock().unwrap_or_else(|e| e.into_inner());
    scrape_tx_batch_stats(&mut state);
    scrape_late_verdicts();
    if let Some((stats, budgets)) = EVENT_LANES.get() {
        observe_event_lanes(&stats.snapshot(), budgets);
    }
}
