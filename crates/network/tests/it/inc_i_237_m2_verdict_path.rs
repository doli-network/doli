//! INC-I-237 M2 (R5 / C6 / F-3 / F-13) — the verdict path on the wire: what a
//! late, duplicate, absent or post-shutdown verdict does to forwarding.
//! Forwarding is the observable of `report_message_validation_result`: the
//! vendor holds a deferred message un-forwarded in mcache (>= ~4 s) until an
//! Accept report, so ANY report at 3.9 s would still forward it.
//!
//! OUTPUT CONTRACT: swarm GossipVerdict command arm + gossip intake on /doli/txs/1
//!   O1: none   O2: none   O3: send_gossip_verdict bool (shutdown row)
//!   O4: none
//!   O5: LATE_VERDICTS[txs] (process-global: asserted as lower-bound deltas)
//!   O6: NewTransaction events at B (node side) and at C (one hop further)
//! PATHS: P1 late Accept (3.9 s) -> not forwarded, counted;
//!   P2 duplicate verdict Ignore then Accept -> not forwarded;
//!   P3 duplicate verdict Accept then Accept -> forwarded exactly once;
//!   P4 no verdict ever -> not forwarded after the mcache window;
//!   P5 swarm shut down -> verdict send refused at once (no report possible)
//! INPUT PARTITIONS: verdict timing {on time, late, never}; verdict count
//!   {1, 2 conflicting, 2 equal}; flood size {within, beyond the per-peer
//!   tx bucket}; swarm {running, exited}
//! MATRIX: O6(C) x P1-P4, O5 x P1/P4, O6(B) x P4 (per-peer share), O3 x P5.

use std::time::{Duration, Instant};

use network::gossip::staleness::GossipTopic;
use network::service::{send_gossip_verdict, Verdict, LATE_VERDICTS};
use network::{GossipHandle, NetworkCommand};

use super::inc_i_237_m2_common as h;
use super::inc_i_237_m2_fake_node::{count, spawn_fake, Fake, Policy};

fn never_bad(_: u64) -> bool {
    false
}

/// F -- B -- C with every node judging; returns once F->B and B->C are warm.
async fn f_b_c(base: u64) -> (Fake, Fake, Fake) {
    let b_port = h::reserve_port();
    let b = spawn_fake(
        h::start_node(b_port, vec![]).await,
        Policy::Judge,
        never_bad,
    );
    let f = spawn_fake(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Policy::Judge,
        never_bad,
    );
    let c = spawn_fake(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Policy::Judge,
        never_bad,
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while f.count(base..base + 1_000) == 0 || c.count(base..base + 1_000) == 0 {
        assert!(Instant::now() < deadline, "setup: F-B-C never formed");
        b.cmd
            .send(NetworkCommand::BroadcastTransaction(h::tx(base + i)))
            .await
            .unwrap();
        i += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    (f, b, c)
}

async fn publish_and_wait_at_b(f: &Fake, b: &Fake, tag: u64) {
    f.cmd
        .send(NetworkCommand::BroadcastTransaction(h::tx(tag)))
        .await
        .unwrap();
    let d = b.delivered.clone();
    assert!(
        h::wait_until(Duration::from_secs(5), || count(&d, tag..tag + 1) > 0).await,
        "setup: B never received probe {tag}"
    );
}

// F-3 / REQ-GRV-008 — Decision: a failure means a verdict that missed the 3.5 s window is still reported, so a backlogged node relays stale content and the late counter (the operator's only signal) stays blind.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_3_late_accept_is_not_forwarded_and_is_counted() {
    let (f, b, c) = f_b_c(91_000_000).await;
    b.set(Policy::Hold(Duration::from_millis(3_900)));
    let late0 = LATE_VERDICTS.get(GossipTopic::Transactions);
    let probe = 92_000_000;
    publish_and_wait_at_b(&f, &b, probe).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        c.count(probe..probe + 1),
        0,
        "F-3: a late Accept was reported and forwarded"
    );
    assert!(
        LATE_VERDICTS.get(GossipTopic::Transactions) > late0,
        "R5: the late verdict was not counted"
    );
}

// F-3 — Decision: a failure means a second verdict on one message changes the relay outcome (a later Accept resurrects an Ignored message, or two Accepts send it twice).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_3_duplicate_verdicts_never_change_the_first_outcome() {
    let (f, b, c) = f_b_c(93_000_000).await;
    b.set(Policy::Twice(Verdict::Ignore, Verdict::Accept));
    let ignored = 94_000_000;
    publish_and_wait_at_b(&f, &b, ignored).await;
    b.set(Policy::Twice(Verdict::Accept, Verdict::Accept));
    let accepted = 94_500_000;
    publish_and_wait_at_b(&f, &b, accepted).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        c.count(ignored..ignored + 1),
        0,
        "F-3: Ignore then Accept forwarded the message"
    );
    assert_eq!(
        c.count(accepted..accepted + 1),
        1,
        "F-3: Accept twice must forward exactly once"
    );
}

// F-3 / F-13 / REQ-GRV-008 — Decision: a failure means deferred tx messages outlive the vendor mcache window in some new pending store (a verdict after 6 s still forwards), or a flood beyond the per-peer tx bucket reaches the node instead of being Ignored at the swarm.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_13_unjudged_flood_is_bounded_by_the_peer_share_and_expires() {
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
    let r = h::raw_peer(&h::addr(b_port)).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while b.count(95_000_000..95_001_000) == 0 || c.count(95_500_000..95_501_000) == 0 {
        assert!(Instant::now() < deadline, "setup: R-B-C never formed");
        r.publish(h::tx(95_000_000 + i).serialize());
        b.cmd
            .send(NetworkCommand::BroadcastTransaction(h::tx(95_500_000 + i)))
            .await
            .unwrap();
        i += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    b.set(Policy::Never);
    let (flood, n) = (96_000_000u64, 700u64);
    let late0 = LATE_VERDICTS.get(GossipTopic::Transactions);
    for k in 0..n {
        r.publish(h::tx(flood + k).serialize());
    }
    tokio::time::sleep(Duration::from_secs(6)).await;
    let at_b = b.count(flood..flood + n);
    assert!(at_b > 0, "non-vacuity: the flood reached B's node");
    assert!(
        (at_b as u64) < n,
        "F-13: all {n} messages of one peer reached the node; the per-peer tx bucket must Ignore its excess at the swarm"
    );
    assert_eq!(
        c.count(flood..flood + n),
        0,
        "F-3: an unjudged message was forwarded"
    );
    let kept: Vec<GossipHandle> = std::mem::take(&mut *b.kept.lock().unwrap());
    let judged = kept.len() as u64;
    for handle in kept {
        send_gossip_verdict(&b.cmd, handle, Verdict::Accept);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        c.count(flood..flood + n),
        0,
        "F-13: a verdict after the window forwarded a message"
    );
    assert!(
        LATE_VERDICTS.get(GossipTopic::Transactions) - late0 >= judged,
        "R5: every expired verdict must be counted late"
    );
}

// F-11 / F-3 — Decision: a failure means a verdict sent after the swarm exited blocks or panics the node instead of being refused, so shutdown can hang on a verdict.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_11_verdict_after_swarm_exit_is_refused_without_waiting() {
    let svc = h::start_node(h::reserve_port(), vec![]).await;
    let cmd = svc.command_sender();
    assert!(
        send_gossip_verdict(&cmd, GossipHandle::now(), Verdict::Accept),
        "running swarm takes it"
    );
    drop(svc);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let t = Instant::now();
        let sent = send_gossip_verdict(&cmd, GossipHandle::now(), Verdict::Accept);
        assert!(t.elapsed() < Duration::from_millis(50), "the send waited");
        if !sent {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "swarm never exited after its node side was dropped"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
