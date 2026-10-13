//! INC-I-237 M2 (D1) — multi-hop tx relay through the node verdict, on live
//! lines of real `NetworkService`s (127.0.0.1). Every relay node runs the fake
//! node loop (`inc_i_237_m2_fake_node`): admission into a `tx.hash()` set and
//! ONE verdict per message via `send_gossip_verdict`.
//!
//! OUTPUT CONTRACT: swarm tx path (Defer -> node verdict -> report on the
//!   ORIGINAL message id -> mesh forward)
//!   O1: none   O2: none   O3: none   O4: none   O5: none
//!   O6: NewTransaction events per node (one per gossip message, tags + time)
//! PATHS: P1 verdict Accept on time -> forwarded one hop further;
//!   P2 verdict withheld -> not forwarded until the verdict;
//!   P3 verdict Ignore (unadmittable) -> not forwarded;
//!   P4 verdict Ignore (all already resident) -> not forwarded
//! INPUT PARTITIONS: publisher {new node own-origin single tx, old-version raw
//!   peer: 0xBA batch of 1, 0xBA batch of 50}; content {new, unadmittable,
//!   resident}; composition {one message, M distinct re-batchings of K txs}
//! MATRIX: O6 at the far end x P1 (3 publisher partitions), O6 at C x P2,
//!   O6 at B and C x P3/P4 — all asserted. Topology is asserted (A and D keep
//!   exactly one peer) so a stray link cannot fake a relay.

use std::time::{Duration, Instant};

use network::gossip::encode_tx_batch;
use network::NetworkCommand;

use super::inc_i_237_m2_common as h;
use super::inc_i_237_m2_fake_node::{count, spawn_fake, Fake, Policy};

fn never_bad(_: u64) -> bool {
    false
}

/// Tags >= 9_000_000_000 are unadmittable at every fake node.
fn bad_range(t: u64) -> bool {
    t >= 9_000_000_000
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

/// Raw publishes of `data(i)` until `to` delivered one.
async fn warm_raw(r: &h::RawPeer, to: &Fake, base: u64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while to.count(base..base + 1_000) == 0 {
        assert!(
            Instant::now() < deadline,
            "setup: raw peer -> B never formed"
        );
        r.publish(encode_tx_batch(&[h::tx(base + i)]));
        i += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// B is the hub; `C` bootstraps to B; D bootstraps to C.
async fn b_c_d(policy: Policy, bad: fn(u64) -> bool) -> (u16, Fake, Fake, Fake) {
    let (b_port, c_port) = (h::reserve_port(), h::reserve_port());
    let b = spawn_fake(h::start_node(b_port, vec![]).await, policy, bad);
    let c = spawn_fake(
        h::start_node(c_port, vec![h::addr(b_port)]).await,
        policy,
        bad,
    );
    let d = spawn_fake(
        h::start_node(h::reserve_port(), vec![h::addr(c_port)]).await,
        policy,
        bad,
    );
    (b_port, b, c, d)
}

async fn assert_line(ends: &[(&str, &Fake)]) {
    for (name, f) in ends {
        assert_eq!(
            f.peer_count().await,
            1,
            "topology: {name} must have exactly one peer"
        );
    }
}

// REQ-GRV-001 — Decision: a failure means a valid tx from a new node stops after one hop again (the [E8] one-hop gap), so producers 2+ hops away never see it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_001_own_origin_single_tx_crosses_the_a_b_c_d_line() {
    let (b_port, b, c, d) = b_c_d(Policy::Judge, never_bad).await;
    let a = spawn_fake(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Policy::Judge,
        never_bad,
    );
    warm(&b, &[&a, &c], 51_000_000).await;
    warm(&c, &[&d], 52_000_000).await;
    let probe = 53_000_000;
    a.cmd
        .send(NetworkCommand::BroadcastTransaction(h::tx(probe)))
        .await
        .unwrap();
    let delivered = d.delivered.clone();
    h::wait_until(Duration::from_secs(5), || {
        count(&delivered, probe..probe + 1) > 0
    })
    .await;
    assert_eq!(
        c.count(probe..probe + 1),
        1,
        "C must receive the probe exactly once"
    );
    assert_eq!(
        d.count(probe..probe + 1),
        1,
        "REQ-GRV-001: A's tx must reach D (3 hops) exactly once"
    );
    assert_line(&[("A", &a), ("D", &d)]).await;
}

// REQ-GRV-001 / REQ-GRV-012 — Decision: a failure means a valid batch from an old-version node (0xBA, 1 or 50 txs) is not relayed past the first upgraded node, or is split into a second identity on the way.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_001_old_node_batches_of_1_and_50_cross_the_r_b_c_d_line() {
    let (b_port, b, c, d) = b_c_d(Policy::Judge, never_bad).await;
    let r = h::raw_peer(&h::addr(b_port)).await;
    warm_raw(&r, &b, 61_000_000).await;
    warm(&b, &[&c], 62_000_000).await;
    warm(&c, &[&d], 63_000_000).await;
    for (base, n) in [(64_000_000u64, 1u64), (65_000_000, 50)] {
        r.publish(encode_tx_batch(
            &(base..base + n).map(h::tx).collect::<Vec<_>>(),
        ));
        let delivered = d.delivered.clone();
        h::wait_until(Duration::from_secs(5), || {
            count(&delivered, base..base + n) > 0
        })
        .await;
        let at_d: Vec<Vec<u64>> = d
            .delivered
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t.iter().any(|x| (base..base + n).contains(x)))
            .map(|(t, _)| t.clone())
            .collect();
        assert_eq!(
            at_d,
            vec![(base..base + n).collect::<Vec<_>>()],
            "REQ-GRV-001: the {n}-tx batch must reach D as ONE message with every tx"
        );
    }
    assert_line(&[("D", &d)]).await;
}

// REQ-GRV-002 — Decision: a failure means B forwards a tx before its node judged it (the INC-I-237 flood path), or never forwards an admitted foreign tx after the verdict.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_002_c_sees_nothing_before_b_verdict_and_the_tx_after_it() {
    let (b_port, b, c, _d) = b_c_d(Policy::Judge, never_bad).await;
    let f = spawn_fake(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Policy::Judge,
        never_bad,
    );
    warm(&b, &[&f, &c], 71_000_000).await;
    b.set(Policy::Hold(Duration::from_millis(1_500)));
    let probe = 72_000_000;
    f.cmd
        .send(NetworkCommand::BroadcastTransaction(h::tx(probe)))
        .await
        .unwrap();
    let delivered = b.delivered.clone();
    assert!(
        h::wait_until(Duration::from_secs(5), || count(
            &delivered,
            probe..probe + 1
        ) > 0)
        .await,
        "setup: B never received F's tx"
    );
    let at_b = b.delivered.lock().unwrap().last().map(|(_, t)| *t).unwrap();
    tokio::time::sleep(
        (at_b + Duration::from_millis(1_000)).saturating_duration_since(Instant::now()),
    )
    .await;
    assert_eq!(
        c.count(probe..probe + 1),
        0,
        "REQ-GRV-002: C got F's tx before B's verdict"
    );
    tokio::time::sleep((at_b + Duration::from_secs(4)).saturating_duration_since(Instant::now()))
        .await;
    assert_eq!(
        c.count(probe..probe + 1),
        1,
        "REQ-GRV-002: B admitted F's tx, C must receive it"
    );
    assert_line(&[("F", &f)]).await;
}

// REQ-GRV-004 / F-5 — Decision: a failure means a publisher that re-wraps unadmittable or already-resident txs into new batches (new ids) gets them relayed past its first honest neighbour, the INC-I-237 amplification.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_004_rebatched_unadmittable_and_resident_content_stops_at_b() {
    let (b_port, b, c, _d) = b_c_d(Policy::Judge, bad_range).await;
    let r = h::raw_peer(&h::addr(b_port)).await;
    warm_raw(&r, &b, 81_000_000).await;
    warm(&b, &[&c], 82_000_000).await;
    let shapes: [&[u64]; 5] = [&[0, 1, 2], &[2, 1, 0], &[1, 0, 2], &[0, 2], &[1, 2]];
    let junk = 9_000_000_000u64;
    for s in shapes {
        r.publish(encode_tx_batch(
            &s.iter().map(|k| h::tx(junk + k)).collect::<Vec<_>>(),
        ));
    }
    let good = 83_000_000u64;
    r.publish(encode_tx_batch(&[
        h::tx(good + 2),
        h::tx(good),
        h::tx(good + 1),
    ]));
    let delivered = c.delivered.clone();
    assert!(
        h::wait_until(Duration::from_secs(5), || count(&delivered, good..good + 3)
            > 0)
        .await,
        "non-vacuity: B must relay the first, all-new batch"
    );
    for s in shapes {
        r.publish(encode_tx_batch(
            &s.iter().map(|k| h::tx(good + k)).collect::<Vec<_>>(),
        ));
    }
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        b.count(junk..junk + 3),
        5,
        "B delivers each of the M=5 re-batchings once"
    );
    assert_eq!(
        b.count(good..good + 3),
        6,
        "B delivers the first batch + 5 recompositions"
    );
    assert_eq!(
        c.count(junk..junk + 3),
        0,
        "REQ-GRV-004: unadmittable re-batchings crossed B"
    );
    assert_eq!(
        c.count(good..good + 3),
        1,
        "REQ-GRV-004: resident recompositions crossed B"
    );
}
