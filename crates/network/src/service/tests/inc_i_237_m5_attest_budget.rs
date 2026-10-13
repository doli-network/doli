//! INC-I-237 M5 decision C — attestations draw on their own per-peer bucket
//! (20/s, burst 200, the request bucket's shape); votes and producer gossip
//! keep the request bucket. Live-link half: tests/it/inc_i_237_m5_attest_budget_line.rs.
//!
//! OUTPUT CONTRACT: fn RateLimiter::check_attestation(&PeerId) -> bool;
//!   fn RateLimiter::record_attestation(&PeerId, usize); contract doc §C1
//!   O3: bool   O5: per-peer attestation and request bucket tokens (probed
//!   through check_attestation / check_request)   O1, O2, O4, O6: none
//! PATHS: P1 request bucket empty -> attestation still within budget; P2
//!   attestation burst spent -> refused, request bucket untouched, other peer
//!   untouched; P3 refill after the burst
//! INPUT PARTITIONS: charges {0, 200 + refill slack}; peer {drained, fresh};
//!   elapsed {~0, 250 ms}
//! MATRIX: O3 x P1-P3 — asserted below.

use std::time::{Duration, Instant};

use libp2p::PeerId;

use crate::rate_limit::{RateLimitConfig, RateLimiter};

const RATE: f64 = 20.0;
const BURST: u32 = 200;
const ATT_BYTES: usize = 300;

fn limiter() -> RateLimiter {
    RateLimiter::new(RateLimitConfig::default())
}

/// Tokens a bucket of `rate`/s can have refilled since `t0`, rounded up.
fn slack(t0: Instant, rate: f64) -> u32 {
    (t0.elapsed().as_secs_f64() * rate).ceil() as u32 + 1
}

// REQ-GRV-SEC-002 / REQ-GRV-SEC-003 (decision C) — Decision: a failure means vote variants or producer gossip that empty a peer's request bucket also refuse that peer's genuine attestations (soak #21 delivery 0.70).
#[test]
fn m5_decision_c_a_drained_request_bucket_leaves_attestations_within_budget() {
    let mut rl = limiter();
    let p = PeerId::random();
    let mut spent = 0u32;
    while rl.check_request(&p) {
        rl.record_request(&p, ATT_BYTES);
        spent += 1;
        assert!(spent <= 10_000, "the request bucket never empties");
    }
    assert!(
        spent >= BURST,
        "non-vacuity: request burst {spent} < {BURST}"
    );
    assert!(
        rl.check_attestation(&p),
        "decision C: the attestation budget shares the request bucket"
    );
}

// REQ-GRV-SEC-002 / REQ-GRV-SEC-003 (decision C) — Decision: a failure means the attestation bucket is unbounded, never refills, shares tokens with votes / producers, or is shared between peers.
#[test]
fn m5_decision_c_attestations_spend_only_their_own_per_peer_bucket() {
    let mut rl = limiter();
    let (p, q) = (PeerId::random(), PeerId::random());
    let t0 = Instant::now();
    for i in 0..BURST {
        assert!(rl.check_attestation(&p), "burst: attestation {i} refused");
        rl.record_attestation(&p, ATT_BYTES);
    }
    let cap = slack(t0, RATE);
    let mut extra = 0u32;
    while rl.check_attestation(&p) {
        rl.record_attestation(&p, ATT_BYTES);
        extra += 1;
        assert!(extra <= cap, "attestation bucket exceeds burst {BURST}");
    }
    assert!(
        rl.check_request(&p),
        "decision C: attestations spent the vote / producer request bucket"
    );
    assert!(rl.check_attestation(&q), "per peer: q shares p's bucket");
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        rl.check_attestation(&p),
        "the attestation bucket must refill"
    );
}
