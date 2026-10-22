//! INC-I-257 M1 — one spend-lock verdict on every path: the spend of a gated
//! payout gets the same verdict from mempool admission, the block builder and
//! block apply at lock_until - 1 and at lock_until; Replay reaches the same
//! state root as apply. Production RocksDB UTXO backend.
//!
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64 /*tip*/, u32) -> Result<..>
//   O1 return for the spend at tip = H - 1
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O2 whether the resident spend is selected for height H
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O3 return for a block at H holding the spend (Light); O4 whole state after Replay of the same blocks
// PATHS: P-EARLY H = lock_until - 1 | P-ON H = lock_until | P-REPLAY twin re-applies every block in Replay mode
// INPUT PARTITIONS:
//   height: lock_until - 1 (all three refuse) | lock_until (all three accept)
//   replay checkpoint: after the withdrawal block (Unbonding live) | after the spend block (Unbonding spent)
// MATRIX:
//   O1,O2,O3 x {P-EARLY, P-ON} -> inc_i_257_m1_req_unbond_005_same_verdict_on_three_paths
//   O4 x P-REPLAY x checkpoint -> inc_i_257_m1_req_unbond_005_replay_reaches_apply_state_root

use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::Block;

use super::inc_i_255_m1_common::full_block_with;
use super::inc_i_255_m2_common::{assert_same_root, rig, snap, Keys, Rig};
use super::inc_i_255_m4_common::{admit_at, err_text};
use super::inc_i_257_m1_fx::*;

#[derive(Debug, PartialEq, Eq)]
struct Seen {
    mempool: bool,
    builder: bool,
    apply: bool,
}

async fn mine(r: &mut Rig, txs: Vec<Transaction>, what: &str) -> Block {
    let block = r.next_block(txs, 0).await;
    let h = r.best().await.0 + 1;
    let res = r.apply(block.clone()).await;
    assert!(
        res.is_ok(),
        "PRECONDITION: {what} is mined at {h}; got {}",
        err_text(&res)
    );
    block
}

/// Verdicts for the spend `t` in a block at the next height.
async fn verdicts_next(r: &mut Rig, wd: &Wd, t: &Transaction) -> (Seen, String) {
    let tip = r.best().await.0;
    let pool = admit_at(&r.node, t, tip).await;
    r.node.mempool.write().await.remove_transaction(&t.hash());
    let forced = admit_at(&r.node, t, wd.lock - 1).await;
    assert!(
        forced.is_ok(),
        "PRECONDITION: the spend is made resident for the builder probe: {forced:?}"
    );
    let (_, selected) = full_block_with(&mut r.node, &[]).await;
    r.node.mempool.write().await.remove_transaction(&t.hash());
    let block = r.next_block(vec![t.clone()], 0).await;
    let applied = r.apply(block).await;
    let seen = Seen {
        mempool: pool.is_ok(),
        builder: selected.contains(&t.hash()),
        apply: applied.is_ok(),
    };
    let detail = format!(
        "H={} mempool {pool:?}; apply {}",
        tip + 1,
        err_text(&applied)
    );
    (seen, detail)
}

// REQ-UNBOND-005 / REQ-UNBOND-006 — Decision: a failure means mempool, builder and apply disagree on the lock (the INC-I-173 both-or-neither class): a builder block that apply refuses, or a mempool that holds an unminable spend.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_005_same_verdict_on_three_paths() {
    require_gated_type();
    let mut r = rig(&Keys::new()).await;
    r.advance_to(5).await;
    let wd = withdrawal_for_next(&r, Shape::Gated).await;
    mine(&mut r, vec![wd.tx.clone()], "the gated withdrawal").await;
    let t = spend_of(&r, &wd);
    r.advance_to(wd.lock - 2).await;
    let (early, early_detail) = verdicts_next(&mut r, &wd, &t).await;
    r.advance_to(wd.lock - 1).await;
    let (on_time, on_detail) = verdicts_next(&mut r, &wd, &t).await;
    let refused = Seen {
        mempool: false,
        builder: false,
        apply: false,
    };
    let accepted = Seen {
        mempool: true,
        builder: true,
        apply: true,
    };
    assert_eq!(
        early, refused,
        "at lock_until - 1 every path must refuse the spend; {early_detail}"
    );
    assert_eq!(
        on_time, accepted,
        "at lock_until every path must accept the spend; {on_detail}"
    );
}

// REQ-UNBOND-005 — Decision: a failure means a node that replays trusted blocks (restart, re-sync) builds a different UTXO set or state root than one that applied them live.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_005_replay_reaches_apply_state_root() {
    require_gated_type();
    let (mut a, mut c) = twins(5).await;
    let wd = withdrawal_for_next(&a, Shape::Gated).await;
    let first = mine(&mut a, vec![wd.tx.clone()], "the gated withdrawal").await;
    let after_wd = snap(&a.node).await;
    let mut rest = Vec::new();
    while a.best().await.0 < wd.lock - 1 {
        rest.push(a.step(Vec::new()).await);
    }
    let spend = spend_of(&a, &wd);
    rest.push(mine(&mut a, vec![spend], "the spend at lock_until").await);

    c.node
        .apply_block(first, ValidationMode::Replay)
        .await
        .unwrap_or_else(|e| panic!("replay of the withdrawal block: {e:#}"));
    assert!(
        after_wd.ops.contains(&wd.payout),
        "NON-VACUITY: the payout is live after the withdrawal block"
    );
    assert_same_root(
        &snap(&c.node).await,
        &after_wd,
        "replay vs apply after the withdrawal block",
    );
    for (i, b) in rest.into_iter().enumerate() {
        c.node
            .apply_block(b, ValidationMode::Replay)
            .await
            .unwrap_or_else(|e| panic!("replay of block #{i} after the withdrawal: {e:#}"));
    }
    let end = snap(&a.node).await;
    assert!(
        !end.ops.contains(&wd.payout),
        "NON-VACUITY: the payout is spent at the end"
    );
    assert_same_root(
        &snap(&c.node).await,
        &end,
        "replay vs apply after the spend block",
    );
}
