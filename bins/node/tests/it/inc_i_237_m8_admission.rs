//! INC-I-237 M8 — zero-flow admission authenticates the 4 signed types
//! (DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer) exactly as
//! apply does, and a newly admitted genuine one authorizes a gossip forward.
//!
//! OUTPUT CONTRACT: Node::handle_new_transaction(tx) -> bool and
//!   Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict
//!   O1: none (txs moved in)
//!   O2: self.mempool membership / len (admission)
//!   O3: bool admitted; Verdict::{Accept, Ignore}
//!   O4: none (apply-side state is read through copies by `apply_effect`)
//!   O5: none   O6: none here (network = None; relay file covers the wire)
//! PATHS: P1 genuine zero-flow -> admitted + Accept; P2 forged delegation
//!   (auth AH active) -> refused + Ignore; P3 maintainer change below the current
//!   set's threshold -> refused + Ignore; P4 mixed [forged, genuine] -> Ignore,
//!   genuine admitted alone; P5 zero-flow Registration -> Ignore; P6 devnet
//!   (auth AH u64::MAX) unsigned delegation -> admitted + Accept; P7
//!   (REQ-GRV-038) genuine delegation by throwaway keys -> refused + Ignore
//! INPUT PARTITIONS: type {DelegateBond, RevokeDelegation, AddMaintainer,
//!   RemoveMaintainer, Registration}; delegation signature {genuine, absent
//!   (legacy bytes), malformed, wrong key}; maintainer signatures {3 of 4, 4 of 4,
//!   2 of 4, non-members, other bytes, garbage, one member x3, target as signer};
//!   delegation parties {registered and active on the node, throwaway};
//!   params {auth AH 0 (testnet), devnet u64::MAX}; message {single, mixed order}
//! MATRIX: O2 x O3 x every partition below; parity rows pin O3 (admitted) ==
//!   apply effect for every (type, signature, params) cell.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use doli_core::Transaction;
use doli_node::node::Node;
use network::Verdict;

use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common::{self as m8, Parties, Sig};

/// Rows that leaked: admitted, forwarded, or grew the mempool.
async fn leaks(node: &Node, rows: Vec<(String, Transaction)>) -> Vec<String> {
    let mut out = Vec::new();
    for (row, tx) in rows {
        let before = m8::pool_len(node).await;
        let admitted = node.handle_new_transaction(tx.clone()).await;
        let verdict = node.handle_gossip_txs(vec![tx.clone()]).await;
        let grew = m8::pool_len(node).await != before;
        if admitted || grew || verdict == Verdict::Accept {
            out.push(format!(
                "{row}: admitted={admitted} pool_grew={grew} verdict={verdict:?}"
            ));
        }
    }
    out
}

/// One tx per signature in `sigs`, each by fresh parties registered on `node` (delegated for a revoke).
async fn delegation_rows(node: &Node, revoke: bool, sigs: &[Sig]) -> Vec<(String, Transaction)> {
    let mut rows = Vec::new();
    for s in sigs {
        let p = Parties::registered(&[node], revoke).await;
        let (kind, tx) = if revoke {
            ("RevokeDelegation", m8::revoke(&p, *s))
        } else {
            ("DelegateBond", m8::delegate_bond(&p, *s))
        };
        rows.push((format!("{kind}: {s:?} signature"), tx));
    }
    rows
}

// REQ-GRV-SEC-007 — Decision: a failure means gossip admits a DelegateBond whose signature apply refuses, so forged delegations sit in mempools and (post-M8) would relay.
#[tokio::test]
async fn req_grv_sec_007_gossip_refuses_forged_delegate_bond() {
    let (node, _kp, _m, _t) = m8::auth_node().await;
    let bad = leaks(&node, delegation_rows(&node, false, &m8::FORGED_SIGS).await).await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-007 gossip: {bad:#?}");
    let genuine = m8::genuine_delegate_bond(&[&node]).await;
    assert!(
        node.handle_new_transaction(genuine).await,
        "non-vacuity: a genuine DelegateBond is admitted"
    );
}

// REQ-GRV-SEC-007 — Decision: a failure means gossip admits a RevokeDelegation whose signature apply refuses.
#[tokio::test]
async fn req_grv_sec_007_gossip_refuses_forged_revoke_delegation() {
    let (node, _kp, _m, _t) = m8::auth_node().await;
    let bad = leaks(&node, delegation_rows(&node, true, &m8::FORGED_SIGS).await).await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-007 gossip: {bad:#?}");
    let genuine = m8::genuine_revoke(&[&node]).await;
    assert!(
        node.handle_new_transaction(genuine).await,
        "non-vacuity: a genuine RevokeDelegation is admitted"
    );
}

// REQ-GRV-SEC-007 — Decision: a failure means gossip admits an AddMaintainer the CURRENT set's multisig refuses.
#[tokio::test]
async fn req_grv_sec_007_gossip_refuses_add_maintainer_below_the_current_threshold() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let rows = m8::forged_changes(&node, &m, true).await;
    let bad = leaks(&node, rows).await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-007 gossip: {bad:#?}");
    let genuine = m8::genuine_add(&node, &m).await;
    assert!(
        node.handle_new_transaction(genuine).await,
        "non-vacuity: a 3-of-4 AddMaintainer is admitted"
    );
}

// REQ-GRV-SEC-007 — Decision: a failure means gossip admits a RemoveMaintainer the CURRENT set's multisig refuses (incl. the target signing its own removal).
#[tokio::test]
async fn req_grv_sec_007_gossip_refuses_remove_maintainer_below_the_current_threshold() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let rows = m8::forged_changes(&node, &m, false).await;
    let bad = leaks(&node, rows).await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-007 gossip: {bad:#?}");
    let genuine = m8::genuine_remove(&node, &m).await;
    assert!(
        node.handle_new_transaction(genuine).await,
        "non-vacuity: a 3-of-4 RemoveMaintainer is admitted"
    );
}

// REQ-GRV-029 — Decision: a failure means a newly admitted genuine zero-flow tx still never authorizes a forward, so it stops at hop 1 (the INC-I-195 gap).
#[tokio::test]
async fn req_grv_029_genuine_zero_flow_of_each_type_authorizes_a_forward() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let rows = vec![
        ("DelegateBond", m8::genuine_delegate_bond(&[&node]).await),
        ("RevokeDelegation", m8::genuine_revoke(&[&node]).await),
        ("AddMaintainer", m8::genuine_add(&node, &m).await),
        ("RemoveMaintainer", m8::genuine_remove(&node, &m).await),
    ];
    let mut bad = Vec::new();
    for (row, tx) in rows {
        let v = node.handle_gossip_txs(vec![tx.clone()]).await;
        let admitted = m8::has(&node, &tx).await;
        if v != Verdict::Accept || !admitted {
            bad.push(format!("{row}: verdict={v:?} admitted={admitted}"));
        }
    }
    assert!(bad.is_empty(), "REQ-GRV-029: {bad:#?}");
}

// D5 / REQ-GRV-SEC-008 — Decision: a failure means one genuine zero-flow tx carries a forged sibling across the hop, or the forged one is admitted next to it.
#[tokio::test]
async fn d5_mixed_forged_and_genuine_zero_flow_message_is_ignored() {
    for forged_first in [true, false] {
        let (node, _kp, _m, _t) = m8::auth_node().await;
        let forged = m8::delegate_bond(&Parties::registered(&[&node], false).await, Sig::Wrong);
        let genuine = m8::genuine_delegate_bond(&[&node]).await;
        let msg = if forged_first {
            vec![forged.clone(), genuine.clone()]
        } else {
            vec![genuine.clone(), forged.clone()]
        };
        let v = node.handle_gossip_txs(msg).await;
        assert_eq!(v, Verdict::Ignore, "D5 forged_first={forged_first}");
        assert!(
            !m8::has(&node, &forged).await,
            "REQ-GRV-SEC-008: forged sibling admitted (forged_first={forged_first})"
        );
        assert!(
            m8::has(&node, &genuine).await,
            "the genuine sibling is still admitted on its own"
        );
    }
}

// REQ-GRV-031 — Decision: a failure means a zero-flow Registration (no signature check at admission) authorizes a forward.
#[tokio::test]
async fn req_grv_031_zero_flow_registration_never_forwards() {
    let registration = || {
        let pk = *crypto::KeyPair::generate().public_key();
        Transaction::new_registration(vec![], pk, 0, 0, 0)
    };
    assert!(registration().is_zero_flow(), "fixture: zero-flow shape");
    let (auth, _kp, _m, _t) = m8::auth_node().await;
    let (dev, _kp2, _m2, _t2) = m8::devnet_node().await;
    for (net, node) in [("auth", &auth), ("devnet", &dev)] {
        let tx = registration();
        let v = node.handle_gossip_txs(vec![tx]).await;
        assert_eq!(v, Verdict::Ignore, "REQ-GRV-031 ({net})");
    }
}

// REQ-GRV-SEC-007 fixture — Decision: a failure means the auth-params node refuses or never forwards ordinary fee-paying txs, so every M8 red above would be vacuous.
#[tokio::test]
async fn fixture_auth_node_admits_and_forwards_a_fee_paying_transfer() {
    let (node, kp, _m, _t) = m8::auth_node().await;
    c::seed(&node, &kp, c::prev(42)).await;
    let x = c::transfer(&kp, c::prev(42));
    assert_eq!(
        node.handle_gossip_txs(vec![x.clone()]).await,
        Verdict::Accept
    );
    assert!(m8::has(&node, &x).await, "admitted");
}

// REQ-GRV-SEC-007 (devnet AH) — Decision: a failure means admission became stricter than apply: devnet applies an unsigned legacy DelegateBond, so refusing it strands it.
#[tokio::test]
async fn devnet_unsigned_legacy_delegate_bond_is_still_admitted() {
    let (node, _kp, _m, _t) = m8::devnet_node().await;
    let tx = m8::delegate_bond(&Parties::registered(&[&node], false).await, Sig::Absent);
    let h = m8::next_height(&node).await;
    assert!(
        m8::apply_effect(&node, &tx, h).await,
        "fixture: devnet apply queues it"
    );
    assert!(
        node.handle_new_transaction(tx.clone()).await,
        "devnet: unsigned legacy DelegateBond must be admitted"
    );
}

// REQ-GRV-029 (devnet AH) — Decision: a failure means a delegation devnet applies is admitted but still stops at hop 1.
#[tokio::test]
async fn devnet_unsigned_legacy_delegate_bond_authorizes_a_forward() {
    let (node, _kp, _m, _t) = m8::devnet_node().await;
    let tx = m8::delegate_bond(&Parties::registered(&[&node], false).await, Sig::Absent);
    let v = node.handle_gossip_txs(vec![tx.clone()]).await;
    assert!(m8::has(&node, &tx).await, "admitted");
    assert_eq!(v, Verdict::Accept, "devnet legacy DelegateBond forwards");
}

/// Every (type, signature) cell: admitted at the next height == apply has a state effect there.
async fn parity_mismatches(node: &Node, m: &[crypto::KeyPair]) -> Vec<String> {
    let all_sigs = [Sig::Genuine, Sig::Absent, Sig::Malformed, Sig::Wrong];
    let mut rows = delegation_rows(node, false, &all_sigs).await;
    rows.extend(delegation_rows(node, true, &all_sigs).await);
    rows.push((
        "AddMaintainer 3 of 4".into(),
        m8::genuine_add(node, m).await,
    ));
    let all: Vec<&crypto::KeyPair> = m.iter().collect();
    let target = *crypto::KeyPair::generate().public_key();
    rows.push((
        "AddMaintainer 4 of 4".into(),
        m8::signed_change(node, true, target, &all).await,
    ));
    rows.push((
        "RemoveMaintainer 3 of 4".into(),
        m8::genuine_remove(node, m).await,
    ));
    let dup_target = *crypto::KeyPair::generate().public_key();
    let dup_msg = m8::maintainer_message(node, true, &dup_target, m8::next_height(node).await);
    rows.push((
        "AddMaintainer one member x3".into(),
        m8::maintainer_tx(true, dup_target, vec![m8::msig(&m[0], &dup_msg); 3]),
    ));
    rows.extend(m8::forged_changes(node, m, true).await);
    rows.extend(m8::forged_changes(node, m, false).await);
    let h = m8::next_height(node).await;
    let mut out = Vec::new();
    for (row, tx) in rows {
        let effect = m8::apply_effect(node, &tx, h).await;
        let admitted = node.handle_new_transaction(tx).await;
        if admitted != effect {
            out.push(format!("{row}: admitted={admitted} apply_effect={effect}"));
        }
    }
    out
}

// REQ-GRV-SEC-007 / REQ-GRV-SEC-008 parity (auth AH active) — Decision: a failure means admission and apply disagree on a cell: either a tx apply refuses is admitted, or admission is stricter than apply.
#[tokio::test]
async fn admission_matches_apply_with_delegation_auth_active() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let bad = parity_mismatches(&node, &m).await;
    assert!(bad.is_empty(), "admission != apply (auth AH 0): {bad:#?}");
}

// REQ-GRV-SEC-007 parity (devnet AH u64::MAX) — Decision: a failure means admission is stricter than devnet apply for delegations, or admits maintainer changes devnet apply refuses.
#[tokio::test]
async fn admission_matches_apply_on_devnet_params() {
    let (node, _kp, m, _t) = m8::devnet_node().await;
    let bad = parity_mismatches(&node, &m).await;
    assert!(bad.is_empty(), "admission != apply (devnet): {bad:#?}");
}

// REQ-GRV-038 / REQ-GRV-SEC-012 / REQ-GRV-SEC-013 — Decision: a failure means a delegation signed by a key no node has registered (a boundary no-op on every network) is still admitted and authorizes a forward.
#[tokio::test]
async fn req_grv_038_throwaway_key_delegations_are_refused() {
    let (auth, _kp, _m, _t) = m8::auth_node().await;
    let (dev, _kp2, _m2, _t2) = m8::devnet_node().await;
    let mut bad = Vec::new();
    for (net, node, sig) in [("auth", &auth, Sig::Genuine), ("devnet", &dev, Sig::Absent)] {
        let p = Parties::throwaway();
        let rows = vec![
            (
                format!("{net} DelegateBond {sig:?}"),
                m8::delegate_bond(&p, sig),
            ),
            (
                format!("{net} RevokeDelegation {sig:?}"),
                m8::revoke(&p, sig),
            ),
        ];
        bad.extend(leaks(node, rows).await);
    }
    assert!(bad.is_empty(), "REQ-GRV-038 throwaway rows: {bad:#?}");
}
