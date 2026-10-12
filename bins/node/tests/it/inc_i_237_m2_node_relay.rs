//! INC-I-237 M2 (D1 / D5) — the REAL node at B on a live A -> B -> C line
//! (real `NetworkService`s, 127.0.0.1): B's tx gossip events go through the
//! production dispatch (`Node::handle_network_event`), which must admit, send
//! ONE verdict with try_send, and never re-publish received content.
//!
//! OUTPUT CONTRACT: Node::handle_network_event(NetworkEvent::NewTransaction(txs, handle))
//!   O1: none   O2: B's mempool (admission)   O3: Result (ignored)
//!   O4: none   O5: none
//!   O6: what C receives: tx gossip events (count per tx hash, raw bytes)
//! PATHS: P1 stamped message, all new -> Accept -> forwarded to C once, original bytes;
//!   P2 unstamped event (no gossip origin), all new -> admitted, nothing published;
//!   P3 forged-signature variant -> Ignore -> not forwarded; P4 the genuine
//!   tx after its forged variant -> forwarded
//! INPUT PARTITIONS: handle {stamped by B's swarm, unstamped}; content {new,
//!   forged variant first then genuine}; size {1, 2}
//! MATRIX: O6 x P1-P4 and O2 x P1-P3 — asserted below. Non-vacuity: B's own
//!   publish reaches C before every probe (the B -> C mesh is live).

use std::time::{Duration, Instant};

use doli_core::Transaction;
use doli_node::node::Node;
use network::{GossipHandle, NetworkCommand, NetworkEvent};
use tokio::sync::mpsc;

use super::inc_i_237_m2_node_common as c;

struct Line {
    b: Node,
    a_cmd: mpsc::Sender<NetworkCommand>,
    c_log: c::TxLog,
    kp: crypto::KeyPair,
    _t: tempfile::TempDir,
}

/// A -> B(node) -> C, warmed until B's own publish reaches C.
async fn line() -> Line {
    let (mut b, kp, t) = c::make_node().await;
    let b_port = c::reserve_port();
    b.network = Some(c::start_net(b_port, vec![]).await);
    let mut a = c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await;
    let a_cmd = a.command_sender();
    tokio::spawn(async move { while a.next_event().await.is_some() {} });
    let c_log = c::observe(c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0u64;
    while c_log.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "setup: B -> C mesh never formed");
        let warm = Transaction::new_coinbase(i, crypto::Hash::default(), i, 0);
        b.network
            .as_ref()
            .unwrap()
            .command_sender()
            .send(NetworkCommand::BroadcastTransaction(warm))
            .await
            .unwrap();
        i += 1;
        c::pump(&mut b, Duration::from_millis(400)).await;
    }
    Line {
        b,
        a_cmd,
        c_log,
        kp,
        _t: t,
    }
}

// REQ-GRV-001 / REQ-GRV-005 — Decision: a failure means the real node still turns an admitted foreign tx into a refused re-publish ([E8] Duplicate) instead of a verdict, so valid txs stop at one hop; or C gets it under a second identity.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_001_real_node_forwards_an_admitted_foreign_tx_once_with_original_bytes() {
    let mut l = line().await;
    c::seed(&l.b, &l.kp, c::prev(7)).await;
    let x = c::transfer(&l.kp, c::prev(7));
    l.a_cmd
        .send(NetworkCommand::BroadcastTransaction(x.clone()))
        .await
        .unwrap();
    c::pump(&mut l.b, Duration::from_secs(5)).await;
    assert!(
        l.b.mempool.read().await.contains(&x.hash()),
        "setup: B admitted A's tx"
    );
    let at_c = c::events_with(&l.c_log, x.hash());
    assert_eq!(
        at_c.len(),
        1,
        "REQ-GRV-001/005: C must get A's tx exactly once, got {}",
        at_c.len()
    );
    assert_eq!(
        at_c[0],
        vec![x.serialize()],
        "forwarded under the original bytes, alone"
    );
}

// REQ-GRV-005 / D1 — Decision: a failure means the node still re-publishes gossip-received txs (the removed 6.43.3 path), giving every relayed tx a second wire identity and re-opening re-batch amplification.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_005_gossip_received_admitted_txs_are_never_republished() {
    let mut l = line().await;
    c::seed(&l.b, &l.kp, c::prev(8)).await;
    c::seed(&l.b, &l.kp, c::prev(9)).await;
    let (y1, y2) = (
        c::transfer(&l.kp, c::prev(8)),
        c::transfer(&l.kp, c::prev(9)),
    );
    let ev = NetworkEvent::NewTransaction(vec![y1.clone(), y2.clone()], GossipHandle::now());
    let _ = l.b.handle_network_event(ev).await;
    c::pump(&mut l.b, Duration::from_secs(3)).await;
    let pool = l.b.mempool.read().await;
    assert!(
        pool.contains(&y1.hash()) && pool.contains(&y2.hash()),
        "setup: both admitted"
    );
    drop(pool);
    for y in [&y1, &y2] {
        let n = c::events_with(&l.c_log, y.hash()).len();
        assert_eq!(
            n, 0,
            "REQ-GRV-005: B re-published a gossip-received tx ({n} events at C)"
        );
    }
}

// REQ-GRV-SEC-001 / REQ-GRV-SEC-002 — Decision: a failure means a forged-signature tx crosses the first honest node, or its earlier arrival stops the genuine tx from being relayed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_sec_001_002_forged_variant_stops_at_b_genuine_still_crosses() {
    let mut l = line().await;
    c::seed(&l.b, &l.kp, c::prev(10)).await;
    let x = c::transfer(&l.kp, c::prev(10));
    let forged = c::malleate(&x);
    l.a_cmd
        .send(NetworkCommand::BroadcastTransaction(forged))
        .await
        .unwrap();
    c::pump(&mut l.b, Duration::from_secs(3)).await;
    assert_eq!(
        c::events_with(&l.c_log, x.hash()).len(),
        0,
        "SEC-001: forged variant crossed B"
    );
    l.a_cmd
        .send(NetworkCommand::BroadcastTransaction(x.clone()))
        .await
        .unwrap();
    c::pump(&mut l.b, Duration::from_secs(5)).await;
    let at_c = c::events_with(&l.c_log, x.hash());
    assert_eq!(at_c.len(), 1, "SEC-002: the genuine tx must still cross B");
    assert_eq!(at_c[0], vec![x.serialize()], "C got the genuine signature");
}
