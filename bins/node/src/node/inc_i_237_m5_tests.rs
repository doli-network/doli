//! INC-I-237 M5 — the node's own-origin tx budget (AUDIT-P2-511) and the
//! digest re-publish margin past the vendor duplicate cache.
//!
//! OUTPUT CONTRACT: fn reserve_tx_broadcast(&Sender<NetworkCommand>,
//!   &Mutex<TokenBucket>) -> Option<TxBroadcastPermit> over Node.origin_tx_bucket
//!   (None = RPC Busy); fn digest_due(Option<&(Vec<u8>, Instant)>, &[u8]) -> bool
//!   O3: Option / bool   O5: origin bucket tokens   O1, O2, O4, O6: none here
//!   (slot rows stay in inc_i_237_m15_tests.rs); contract doc §1e, §1h
//! PATHS: reserve P1 tokens -> Some; P2 bucket empty -> None; P3 refill -> Some.
//!   digest P1 never sent / other bytes -> due; P2 same bytes within
//!   GOSSIP_DUPLICATE_CACHE_TIME + 30 s -> not due; P3 past it -> due
//! INPUT PARTITIONS: reservations {1..=2,000, 2,001st (+ refill slack)};
//!   digest age {10 s, ttl + 1 s, ttl + 29 s, ttl + 31 s}
//! MATRIX: O3 x every path — asserted below.

use std::time::{Duration, Instant};

use network::NetworkCommand;
use tokio::sync::mpsc;

use super::event_loop::digest_due;
use super::rpc_broadcast::reserve_tx_broadcast;
use super::Node;

// REQ-GRV-SEC-003 / REQ-GRV-017 (AUDIT-P2-511) — Decision: a failure means the RPC accepts own txs faster than every receiver's per-peer budget, so they are Ignored and dup-cached downstream while the submitter was told success.
#[tokio::test]
async fn m5_audit_p2_511_submit_above_the_origin_budget_returns_busy() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let node = Node::new_for_test(dir.path().to_path_buf(), vec![crypto::KeyPair::generate()])
        .await
        .expect("Node::new_for_test");
    let (cmd, _rx) = mpsc::channel::<NetworkCommand>(8);
    let t0 = Instant::now();
    let busy_at =
        (1..=6_000u64).find(|_| reserve_tx_broadcast(&cmd, &node.origin_tx_bucket).is_none());
    let slack = (t0.elapsed().as_secs_f64() * 500.0).ceil() as u64 + 1;
    let n = busy_at.expect("AUDIT-P2-511: 6,000 submits in one burst were never Busy");
    assert!(
        (2_001..=2_001 + slack).contains(&n),
        "first Busy at submit {n}, want 2,001 (+{slack} refill)"
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        reserve_tx_broadcast(&cmd, &node.origin_tx_bucket).is_some(),
        "the origin budget refills"
    );
}

fn sent(bytes: &[u8], ago: Duration) -> (Vec<u8>, Instant) {
    let at = Instant::now()
        .checked_sub(ago)
        .expect("monotonic clock past `ago`");
    (bytes.to_vec(), at)
}

// REQ-GRV-013 (digest WARN) — Decision: a failure means the node re-publishes a byte-identical digest the vendor duplicate cache still refuses (contains() keeps an expired id until the next insert), one refused publish + WARN per ~305 s.
#[test]
fn m5_digest_due_waits_for_the_duplicate_cache_plus_a_30_s_margin() {
    let ttl = network::gossip::GOSSIP_DUPLICATE_CACHE_TIME;
    let b = b"digest-bytes".to_vec();
    assert!(digest_due(None, &b), "never sent -> due");
    assert!(
        digest_due(Some(&sent(b"other", Duration::from_secs(10))), &b),
        "changed bytes -> due"
    );
    for age in [
        Duration::from_secs(10),
        ttl + Duration::from_secs(1),
        ttl + Duration::from_secs(29),
    ] {
        assert!(
            !digest_due(Some(&sent(&b, age)), &b),
            "same bytes {age:?} ago must wait"
        );
    }
    let past = ttl + Duration::from_secs(31);
    assert!(
        digest_due(Some(&sent(&b, past)), &b),
        "same bytes {past:?} ago are due"
    );
}
