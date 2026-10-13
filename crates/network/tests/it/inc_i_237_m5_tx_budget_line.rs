//! INC-I-237 M5 (AUDIT-P2-506 / AUDIT-P2-511) — the swarm charges the tx
//! bucket per tx, on live links of real `NetworkService`s (127.0.0.1) with the
//! fake node loop judging every message.
//!
//! OUTPUT CONTRACT: swarm tx intake (`handle_gossip_message` ->
//!   `within_peer_budget` / `defer`); contract doc §1d
//!   O1-O5: none   O6: NewTransaction events per node (tags + time)
//! PATHS: P1 batch above the per-peer burst -> no event; P2 batch within the
//!   remaining budget -> one event; P3 second batch above the remaining per-tx
//!   budget -> no event; P4 100 own-origin msgs/s for 15 s -> >= 99 % at C
//! INPUT PARTITIONS: batch size {10, 1,000, 1,500, 2,001}; publisher {raw
//!   old-version peer, own-origin NetworkService}; rate {burst, 100/s}
//! MATRIX: O6 at B x P1-P3, O6 at C x P4 — asserted; every negative has a
//!   later positive on the same link (non-vacuity), topology asserted for P4.

use std::time::{Duration, Instant};

use network::gossip::encode_tx_batch;
use network::NetworkCommand;

use super::inc_i_237_m2_common as h;
use super::inc_i_237_m2_fake_node::{count, spawn_fake, Fake, Policy};

fn never_bad(_: u64) -> bool {
    false
}

fn batch(base: u64, n: u64) -> Vec<u8> {
    encode_tx_batch(&(base..base + n).map(h::tx).collect::<Vec<_>>())
}

/// A raw peer R linked to a fake node B, warmed until R's publish reaches B.
async fn r_to_b(base: u64) -> (h::RawPeer, Fake) {
    let b_port = h::reserve_port();
    let b = spawn_fake(
        h::start_node(b_port, vec![]).await,
        Policy::Judge,
        never_bad,
    );
    let r = h::raw_peer(&h::addr(b_port)).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while b.count(base..base + 1_000) == 0 {
        assert!(
            Instant::now() < deadline,
            "setup: raw peer -> B never formed"
        );
        r.publish(batch(base + i, 1));
        i += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    (r, b)
}

/// Own-origin publishes from `from` until each `to` delivered one of them.
async fn warm(from: &Fake, to: &[&Fake], base: u64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while to.iter().any(|f| f.count(base..base + 1_000) == 0) {
        assert!(
            Instant::now() < deadline,
            "setup: mesh link never formed (base {base})"
        );
        from.cmd
            .send(NetworkCommand::BroadcastTransaction(h::tx(base + i)))
            .await
            .unwrap();
        i += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

async fn delivered_within(f: &Fake, r: std::ops::Range<u64>, within: Duration) -> bool {
    let d = f.delivered.clone();
    h::wait_until(within, || count(&d, r.clone()) > 0).await
}

// REQ-GRV-SEC-003 / REQ-GRV-017 (AUDIT-P2-506) — Decision: a failure means one 0xBA message with more txs than a peer's whole burst is still validated inline (up to 10,000 x ~100 us of node-loop stall) for the price of one token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_audit_p2_506_a_2001_tx_batch_is_ignored_at_the_swarm() {
    let (r, b) = r_to_b(101_000_000).await;
    let (big, probe) = (102_000_000u64, 103_000_000u64);
    r.publish(batch(big, 2_001));
    r.publish(batch(probe, 1_000));
    let live = delivered_within(&b, probe..probe + 1_000, Duration::from_secs(10)).await;
    assert!(
        live,
        "non-vacuity: the 1,000-tx batch after it must reach B's node"
    );
    let n = b.count(big..big + 2_001);
    assert_eq!(n, 0, "AUDIT-P2-506: a 2,001-tx batch reached B's node");
}

// REQ-GRV-SEC-003 (AUDIT-P2-506) — Decision: a failure means a batch costs one token whatever its tx count, so the per-peer tx budget bounds messages, not txs, and a peer pushes its whole burst in every message.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_audit_p2_506_a_second_1500_tx_batch_exceeds_the_remaining_budget() {
    let (r, b) = r_to_b(111_000_000).await;
    let (first, second, probe) = (112_000_000u64, 113_000_000u64, 114_000_000u64);
    r.publish(batch(first, 1_500));
    r.publish(batch(second, 1_500));
    r.publish(batch(probe, 10));
    let live = delivered_within(&b, probe..probe + 10, Duration::from_secs(10)).await;
    assert!(
        live,
        "non-vacuity: a 10-tx batch after both must reach B's node"
    );
    // Arrival order is not publish order (the raw peer's queue drains LIFO),
    // so exactly one of the two batches fits the 2,000-tx burst.
    let n = b.count(first..first + 1_500) + b.count(second..second + 1_500);
    assert_eq!(
        n, 1,
        "AUDIT-P2-506: two 1,500-tx batches were charged below their tx count"
    );
}

// REQ-GRV-017 / REQ-GRV-SEC-003 (AUDIT-P2-511) — Decision: a failure means an origin publishing below the per-peer tx budget loses txs at the first and second hop (V: 50 tx/s per peer) while its RPC answered success.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_audit_p2_511_origin_publishes_100_tx_per_s_for_15_s_and_c_receives_99_pct() {
    let b_port = h::reserve_port();
    let b = spawn_fake(
        h::start_node(b_port, vec![]).await,
        Policy::Judge,
        never_bad,
    );
    let c = spawn_fake(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Policy::Judge,
        never_bad,
    );
    let a = spawn_fake(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Policy::Judge,
        never_bad,
    );
    warm(&b, &[&a, &c], 121_000_000).await;
    let (base, total) = (122_000_000u64, 1_500u64);
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    for i in 0..total {
        tick.tick().await;
        a.cmd
            .send(NetworkCommand::BroadcastTransaction(h::tx(base + i)))
            .await
            .unwrap();
    }
    let d = c.delivered.clone();
    h::wait_until(Duration::from_secs(10), || {
        count(&d, base..base + total) as u64 >= total
    })
    .await;
    let at_c = c.count(base..base + total) as u64;
    assert!(
        at_c * 100 >= total * 99,
        "AUDIT-P2-511: C received {at_c} of {total} (< 99 %)"
    );
    for (name, f) in [("A", &a), ("C", &c)] {
        assert_eq!(
            f.peer_count().await,
            1,
            "topology: {name} must have exactly one peer"
        );
    }
}
