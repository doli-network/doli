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
//!   P2 (REQ-GRV-038) throwaway-key DelegateBond / RevokeDelegation at A ->
//!   refused at B, never arrives at C (counted on C's NewTransaction events)
//! INPUT PARTITIONS: type {DelegateBond, RevokeDelegation, AddMaintainer,
//!   RemoveMaintainer}; delegation parties {registered and active at B and C,
//!   throwaway}; params {auth AH 0, same 5-member set at B and C}.
//!   Non-vacuity: B's own publish of a fee-paying tx reaches C's mempool first;
//!   P2 waits for a registered marker published after the throwaways.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Transaction;
use network::NetworkCommand;

use super::inc_i_237_m11_common::{line, publish, pump_both};
use super::inc_i_237_m12_common as m12;
use super::inc_i_237_m8_common::{self as m8, Parties, Sig};

// REQ-GRV-029 — Decision: a failure means a genuine DelegateBond / RevokeDelegation / AddMaintainer / RemoveMaintainer submitted at a non-producer still stops at hop 1 and never reaches a block builder two hops away.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_029_genuine_zero_flow_txs_reach_a_node_two_hops_away() {
    let mut l = line().await;
    let rows: Vec<(&str, Transaction)> = vec![
        (
            "DelegateBond",
            m8::genuine_delegate_bond(&[&l.b, &l.c]).await,
        ),
        ("RevokeDelegation", m8::genuine_revoke(&[&l.b, &l.c]).await),
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

// REQ-GRV-038 / REQ-GRV-SEC-012 / REQ-GRV-SEC-013 — Decision: a failure means a validly signed delegation from a key no node has registered still crosses the first honest node (AUDIT-P1-515).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_038_throwaway_delegations_never_reach_c() {
    let mut l = line().await;
    let throwaway = Parties::throwaway();
    let rows: Vec<(&str, Transaction)> = vec![
        ("DelegateBond", m8::delegate_bond(&throwaway, Sig::Genuine)),
        ("RevokeDelegation", m8::revoke(&throwaway, Sig::Genuine)),
    ];
    let marker = m8::genuine_delegate_bond(&[&l.b, &l.c]).await;
    let mut watch: HashSet<Hash> = rows.iter().map(|(_, t)| t.hash()).collect();
    watch.insert(marker.hash());
    for (_, tx) in &rows {
        publish(&l, tx).await;
    }
    publish(&l, &marker).await;
    let mut arrived = HashSet::new();
    let reached = m12::pump_until_marker(&mut l, marker.hash(), &watch, &mut arrived, 30).await;
    assert!(reached, "setup: the registered marker never reached C");
    let mut bad = Vec::new();
    for (row, tx) in &rows {
        let (at_b, at_c) = (m8::has(&l.b, tx).await, arrived.contains(&tx.hash()));
        if at_b || at_c {
            bad.push(format!(
                "throwaway {row}: resident_at_B={at_b} arrived_at_C={at_c}"
            ));
        }
    }
    assert!(bad.is_empty(), "REQ-GRV-038: {bad:#?}");
}
