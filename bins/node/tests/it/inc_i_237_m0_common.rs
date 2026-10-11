//! INC-I-237 M0 — shared registry readers for the gossip metrics modules.
//!
//! OUTPUT CONTRACT: N/A — fixture file; it declares no `#[test]`.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! The gossip metrics are process-global: every module that moves them takes
//! `lock()` and asserts deltas.

#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard};

use doli_node::metrics::REGISTRY;

pub const VERDICT: &str = "doli_gossip_verdict_latency_seconds";
pub const RECEIVED: &str = "doli_gossip_events_received_total";
pub const BLOCK_RECV: &str = "doli_gossip_block_recv_latency_seconds";
pub const COVERAGE: &str = "doli_attestation_coverage_ratio";
pub const BATCH_MSGS: &str = "doli_gossip_tx_batch_messages_total";
pub const BATCH_TXS: &str = "doli_gossip_tx_batch_txs_total";

static GOSSIP_METRICS: Mutex<()> = Mutex::new(());

pub fn lock() -> MutexGuard<'static, ()> {
    super::inc_i_204_m0_common::ensure_registered();
    GOSSIP_METRICS.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hist {
    pub count: u64,
    pub sum: f64,
    /// (upper bound, cumulative count), finite buckets only.
    pub buckets: Vec<(f64, u64)>,
}

impl Hist {
    pub fn cumulative_at(&self, le: f64) -> u64 {
        self.buckets
            .iter()
            .find(|(b, _)| *b == le)
            .map(|(_, c)| *c)
            .expect("bucket bound")
    }

    pub fn bounds(&self) -> Vec<f64> {
        self.buckets.iter().map(|(b, _)| *b).collect()
    }
}

fn labels_match(have: &[(String, String)], want: &[(&str, &str)]) -> bool {
    have.len() == want.len()
        && want
            .iter()
            .all(|(k, v)| have.iter().any(|(hk, hv)| hk == k && hv == v))
}

pub fn hist(name: &str, labels: &[(&str, &str)]) -> Option<Hist> {
    for mf in REGISTRY.gather() {
        if mf.get_name() != name {
            continue;
        }
        for m in mf.get_metric() {
            let have: Vec<(String, String)> = m
                .get_label()
                .iter()
                .map(|l| (l.get_name().to_string(), l.get_value().to_string()))
                .collect();
            if labels_match(&have, labels) {
                let h = m.get_histogram();
                let buckets = h
                    .get_bucket()
                    .iter()
                    .map(|b| (b.get_upper_bound(), b.get_cumulative_count()))
                    .filter(|(b, _)| b.is_finite())
                    .collect();
                return Some(Hist {
                    count: h.get_sample_count(),
                    sum: h.get_sample_sum(),
                    buckets,
                });
            }
        }
    }
    None
}

pub fn counter(name: &str, labels: &[(&str, &str)]) -> Option<f64> {
    for mf in REGISTRY.gather() {
        if mf.get_name() != name {
            continue;
        }
        for m in mf.get_metric() {
            let have: Vec<(String, String)> = m
                .get_label()
                .iter()
                .map(|l| (l.get_name().to_string(), l.get_value().to_string()))
                .collect();
            if labels_match(&have, labels) {
                return Some(m.get_counter().get_value());
            }
        }
    }
    None
}

pub fn label_values(name: &str, label: &str) -> Vec<String> {
    super::inc_i_204_m0_common::exported_label_values(name, label)
}

pub fn received(topic: &str) -> f64 {
    counter(RECEIVED, &[("topic", topic)])
        .unwrap_or_else(|| panic!("{RECEIVED}{{topic={topic}}} missing"))
}

pub fn verdict(topic: &str) -> Hist {
    hist(VERDICT, &[("topic", topic)])
        .unwrap_or_else(|| panic!("{VERDICT}{{topic={topic}}} missing"))
}

pub fn block_recv() -> Hist {
    hist(BLOCK_RECV, &[]).expect("block recv histogram registered")
}

pub fn coverage() -> Hist {
    hist(COVERAGE, &[]).expect("attestation coverage histogram registered")
}
