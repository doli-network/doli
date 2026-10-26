//! INC-I-182 M1 — rollback and reorg across a block holding a producer-signed
//! RequestWithdrawal restore the exact earlier state (INV-SLASH-002 shape).
//! Production RocksDB UTXO backend.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn Node::rollback_one_block(&mut self, RollbackAuthority) -> Result<RollbackOutcome>
//   O1 return (RolledBack); O2 UTXO set (payout, Bond, digest, supply); O3 producer set bytes, queue, exits
//   O4 chain tip; O5 state root, also after both twins apply one more block
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (sibling after the undo)
//   O6 state root equals the twin that only applied the sibling
// PATHS: child process, producer-auth gate env 0 | P-REQ undo the request block | P-REORG undo, then a sibling
// MATRIX
//   O1-O5 x P-REQ x {epoch 0 (height 2), mid-epoch (height 6)} -> inc_i_182_m1_req_pauth_002_rollback_restores_state_{epoch0,mid_epoch}
//   O2, O6 x P-REORG -> inc_i_182_m1_req_pauth_002_reorg_across_signed_withdrawal
// INPUT PARTITIONS:
//   withdrawal: one honest Bond, Unbonding payout, producer auth tail signed with the node's genesis

use doli_core::transaction::{Transaction, TxType};
use storage::Outpoint;

use super::inc_i_182_m1_node_fx::*;
use super::inc_i_255_m2_common::{
    assert_converged, assert_same, assert_same_root, live, snap, Keys, Rig,
};
use super::inc_i_255_m4_common::err_text;

const MOD: &str = "inc_i_182_m1_rollback::";

async fn twins(height: u64) -> (Px, Px) {
    let keys = Keys::new();
    let (mut a, mut b) = (px(&keys).await, px(&keys).await);
    a.r.advance_to(height).await;
    b.r.advance_to(height).await;
    assert_same_root(
        &snap(&a.r.node).await,
        &snap(&b.r.node).await,
        "PRECONDITION: twins agree",
    );
    (a, b)
}

fn signed_withdrawal(p: &Px) -> Transaction {
    producer_tx(p, TxType::RequestWithdrawal, Form::Signed, &p.genesis())
}

async fn mine(r: &mut Rig, tx: &Transaction) {
    let block = r.next_block(vec![tx.clone()], 0).await;
    let res = r.apply(block).await;
    assert!(
        res.is_ok(),
        "PRECONDITION: the signed withdrawal is mined; got {}",
        err_text(&res)
    );
}

async fn is_live(r: &Rig, op: Outpoint) -> bool {
    !live(&r.node, &[op]).await.is_empty()
}

async fn rollback_case(before: u64) {
    let (mut a, mut b) = twins(before).await;
    let tx = signed_withdrawal(&a);
    let input = &tx.inputs[0];
    let (payout, bond) = (
        Outpoint::new(tx.hash(), 0),
        Outpoint::new(input.prev_tx_hash, input.output_index),
    );
    mine(&mut a.r, &tx).await;
    assert!(
        is_live(&a.r, payout).await && !is_live(&a.r, bond).await,
        "PRECONDITION: the request block created the payout and spent the Bond"
    );
    a.r.rollback().await;
    let what = format!("signed withdrawal block at {} undone", before + 1);
    assert!(
        !is_live(&a.r, payout).await,
        "{what}: the payout survived the undo"
    );
    assert!(
        is_live(&a.r, bond).await,
        "{what}: the Bond UTXO was not restored"
    );
    assert_same(&snap(&a.r.node).await, &snap(&b.r.node).await, &what);
    assert_converged(&mut a.r, &mut b.r, &what).await;
}

fn child(test: &str) {
    run_child(&format!("{MOD}{test}"), &[(PAUTH_ENV, "0".to_string())]);
}

// REQ-PAUTH-002 — Decision: a failure means undoing an authorized withdrawal block in epoch 0 (producer set changed in the block) leaves state behind, so a reorged node forks.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_002_rollback_restores_state_epoch0() {
    if !in_child() {
        return child("inc_i_182_m1_req_pauth_002_rollback_restores_state_epoch0");
    }
    rollback_case(1).await;
}

// REQ-PAUTH-002 — Decision: a failure means undoing a mid-epoch authorized withdrawal leaves its queued update or payout behind.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_002_rollback_restores_state_mid_epoch() {
    if !in_child() {
        return child("inc_i_182_m1_req_pauth_002_rollback_restores_state_mid_epoch");
    }
    rollback_case(5).await;
}

// REQ-PAUTH-002 — Decision: a failure means a reorg across an authorized withdrawal leaves state that differs from a node that never saw the orphan, a permanent split.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_002_reorg_across_signed_withdrawal() {
    if !in_child() {
        return child("inc_i_182_m1_req_pauth_002_reorg_across_signed_withdrawal");
    }
    let (mut a, mut b) = twins(5).await;
    let sibling = b.r.next_block(Vec::new(), 1).await;
    let tx = signed_withdrawal(&a);
    mine(&mut a.r, &tx).await;
    a.r.rollback().await;
    a.r.apply(sibling.clone())
        .await
        .expect("sibling applies after the undo");
    b.r.apply(sibling)
        .await
        .expect("sibling applies on the twin");
    let what = "reorg onto a sibling of the signed withdrawal block";
    assert!(
        !is_live(&a.r, Outpoint::new(tx.hash(), 0)).await,
        "{what}: orphaned payout survived"
    );
    assert_same_root(&snap(&a.r.node).await, &snap(&b.r.node).await, what);
}
