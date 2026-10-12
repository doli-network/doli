//! INC-I-237 M8 / REQ-GRV-029 — a genuine zero-flow tx of each signed type,
//! published by a non-producer origin A, crosses the first honest node B and
//! reaches node C two hops away, where the block builder's selection takes it.
//! Live line A -> B(node) -> C(node), real `NetworkService`s on 127.0.0.1.
//!
//! OUTPUT CONTRACT: Node::handle_network_event(NetworkEvent::NewTransaction) at B and C
//!   O1: none   O2: B's and C's mempools   O3: Result (ignored)
//!   O4: none   O5: none   O6: B's forward to C (observed as C's admission)
//! PATHS: P1 genuine zero-flow at B -> admitted + Accept -> forwarded -> admitted
//!   at C -> in C's `select_for_block` (the builder input, assembly.rs)
//! INPUT PARTITIONS: type {DelegateBond, RevokeDelegation, AddMaintainer,
//!   RemoveMaintainer}; params {auth AH 0, same 5-member set at B and C}.
//!   Non-vacuity: B's own publish of a fee-paying tx reaches C's mempool first.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::time::{Duration, Instant};

use doli_core::Transaction;
use doli_node::node::Node;
use network::NetworkCommand;
use tokio::sync::mpsc;

use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common::{self as m8, Sig};

struct Line {
    a_cmd: mpsc::Sender<NetworkCommand>,
    b: Node,
    c: Node,
    m: Vec<crypto::KeyPair>,
    _t: (tempfile::TempDir, tempfile::TempDir),
}

async fn pump_both(l: &mut Line, d: Duration) {
    c::pump(&mut l.b, d).await;
    c::pump(&mut l.c, d).await;
}

/// A -> B -> C, warmed until B's own publish is admitted at C.
async fn line() -> Line {
    let m = m8::maintainer_keys();
    let (mut b, _kb, tb) = m8::auth_node_with(&m).await;
    let (mut cn, kc, tc) = m8::auth_node_with(&m).await;
    let b_port = c::reserve_port();
    b.network = Some(c::start_net(b_port, vec![]).await);
    cn.network = Some(c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await);
    let mut a = c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await;
    let a_cmd = a.command_sender();
    tokio::spawn(async move { while a.next_event().await.is_some() {} });
    let mut l = Line {
        a_cmd,
        b,
        c: cn,
        m,
        _t: (tb, tc),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    for tag in 100u8.. {
        assert!(Instant::now() < deadline, "setup: B -> C mesh never formed");
        c::seed(&l.c, &kc, c::prev(tag)).await;
        let warm = c::transfer(&kc, c::prev(tag));
        let b_cmd = l.b.network.as_ref().unwrap().command_sender();
        b_cmd
            .send(NetworkCommand::BroadcastTransaction(warm.clone()))
            .await
            .unwrap();
        pump_both(&mut l, Duration::from_millis(400)).await;
        if m8::has(&l.c, &warm).await {
            break;
        }
    }
    l
}

// REQ-GRV-029 — Decision: a failure means a genuine DelegateBond / RevokeDelegation / AddMaintainer / RemoveMaintainer submitted at a non-producer still stops at hop 1 and never reaches a block builder two hops away.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_029_genuine_zero_flow_txs_reach_a_node_two_hops_away() {
    let mut l = line().await;
    let rows: Vec<(&str, Transaction)> = vec![
        ("DelegateBond", m8::delegate_bond(Sig::Genuine)),
        ("RevokeDelegation", m8::revoke(Sig::Genuine)),
        ("AddMaintainer", m8::genuine_add(&l.b, &l.m).await),
        ("RemoveMaintainer", m8::genuine_remove(&l.b, &l.m).await),
    ];
    for (_, tx) in &rows {
        l.a_cmd
            .send(NetworkCommand::BroadcastTransaction(tx.clone()))
            .await
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        pump_both(&mut l, Duration::from_millis(500)).await;
        let mut all = true;
        for (_, tx) in &rows {
            all &= m8::has(&l.c, tx).await;
        }
        if all || Instant::now() > deadline {
            break;
        }
    }
    let selected: Vec<crypto::Hash> =
        l.c.mempool
            .read()
            .await
            .select_for_block(1 << 20)
            .iter()
            .map(|t| t.hash())
            .collect();
    let mut bad = Vec::new();
    for (row, tx) in &rows {
        let (at_b, at_c) = (m8::has(&l.b, tx).await, m8::has(&l.c, tx).await);
        let picked = selected.contains(&tx.hash());
        if !(at_b && at_c && picked) {
            bad.push(format!(
                "{row}: admitted_at_B={at_b} reached_C={at_c} in_C_block_selection={picked}"
            ));
        }
    }
    assert!(bad.is_empty(), "REQ-GRV-029 (2 hops): {bad:#?}");
}
