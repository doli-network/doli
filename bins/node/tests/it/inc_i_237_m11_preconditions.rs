//! INC-I-237 M11 / REQ-GRV-034 — a maintainer change whose multisig verifies
//! but whose state effect apply would refuse (already a member, set full, not a
//! member, set at MIN_MAINTAINERS) is refused at admission and never relayed;
//! apply itself is unchanged.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict
//!   O1: none   O2: self.mempool membership   O3: Verdict::{Accept, Ignore}
//!   O4: none (apply is read through copies by `apply_effect`)   O5: none   O6: none
//! PATHS: P1 precondition fails -> Ignore, not admitted; P2 (GUARD) apply of the
//!   same rows has no effect and a genuine row still has one, as at HEAD
//! INPUT PARTITIONS: row {Add existing member (4-set), Add into full set (5-set),
//!   Remove non-member (4-set), Remove at MIN_MAINTAINERS (3-set)}; signers meet
//!   the current threshold in every row; params {testnet, tip past every AH}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use crypto::KeyPair;
use doli_core::maintainer::{MAX_MAINTAINERS, MIN_MAINTAINERS};
use doli_core::Transaction;
use doli_node::node::Node;
use network::Verdict;
use tempfile::TempDir;

use super::inc_i_237_m8_common as m8;

#[derive(Clone, Copy, Debug)]
enum Row {
    AddExistingMember,
    AddIntoFullSet,
    RemoveNonMember,
    RemoveAtMinimum,
}

const ROWS: [Row; 4] = [
    Row::AddExistingMember,
    Row::AddIntoFullSet,
    Row::RemoveNonMember,
    Row::RemoveAtMinimum,
];

/// A node with the row's current set and a change signed by enough current members.
async fn row(r: Row) -> (Node, Transaction, TempDir) {
    let size = match r {
        Row::AddIntoFullSet => MAX_MAINTAINERS,
        Row::RemoveAtMinimum => MIN_MAINTAINERS,
        _ => m8::MAINTAINERS,
    };
    let m: Vec<KeyPair> = (0..size).map(|_| KeyPair::generate()).collect();
    let (node, _kp, t) = m8::auth_node_with(&m).await;
    let outsider = *KeyPair::generate().public_key();
    let tx = match r {
        Row::AddExistingMember => {
            m8::signed_change(&node, true, *m[3].public_key(), &[&m[0], &m[1], &m[2]]).await
        }
        Row::AddIntoFullSet => {
            m8::signed_change(&node, true, outsider, &[&m[0], &m[1], &m[2]]).await
        }
        Row::RemoveNonMember => {
            m8::signed_change(&node, false, outsider, &[&m[0], &m[1], &m[2]]).await
        }
        Row::RemoveAtMinimum => {
            m8::signed_change(&node, false, *m[2].public_key(), &[&m[0], &m[1]]).await
        }
    };
    (node, tx, t)
}

async fn assert_refused(r: Row) {
    let (node, tx, _t) = row(r).await;
    let v = node.handle_gossip_txs(vec![tx.clone()]).await;
    let admitted = m8::has(&node, &tx).await;
    assert!(
        v == Verdict::Ignore && !admitted,
        "REQ-GRV-034 {r:?}: verdict={v:?} admitted={admitted}"
    );
}

// REQ-GRV-034 — Decision: a failure means admission takes and relays an AddMaintainer for a key that is already a member, a change apply turns into a no-op.
#[tokio::test]
async fn m11_add_maintainer_for_an_existing_member_is_refused() {
    assert_refused(Row::AddExistingMember).await;
}

// REQ-GRV-034 — Decision: a failure means admission takes and relays an AddMaintainer into a set already at MAX_MAINTAINERS, a change apply refuses.
#[tokio::test]
async fn m11_add_maintainer_into_a_full_set_is_refused() {
    assert_refused(Row::AddIntoFullSet).await;
}

// REQ-GRV-034 — Decision: a failure means admission takes and relays a RemoveMaintainer for a key that is not a member, a change apply refuses.
#[tokio::test]
async fn m11_remove_maintainer_for_a_non_member_is_refused() {
    assert_refused(Row::RemoveNonMember).await;
}

// REQ-GRV-034 — Decision: a failure means admission takes and relays a RemoveMaintainer that would drop the set below MIN_MAINTAINERS, a change apply refuses.
#[tokio::test]
async fn m11_remove_maintainer_at_the_minimum_set_size_is_refused() {
    assert_refused(Row::RemoveAtMinimum).await;
}

// REQ-GRV-034 (GUARD, apply parity) — Decision: a failure means the admission precondition leaked into apply (a consensus change without an activation height), or a row no longer isolates the precondition.
#[tokio::test]
async fn m11_precondition_rows_keep_their_head_apply_effect() {
    let mut bad = Vec::new();
    for r in ROWS {
        let (node, tx, _t) = row(r).await;
        let h = m8::next_height(&node).await;
        if m8::apply_effect(&node, &tx, h).await {
            bad.push(format!("{r:?}: apply now has an effect"));
        }
    }
    let (node, _kp, m, _t) = m8::auth_node().await;
    let genuine = m8::genuine_add(&node, &m).await;
    let h = m8::next_height(&node).await;
    if !m8::apply_effect(&node, &genuine, h).await {
        bad.push("genuine AddMaintainer: apply lost its effect".into());
    }
    assert!(bad.is_empty(), "apply parity: {bad:#?}");
}
