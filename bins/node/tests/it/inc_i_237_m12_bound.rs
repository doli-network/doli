//! INC-I-237 M12.1 — at most one resident DelegateBond / RevokeDelegation per
//! delegator (REQ-GRV-SEC-014), and a delegation tx admitted without a
//! producer-set view is refused (REQ-GRV-SEC-015, fail closed).
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict and
//!   Mempool::add_system_transaction(tx, height, Option<&ZeroFlowSnapshot>) -> Result<Hash, MempoolError>
//!   O1: none   O2: node.mempool membership / count of the delegator's residents
//!   O3: Verdict::{Accept, Ignore}; Ok(txid) | Err
//!   O4: none   O5: none   O6: none (network = None)
//! PATHS: P1 50 admissible DelegateBonds from one delegator, one per message ->
//!   1 Accept, 1 resident; P2 that resident evicted -> the next admissible one
//!   Accept + resident; P3 delegation tx with snapshot None -> Err; P4 snapshot
//!   built with producers = None -> Err; P5 (GUARD) same tx with the full
//!   snapshot -> Ok; P6 (GUARD) maintainer change with producers = None -> Ok
//! INPUT PARTITIONS: delegates {50 distinct registered active}; snapshot {None,
//!   producers None, full}; type {DelegateBond, RevokeDelegation, AddMaintainer};
//!   params {testnet: auth AH 0, tip 30_000}.
// covers: zero_flow_policy zero_flow_auth zero_flow_newness zero_flow_standing pool lib transaction

use doli_core::Transaction;
use mempool::zero_flow_auth::snapshot_for;
use network::Verdict;

use super::inc_i_237_m12_common::{delegate_tx, revoke_tx, world, BONDS};
use super::inc_i_237_m8_common as m8;

const BURST: usize = 50;

// REQ-GRV-SEC-014 — Decision: a failure means one registered delegator still mints one fee-free Accept per (active delegate x available bond), because admission ignores its own residents.
#[tokio::test]
async fn m12_sec_014_one_resident_delegation_per_delegator() {
    let w = world().await;
    let d = w.delegator(None).await;
    let mut txs: Vec<Transaction> = Vec::new();
    for _ in 0..BURST {
        let to = w.producer(1).await;
        txs.push(delegate_tx(&d, to.public_key(), 1));
    }
    for tx in &txs {
        assert!(
            w.boundary_effect(tx).await,
            "fixture: each one alone is a boundary-applied delegation"
        );
    }
    let mut accepts = 0;
    for tx in &txs {
        accepts += usize::from(w.gossip(tx).await.0 == Verdict::Accept);
    }
    let pool = w.node.mempool.read().await;
    let resident: Vec<&Transaction> = txs.iter().filter(|t| pool.contains(&t.hash())).collect();
    let residents = resident.len();
    let first = resident.first().map(|t| (*t).clone());
    drop(pool);
    assert!(
        accepts == 1 && residents == 1,
        "REQ-GRV-SEC-014: {BURST} DelegateBonds from one delegator gave accepts={accepts} residents={residents}"
    );
    w.evict(&first.expect("one resident")).await;
    let next = delegate_tx(&d, w.producer(1).await.public_key(), 1);
    let (v, admitted) = w.gossip(&next).await;
    assert!(
        v == Verdict::Accept && admitted,
        "REQ-GRV-SEC-014: after the resident left, the next admissible one: verdict={v:?} admitted={admitted}"
    );
}

/// Genuine delegation txs the full snapshot admits: (row, tx).
async fn admissible(w: &super::inc_i_237_m12_common::World) -> Vec<(&'static str, Transaction)> {
    let fresh = w.delegator(None).await;
    let held = w.delegator(Some(&w.p)).await;
    vec![
        ("DelegateBond", delegate_tx(&fresh, w.q.public_key(), BONDS)),
        ("RevokeDelegation", revoke_tx(&held, w.p.public_key())),
    ]
}

// REQ-GRV-SEC-015 — Decision: a failure means a caller that passes no snapshot admits delegation txs with no standing check at all, re-opening AUDIT-P1-515 through any new call site.
#[tokio::test]
async fn m12_sec_015_delegation_without_a_snapshot_is_refused() {
    let w = world().await;
    let h = w.node.chain_state.read().await.best_height;
    let mut bad = Vec::new();
    for (row, tx) in admissible(&w).await {
        let res = w
            .node
            .mempool
            .write()
            .await
            .add_system_transaction(tx.clone(), h, None);
        if res.is_ok() {
            bad.push(format!("{row}: admitted with snapshot None"));
        }
        w.evict(&tx).await;
    }
    assert!(bad.is_empty(), "REQ-GRV-SEC-015: {bad:#?}");
}

// REQ-GRV-SEC-015 — Decision: a failure means a site that builds its snapshot without the producer set admits delegation txs it cannot check for standing.
#[tokio::test]
async fn m12_sec_015_snapshot_without_producers_is_refused() {
    let w = world().await;
    let h = w.node.chain_state.read().await.best_height;
    let mut bad = Vec::new();
    for (row, tx) in admissible(&w).await {
        let blind = snapshot_for(
            &tx,
            w.node.maintainer_state.as_ref(),
            None,
            &w.node.block_store,
        )
        .await;
        let res = w
            .node
            .mempool
            .write()
            .await
            .add_system_transaction(tx.clone(), h, Some(&blind));
        if res.is_ok() {
            bad.push(format!("{row}: admitted with producers = None"));
        }
        w.evict(&tx).await;
        let full = snapshot_for(
            &tx,
            w.node.maintainer_state.as_ref(),
            Some(&w.node.producer_set),
            &w.node.block_store,
        )
        .await;
        let res = w
            .node
            .mempool
            .write()
            .await
            .add_system_transaction(tx.clone(), h, Some(&full));
        if let Err(e) = res {
            bad.push(format!("{row}: refused with the full snapshot: {e}"));
        }
    }
    assert!(bad.is_empty(), "REQ-GRV-SEC-015: {bad:#?}");
}

// REQ-GRV-SEC-015 (GUARD) — Decision: a failure means fail-closed leaked to maintainer changes, which never read the producer set, so a genuine AddMaintainer from a producer-less site is refused.
#[tokio::test]
async fn m12_sec_015_maintainer_change_without_producers_still_admitted() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let tx = m8::genuine_add(&node, &m).await;
    let snap = snapshot_for(&tx, node.maintainer_state.as_ref(), None, &node.block_store).await;
    let h = node.chain_state.read().await.best_height;
    let res = node
        .mempool
        .write()
        .await
        .add_system_transaction(tx, h, Some(&snap));
    assert!(
        res.is_ok(),
        "REQ-GRV-SEC-015 guard: genuine AddMaintainer refused: {res:?}"
    );
}
