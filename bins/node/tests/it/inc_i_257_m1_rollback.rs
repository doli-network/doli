//! INC-I-257 M1 — rollback parity for the gated withdrawal: undoing the request
//! block removes the Unbonding output and restores the Bond and the producer
//! set; undoing the spend after the unlock restores the output with its lock;
//! a reorg across the request block equals a node that never saw it.
//! Production RocksDB UTXO backend.
//!
// OUTPUT CONTRACT: fn Node::rollback_one_block(&mut self, RollbackAuthority) -> Result<RollbackOutcome>
//   O1 return (RolledBack, via Rig::rollback)
//   O2 UTXO set: payout outpoint, Bond outpoint, entry bytes (digest), supply
//   O3 producer set bytes, pending queue, exit history
//   O4 chain tip; O5 state root after both nodes apply one more block
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (sibling after the undo)
//   O6 whole state incl. root equals the node that only applied the sibling
// PATHS: P-REQ undo the request block | P-SPEND undo the spend block after the unlock | P-REORG undo, then a sibling
// INPUT PARTITIONS:
//   request height: epoch 0 (producer set changed in the block) | mid-epoch (update queued)
//   spend: at lock_until (first legal height)
// MATRIX:
//   O1-O5 x P-REQ x {epoch 0, mid-epoch} -> inc_i_257_m1_req_unbond_007_request_block_rollback_{epoch0,mid_epoch}
//   O1-O5 x P-SPEND -> inc_i_257_m1_req_unbond_007_spend_block_rollback
//   O2-O6 x P-REORG -> inc_i_257_m1_req_unbond_007_reorg_across_request_block

use doli_core::transaction::Transaction;
use storage::Outpoint;

use super::inc_i_255_m2_common::{
    assert_converged, assert_same, assert_same_root, live, snap, Rig,
};
use super::inc_i_255_m4_common::err_text;
use super::inc_i_257_m1_common::UNBONDING_TYPE_BYTE;
use super::inc_i_257_m1_fx::*;

async fn mine(r: &mut Rig, txs: Vec<Transaction>, what: &str) {
    let block = r.next_block(txs, 0).await;
    let h = r.best().await.0 + 1;
    let res = r.apply(block).await;
    assert!(
        res.is_ok(),
        "PRECONDITION: {what} is mined at {h}; got {}",
        err_text(&res)
    );
}

async fn is_live(r: &Rig, op: Outpoint) -> bool {
    !live(&r.node, &[op]).await.is_empty()
}

async fn request_block_rollback(before: u64) {
    require_gated_type();
    let (mut a, mut b) = twins(before).await;
    let wd = withdrawal_for_next(&a, Shape::Gated).await;
    mine(&mut a, vec![wd.tx.clone()], "the gated withdrawal").await;
    assert!(
        is_live(&a, wd.payout).await && !is_live(&a, wd.bond).await,
        "PRECONDITION: the request block created the payout and spent the Bond"
    );
    a.rollback().await;
    let what = format!("request block at {}", before + 1);
    assert!(
        !is_live(&a, wd.payout).await,
        "{what}: the Unbonding output survived the undo"
    );
    assert!(
        is_live(&a, wd.bond).await,
        "{what}: the Bond UTXO was not restored"
    );
    assert_same(&snap(&a.node).await, &snap(&b.node).await, &what);
    assert_converged(&mut a, &mut b, &what).await;
}

// REQ-UNBOND-007 — Decision: a failure means undoing a withdrawal block in epoch 0 (producer set changed in the block) leaves the Unbonding output, loses the Bond, or leaves the producer set changed, so a reorged node forks.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_007_request_block_rollback_epoch0() {
    request_block_rollback(1).await;
}

// REQ-UNBOND-007 — Decision: a failure means undoing a mid-epoch withdrawal block leaves its queued update or its Unbonding output behind.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_007_request_block_rollback_mid_epoch() {
    request_block_rollback(5).await;
}

// REQ-UNBOND-007 — Decision: a failure means undoing the first legal spend restores the payout without its lock or type, or diverges from a node that never spent it.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_007_spend_block_rollback() {
    require_gated_type();
    let (mut a, mut b) = twins(5).await;
    let wd = withdrawal_for_next(&a, Shape::Gated).await;
    mine(
        &mut a,
        vec![wd.tx.clone()],
        "the gated withdrawal on node A",
    )
    .await;
    mine(
        &mut b,
        vec![wd.tx.clone()],
        "the gated withdrawal on node B",
    )
    .await;
    a.advance_to(wd.lock - 1).await;
    b.advance_to(wd.lock - 1).await;
    assert_same_root(
        &snap(&a.node).await,
        &snap(&b.node).await,
        "PRECONDITION: twins agree before the spend",
    );
    let spend = spend_of(&a, &wd);
    mine(&mut a, vec![spend.clone()], "the spend at lock_until").await;
    assert!(
        !is_live(&a, wd.payout).await,
        "PRECONDITION: the spend consumed the payout"
    );
    a.rollback().await;
    let what = "spend block undone";
    assert!(
        is_live(&a, wd.payout).await,
        "{what}: the Unbonding output was not restored"
    );
    assert!(
        !is_live(&a, Outpoint::new(spend.hash(), 0)).await,
        "{what}: the spend output survived the undo"
    );
    let entry = a
        .node
        .utxo_set
        .read()
        .await
        .get(&wd.payout)
        .expect("payout entry");
    assert_eq!(
        (entry.output.output_type as u8, entry.output.lock_until),
        (UNBONDING_TYPE_BYTE, wd.lock),
        "{what}: restored entry lost its type or lock"
    );
    assert_same(&snap(&a.node).await, &snap(&b.node).await, what);
    assert_converged(&mut a, &mut b, what).await;
}

// REQ-UNBOND-007 — Decision: a failure means a reorg across a withdrawal leaves state that differs from a node that never saw the orphan, a permanent split.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_007_reorg_across_request_block() {
    require_gated_type();
    let (mut a, mut b) = twins(5).await;
    let sibling = b.next_block(Vec::new(), 1).await;
    let wd = withdrawal_for_next(&a, Shape::Gated).await;
    mine(&mut a, vec![wd.tx.clone()], "the gated withdrawal (orphan)").await;
    a.rollback().await;
    a.apply(sibling.clone())
        .await
        .expect("sibling applies after the undo");
    b.apply(sibling).await.expect("sibling applies on the twin");
    let what = "reorg onto a sibling of the request block";
    assert!(
        !is_live(&a, wd.payout).await,
        "{what}: orphaned Unbonding output survived"
    );
    assert!(
        is_live(&a, wd.bond).await,
        "{what}: orphaned withdrawal kept the Bond spent"
    );
    assert_same_root(&snap(&a.node).await, &snap(&b.node).await, what);
}
