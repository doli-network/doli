//! INC-I-237 M4 — INV-NETWORK-002 queue half: `LaneBudgets::is_bounded` is the
//! value the hardening gate reads instead of the literal `true`.
//!
//! covers: event_sink config
//!
//! OUTPUT CONTRACT: fn LaneBudgets::is_bounded(&self) -> bool
//!   O1: none   O2: none (takes &self)   O4: none   O5: none   O6: none
//!   O3: true iff every one of the six budgets is > 0 and < usize::MAX
//! PATHS: P1 all six finite and positive -> true; P2 one budget 0 -> false;
//!   P3 one budget usize::MAX -> false
//! INPUT PARTITIONS: production for_max_peers(n) for n in {0, 1, 8, 50, 10_000,
//!   usize::MAX} (slot clamp low/mid/high, saturating mul); each of the six
//!   fields alone at 0 and alone at usize::MAX
//! MATRIX: O3 x (6 production + 6 fields x 2 bad values) = 18 cells, all asserted.

use network::service::LaneBudgets;

type Setter = fn(&mut LaneBudgets, usize);

const FIELDS: [(&str, Setter); 6] = [
    ("consensus_cap", |b, v| b.consensus_cap = v),
    ("bulk_cap", |b, v| b.bulk_cap = v),
    ("consensus_bytes", |b, v| b.consensus_bytes = v),
    ("bulk_bytes", |b, v| b.bulk_bytes = v),
    ("backlog_bytes", |b, v| b.backlog_bytes = v),
    ("backlog_cap", |b, v| b.backlog_cap = v),
];

// REQ-GRV-013 — Decision: a failure means a production lane budget reads as unbounded, so the INV-NETWORK-002 construction gate would refuse the real node config.
#[test]
fn req_grv_013_production_lane_budgets_are_bounded() {
    for peers in [0, 1, 8, 50, 10_000, usize::MAX] {
        assert!(
            LaneBudgets::for_max_peers(peers).is_bounded(),
            "max_peers={peers}"
        );
    }
}

// REQ-GRV-013 — Decision: a failure means a zero or usize::MAX lane budget still reads as bounded, so the hardening gate passes a queue that cannot shed or can grow without limit.
#[test]
fn req_grv_013_zero_or_unlimited_budget_is_not_bounded() {
    let base = LaneBudgets::for_max_peers(8);
    assert!(base.is_bounded(), "precondition: base budgets are bounded");
    for (name, set) in FIELDS {
        for bad in [0, usize::MAX] {
            let mut b = base;
            set(&mut b, bad);
            assert!(!b.is_bounded(), "{name}={bad} must not be bounded");
        }
    }
}
