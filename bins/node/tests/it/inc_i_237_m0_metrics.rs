//! INC-I-237 M0 (C5) — node gossip metrics: registration, observers, scrape.
//!
//! OUTPUT CONTRACT: fn register_metrics() / observe_gossip_handled(topic, &handle) /
//!   observe_block_recv_latency(recv_unix_ms, block_ts_secs) /
//!   observe_attestation_coverage(attesting, universe) /
//!   scrape_tx_batch_stats(&mut TxBatchScrapeState)
//!   O1: state (TxBatchScrapeState) — last snapshot, observed via a 2nd scrape
//!   O2: none   O3: none (all return ())   O4: none
//!   O5: REGISTRY series — verdict histogram{topic}, received counter{topic},
//!       block-recv histogram, coverage histogram, batch counters (deltas only:
//!       the registry is process-global, all writers hold `common::lock()`)
//!   O6: none
//! PATHS: P1 zero-init at registration, P2 tracked topic, P3 removed topics
//!   (headers/heartbeats, INC-I-237 M1 D3: no series), P4 recv after block ts, P5 recv before block ts
//!   (saturate 0), P6 universe > 0, P7 universe 0 (skip), P8 scrape with new
//!   records, P9 scrape with none (no double count)
//! INPUT PARTITIONS: 5 tracked topics + 2 removed labels; latency 1.5 s (bucket (1,2]);
//!   block delay 2.5 s / -1 s / block_ts u64::MAX; coverage 3/4, 5/5, 0/0;
//!   batches of 3 and 1
//! MATRIX: O5 x P1-P9 and O1 x {P8, P9} — every cell asserted below.

use std::time::{Duration, Instant};

use super::inc_i_237_m0_common::{self as c, block_recv, coverage, received, verdict};
use doli_node::metrics_gossip::{
    observe_attestation_coverage, observe_block_recv_latency, observe_gossip_handled,
    scrape_tx_batch_stats, TxBatchScrapeState,
};
use network::gossip::staleness::GossipTopic;
use network::gossip::tx_batch_stats::{TX_BATCH_BUCKETS, TX_GOSSIP_BATCH_STATS};
use network::GossipHandle;

const TRACKED: [(GossipTopic, &str); 5] = [
    (GossipTopic::Blocks, "blocks"),
    (GossipTopic::Transactions, "txs"),
    (GossipTopic::Attestations, "attestations"),
    (GossipTopic::Votes, "votes"),
    (GossipTopic::Producers, "producers"),
];
const VERDICT_BUCKETS: [f64; 14] = [
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 3.5, 5.0, 10.0, 30.0,
];
const BLOCK_RECV_BUCKETS: [f64; 14] = [
    0.1, 0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 7.5, 10.0, 15.0, 20.0, 30.0, 60.0,
];
const COVERAGE_BUCKETS: [f64; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 1.0];

fn handle_aged(ms: u64) -> GossipHandle {
    GossipHandle {
        recv_at: Instant::now()
            .checked_sub(Duration::from_millis(ms))
            .unwrap(),
    }
}

fn batch_msgs(size: &str) -> f64 {
    c::counter(c::BATCH_MSGS, &[("size", size)]).unwrap_or(0.0)
}

fn batch_txs() -> f64 {
    c::counter(c::BATCH_TXS, &[]).expect("batch txs counter registered")
}

// REQ-GRV-016 — Decision: a failure means a series the M0 gate, dashboards or the probe script read does not exist until its first sample, so "no data" and "0" are indistinguishable and alerts cannot bind.
#[test]
fn req_grv_016_register_metrics_zero_initialises_every_gossip_series() {
    let _g = c::lock();
    for (_, topic) in TRACKED {
        let h = verdict(topic);
        assert_eq!(
            h.bounds(),
            VERDICT_BUCKETS.to_vec(),
            "{topic}: verdict buckets (2 s SLO, 3.5 s R5, 5 s mcache)"
        );
    }
    for topic in ["blocks", "txs", "attestations", "votes", "producers"] {
        received(topic);
    }
    assert_eq!(block_recv().bounds(), BLOCK_RECV_BUCKETS.to_vec());
    assert_eq!(coverage().bounds(), COVERAGE_BUCKETS.to_vec());
    batch_txs();
}

// REQ-GRV-008 — Decision: a failure means a tracked topic's verdict latency is not observed or not measured from recv_at, so the p99 <= 2 s gate reads the wrong distribution.
#[test]
fn req_grv_008_observe_gossip_handled_counts_and_times_every_tracked_topic() {
    let _g = c::lock();
    for (topic, label) in TRACKED {
        let (n0, h0) = (received(label), verdict(label));
        observe_gossip_handled(topic, &handle_aged(1_500));
        let (n1, h1) = (received(label), verdict(label));
        assert_eq!(n1 - n0, 1.0, "{label}: received counter");
        assert_eq!(h1.count - h0.count, 1, "{label}: verdict histogram count");
        let dt = h1.sum - h0.sum;
        assert!(
            (1.5..5.0).contains(&dt),
            "{label}: observed {dt}s, want recv_at.elapsed() ~1.5 s"
        );
        assert_eq!(
            h1.cumulative_at(1.0),
            h0.cumulative_at(1.0),
            "{label}: 1.5 s is not <= 1 s"
        );
        assert_eq!(
            h1.cumulative_at(2.0) - h0.cumulative_at(2.0),
            1,
            "{label}: 1.5 s is <= 2 s"
        );
    }
}

// REQ-GRV-015 / REQ-GRV-008 — Decision: a failure means a headers/heartbeats series survives D3 (dead labels on dashboards and the M0 gate) or a GossipTopic lands outside the five subscribed-topic labels.
#[test]
fn req_grv_015_headers_and_heartbeats_have_no_series() {
    let _g = c::lock();
    for (topic, _) in TRACKED {
        observe_gossip_handled(topic, &handle_aged(10));
    }
    let want: Vec<&str> = TRACKED.iter().map(|(_, l)| *l).collect();
    for metric in [c::RECEIVED, c::VERDICT] {
        let mut got = c::label_values(metric, "topic");
        got.sort();
        let mut w: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        w.sort();
        assert_eq!(
            got, w,
            "{metric}: topic labels must be exactly the five subscribed topics"
        );
    }
}

// REQ-GRV-009 — Decision: a failure means block propagation latency (wall recv - block timestamp) is mis-measured, so the "not worse than baseline + 50 ms" check compares nothing.
#[test]
fn req_grv_009_block_recv_latency_is_wall_minus_block_timestamp_saturating_at_zero() {
    let _g = c::lock();
    let ts = 1_790_000_000u64;
    let b0 = block_recv();
    observe_block_recv_latency(ts * 1_000 + 2_500, ts);
    let b1 = block_recv();
    assert_eq!(b1.count - b0.count, 1);
    assert!(
        (b1.sum - b0.sum - 2.5).abs() < 1e-9,
        "2.5 s delay observed as {}",
        b1.sum - b0.sum
    );
    assert_eq!(
        b1.cumulative_at(2.0),
        b0.cumulative_at(2.0),
        "2.5 s is not <= 2 s"
    );
    assert_eq!(
        b1.cumulative_at(3.0) - b0.cumulative_at(3.0),
        1,
        "2.5 s is <= 3 s"
    );

    observe_block_recv_latency(ts * 1_000 - 1_000, ts);
    let b2 = block_recv();
    assert_eq!(
        b2.count - b1.count,
        1,
        "a block from the future is still observed"
    );
    assert_eq!(b2.sum, b1.sum, "recv before block_ts saturates to 0");
    assert_eq!(b2.cumulative_at(0.1) - b1.cumulative_at(0.1), 1);

    observe_block_recv_latency(0, u64::MAX);
    let b3 = block_recv();
    assert_eq!(
        (b3.count - b2.count, b3.sum),
        (1, b2.sum),
        "u64::MAX block_ts must not overflow"
    );
}

// REQ-GRV-010 — Decision: a failure means the attestation coverage baseline is wrong or divides by zero, so the "coverage does not drop under flood" check has no valid baseline.
#[test]
fn req_grv_010_attestation_coverage_ratio_and_empty_universe_skip() {
    let _g = c::lock();
    let c0 = coverage();
    observe_attestation_coverage(3, 4);
    let c1 = coverage();
    assert_eq!(c1.count - c0.count, 1);
    assert!((c1.sum - c0.sum - 0.75).abs() < 1e-9);
    assert_eq!(c1.cumulative_at(0.7), c0.cumulative_at(0.7));
    assert_eq!(c1.cumulative_at(0.8) - c0.cumulative_at(0.8), 1);

    observe_attestation_coverage(0, 0);
    observe_attestation_coverage(7, 0);
    assert_eq!(coverage(), c1, "universe 0 must not be observed");

    observe_attestation_coverage(5, 5);
    let c2 = coverage();
    assert_eq!(c2.count - c1.count, 1);
    assert!((c2.sum - c1.sum - 1.0).abs() < 1e-9);
    assert_eq!(
        c2.cumulative_at(0.99),
        c1.cumulative_at(0.99),
        "full coverage lands in le=1.0"
    );
    assert_eq!(c2.cumulative_at(1.0) - c1.cumulative_at(1.0), 1);
}

// REQ-GRV-016 — Decision: a failure means the batch counters miss swarm records or re-add them on every scrape, so the batch-size distribution and origin tx rate are wrong.
#[test]
fn req_grv_016_scrape_tx_batch_stats_adds_each_delta_exactly_once() {
    let _g = c::lock();
    let mut state = TxBatchScrapeState::new();
    scrape_tx_batch_stats(&mut state);
    let m0: Vec<f64> = TX_BATCH_BUCKETS.iter().map(|s| batch_msgs(s)).collect();
    let t0 = batch_txs();

    TX_GOSSIP_BATCH_STATS.record(3);
    TX_GOSSIP_BATCH_STATS.record(1);
    TX_GOSSIP_BATCH_STATS.record(0);
    scrape_tx_batch_stats(&mut state);
    let m1: Vec<f64> = TX_BATCH_BUCKETS.iter().map(|s| batch_msgs(s)).collect();
    let delta: Vec<f64> = m1.iter().zip(&m0).map(|(a, b)| a - b).collect();
    assert_eq!(
        delta,
        vec![1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "per-size deltas {TX_BATCH_BUCKETS:?}"
    );
    assert_eq!(batch_txs() - t0, 4.0);

    scrape_tx_batch_stats(&mut state);
    let m2: Vec<f64> = TX_BATCH_BUCKETS.iter().map(|s| batch_msgs(s)).collect();
    assert_eq!(m2, m1, "a scrape with no new records must add nothing");
    assert_eq!(batch_txs() - t0, 4.0);
}
