//! INC-I-237 M5 (AUDIT-P2-506) — the tx buckets are charged per tx, not per
//! gossip message. The swarm-path half runs on live links in
//! tests/it/inc_i_237_m5_tx_budget_line.rs.
//!
//! OUTPUT CONTRACT: fn RateLimiter::check_transaction(&PeerId, u64) -> bool;
//!   fn RateLimiter::record_transaction(&PeerId, u64, usize); contract doc §1c
//!   O3: bool   O5: per-peer and global tx bucket tokens (probed through
//!   check_transaction, with the refill slack computed from elapsed time)
//!   O1, O2, O4, O6: none
//! PATHS: P1 n within both buckets -> true; P2 n above the per-peer burst ->
//!   false, nothing charged; P3 global burst spent -> false for a fresh peer;
//!   P4 record n -> both buckets lose n; P5 refill at the per-peer rate
//! INPUT PARTITIONS: n {1, 100, 1,900, 2,000, 2,001, 10,000}; peer {fresh,
//!   charged, second, third}; elapsed {~0, 200 ms}
//! MATRIX: O3 x P1-P5 — asserted below.

use std::time::{Duration, Instant};

use libp2p::PeerId;

use crate::rate_limit::{RateLimitConfig, RateLimiter};

const PEER_RATE: f64 = 500.0;
const GLOBAL_RATE: f64 = 1_000.0;

fn limiter() -> RateLimiter {
    RateLimiter::new(RateLimitConfig::default())
}

/// Tokens a bucket of `rate`/s can have refilled since `t0`, rounded up.
fn slack(t0: Instant, rate: f64) -> u64 {
    (t0.elapsed().as_secs_f64() * rate).ceil() as u64 + 1
}

// REQ-GRV-SEC-003 (AUDIT-P2-506) — Decision: a failure means a 100-tx gossip message costs fewer than 100 tokens, so the tx budget bounds messages and a peer pushes 10,000 txs for one token.
#[test]
fn m5_audit_p2_506_tokens_are_charged_per_tx() {
    let mut rl = limiter();
    let p = PeerId::random();
    let t0 = Instant::now();
    assert!(rl.check_transaction(&p, 100), "fresh peer, 100-tx batch");
    rl.record_transaction(&p, 100, 100 * 250);
    assert!(
        rl.check_transaction(&p, 1_900),
        "1,900 tokens left after a 100-tx batch"
    );
    let over = 1_900 + slack(t0, PEER_RATE);
    assert!(
        !rl.check_transaction(&p, over),
        "AUDIT-P2-506: the 100-tx batch was charged < 100 tokens"
    );
}

// REQ-GRV-SEC-003 / REQ-GRV-017 (AUDIT-P2-506) — Decision: a failure means a batch larger than a peer's whole burst (up to the 10,000-tx decode cap) is admitted for validation, or a refused check still drains the bucket.
#[test]
fn m5_audit_p2_506_a_batch_above_the_peer_burst_is_refused_and_not_charged() {
    let mut rl = limiter();
    let p = PeerId::random();
    assert!(rl.check_transaction(&p, 2_000), "the whole burst fits once");
    assert!(!rl.check_transaction(&p, 2_001), "2,001 > burst 2,000");
    assert!(
        !rl.check_transaction(&p, 10_000),
        "10,000 txs cost 10,000 tokens"
    );
    assert!(
        rl.check_transaction(&p, 2_000),
        "a refused check charges nothing"
    );
}

// REQ-GRV-017 (AUDIT-P2-506) — Decision: a failure means the node-wide tx bucket no longer caps inbound validation across peers (CPU at the global cap), or its burst is below two full per-peer bursts.
#[test]
fn m5_audit_p2_506_global_tx_bucket_bounds_all_peers_together() {
    let mut rl = limiter();
    let (p1, p2, p3) = (PeerId::random(), PeerId::random(), PeerId::random());
    let t0 = Instant::now();
    for p in [p1, p2] {
        assert!(
            rl.check_transaction(&p, 2_000),
            "two full per-peer bursts fit the global burst"
        );
        rl.record_transaction(&p, 2_000, 2_000 * 250);
    }
    let want = 500;
    assert!(
        slack(t0, GLOBAL_RATE) < want,
        "setup: test too slow to probe the global bucket"
    );
    assert!(
        !rl.check_transaction(&p3, want),
        "a fresh peer still draws on the spent global bucket"
    );
}

// REQ-GRV-017 / REQ-GRV-SEC-003 (AUDIT-P2-511) — Decision: a failure means a peer's sustained tx budget fell back toward 50/s, so an honest origin's own txs are Ignored downstream while its RPC said success.
#[test]
fn m5_audit_p2_506_peer_bucket_refills_at_500_tx_per_s() {
    let mut rl = limiter();
    let p = PeerId::random();
    rl.record_transaction(&p, 2_000, 2_000 * 250);
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        rl.check_transaction(&p, 80),
        "200 ms at 500/s refills >= 100 tokens"
    );
    let over = slack(t0, PEER_RATE) + 1;
    assert!(!rl.check_transaction(&p, over), "refill faster than 500/s");
}
