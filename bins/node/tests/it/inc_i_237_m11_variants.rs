//! INC-I-237 M11 / AUDIT-P1-509 — keyless rewrites of one captured genuine
//! maintainer change (reason, trailing byte, entry order, duplicated entry) are
//! new txids the multisig does not cover; admission must not take or relay them.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict
//!   O1: none (txs moved in)
//!   O2: self.mempool membership / len
//!   O3: Verdict::{Accept, Ignore}
//!   O4: maintainer state (written only by the fixture apply step)
//!   O5: none   O6: none (network = None; the probe file covers the wire)
//! PATHS: P1 genuine T (reason None or Some) -> Accept, admitted;
//!   P2 variant while T is resident -> Ignore, mempool len stays 1, T resident;
//!   P3 variant or byte-identical copy after T applied and was mined -> Ignore, not admitted
//! INPUT PARTITIONS: type {AddMaintainer, RemoveMaintainer}; rewrite {reason,
//!   trailing byte, reordered entries, duplicated entry, none (byte copy)};
//!   T state {resident, applied + mined}; params {testnet, tip past every AH}.
//!   Non-vacuity: every variant still has T's apply effect (it is not forged).
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use doli_core::Transaction;
use doli_node::node::Node;
use network::Verdict;

use super::inc_i_237_m11_common as m11;
use super::inc_i_237_m8_common as m8;

async fn genuine(node: &Node, m: &[crypto::KeyPair], is_add: bool) -> Transaction {
    if is_add {
        m8::genuine_add(node, m).await
    } else {
        m8::genuine_remove(node, m).await
    }
}

/// Variants of a resident genuine T that were admitted, relayed, grew the pool, or displaced T.
async fn resident_variant_leaks(is_add: bool) -> Vec<String> {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let t = genuine(&node, &m, is_add).await;
    assert_eq!(
        node.handle_gossip_txs(vec![t.clone()]).await,
        Verdict::Accept,
        "non-vacuity: genuine T relays"
    );
    assert_eq!(m8::pool_len(&node).await, 1, "fixture: T alone in the pool");
    let h = m8::next_height(&node).await;
    let mut out = Vec::new();
    for (row, v) in m11::variants(&t) {
        assert_ne!(v.hash(), t.hash(), "fixture: {row} is a new txid");
        assert!(
            m8::apply_effect(&node, &v, h).await,
            "fixture: {row} keeps T's apply effect (not a forgery)"
        );
        let verdict = node.handle_gossip_txs(vec![v.clone()]).await;
        let len = m8::pool_len(&node).await;
        let (t_in, v_in) = (m8::has(&node, &t).await, m8::has(&node, &v).await);
        if verdict != Verdict::Ignore || len != 1 || !t_in || v_in {
            out.push(format!(
                "{row}: verdict={verdict:?} pool_len={len} T_resident={t_in} variant_resident={v_in}"
            ));
        }
    }
    out
}

// REQ-GRV-033 / REQ-GRV-035 — Decision: a failure means one captured AddMaintainer still mints new authenticated zero-fee txids that every hop admits and forwards (AUDIT-P1-509).
#[tokio::test]
async fn m11_gossip_variants_of_a_resident_add_maintainer_are_ignored() {
    let bad = resident_variant_leaks(true).await;
    assert!(bad.is_empty(), "AUDIT-P1-509 AddMaintainer: {bad:#?}");
}

// REQ-GRV-033 / REQ-GRV-035 — Decision: a failure means one captured RemoveMaintainer still mints new authenticated zero-fee txids that every hop admits and forwards (AUDIT-P1-509).
#[tokio::test]
async fn m11_gossip_variants_of_a_resident_remove_maintainer_are_ignored() {
    let bad = resident_variant_leaks(false).await;
    assert!(bad.is_empty(), "AUDIT-P1-509 RemoveMaintainer: {bad:#?}");
}

/// After T applied and left the mempool: a rewrite and T's own bytes, each admitted or relayed.
async fn applied_carrier_leaks(is_add: bool) -> Vec<String> {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let t = genuine(&node, &m, is_add).await;
    assert!(
        node.handle_new_transaction(t.clone()).await,
        "fixture: T admitted"
    );
    m11::apply_and_mine(&node, &t).await;
    let target = m11::data(&t).target;
    assert_eq!(
        m11::is_member(&node, &target).await,
        is_add,
        "fixture: T took effect"
    );
    assert!(
        !m8::has(&node, &t).await,
        "fixture: T mined out of the pool"
    );
    let h = m8::next_height(&node).await;
    let rows = vec![
        ("different reason", m11::with_reason(&t, "after apply")),
        ("byte-identical copy of T", t.clone()),
    ];
    let mut out = Vec::new();
    for (row, tx) in rows {
        assert!(
            !m8::apply_effect(&node, &tx, h).await,
            "fixture: apply has no effect for {row} now"
        );
        let before = m8::pool_len(&node).await;
        let verdict = node.handle_gossip_txs(vec![tx.clone()]).await;
        let admitted = m8::has(&node, &tx).await;
        let grew = m8::pool_len(&node).await != before;
        if verdict != Verdict::Ignore || admitted || grew {
            out.push(format!(
                "{row}: verdict={verdict:?} admitted={admitted} pool_grew={grew}"
            ));
        }
    }
    out
}

// REQ-GRV-034 — Decision: a failure means a mined AddMaintainer stays a live carrier: its rewrites and its own bytes are still admitted and relayed after the target joined.
#[tokio::test]
async fn m11_after_add_maintainer_applies_its_variant_and_copy_are_refused() {
    let bad = applied_carrier_leaks(true).await;
    assert!(
        bad.is_empty(),
        "REQ-GRV-034 applied AddMaintainer: {bad:#?}"
    );
}

// REQ-GRV-034 — Decision: a failure means a mined RemoveMaintainer stays a live carrier: its rewrites and its own bytes are still admitted and relayed after the target left.
#[tokio::test]
async fn m11_after_remove_maintainer_applies_its_variant_and_copy_are_refused() {
    let bad = applied_carrier_leaks(false).await;
    assert!(
        bad.is_empty(),
        "REQ-GRV-034 applied RemoveMaintainer: {bad:#?}"
    );
}

// REQ-GRV-035 (GUARD) — Decision: a failure means the fix gated `reason` and refuses a genuine change that apply accepts, stranding submitMaintainerChange callers.
#[tokio::test]
async fn m11_genuine_change_with_a_reason_still_admits_and_relays() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let rows = vec![
        (
            "AddMaintainer + reason",
            m11::with_reason(&m8::genuine_add(&node, &m).await, "onboard ops key"),
        ),
        (
            "RemoveMaintainer + reason",
            m11::with_reason(&m8::genuine_remove(&node, &m).await, "key rotation"),
        ),
    ];
    let mut bad = Vec::new();
    for (row, tx) in rows {
        let v = node.handle_gossip_txs(vec![tx.clone()]).await;
        let admitted = m8::has(&node, &tx).await;
        if v != Verdict::Accept || !admitted {
            bad.push(format!("{row}: verdict={v:?} admitted={admitted}"));
        }
    }
    assert!(bad.is_empty(), "genuine change with a reason: {bad:#?}");
}
