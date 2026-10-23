//! INC-I-256 M1 — consequences of a failed apply: an unbonding producer due at
//! the failed height still completes in the next valid block, a valid
//! withdrawal at the same height is accepted, and a restart plus a rollback of
//! the next block return the control's producer set. Production RocksDB UTXO backend.
//!
// covers: apply_block producer_undo helpers mod block_handling rollback periodic fork_cache set_core set_lifecycle set_persistence chain snapshot_install
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (F, then the next valid block)
//   O1 return of the next valid block (Ok on both nodes)
//   O2 producer set observables (canonical bytes, queue, wpc(p), slash flag, AddBond in flight)
//   O3 state_root_component(true) and (false); O4 status and exit height of the unbonding producer q
// OUTPUT CONTRACT: fn Node::rollback_one_block(&mut self, RollbackAuthority) -> Result<RollbackOutcome> (after a reload from disk)
//   O5 producer set observables after the undo equal the control before F
// PATHS: P-UNBOND F at the height where q's unbonding completes, then an empty block
//   | P-VALID F, then [coinbase, RequestWithdrawal(p, 1)] at the same height (holdings gate armed)
//   | P-RESTART F, then B with a producer tx, reload from disk, rollback of B
// INPUT PARTITIONS: q due exactly at F's height | p's allowance 2 before F (1 requested after)
//   | B mutates another producer (non-empty undo snapshot)
// MATRIX: O1-O4 x P-UNBOND, O1-O3 x P-VALID, O5 x P-RESTART = 3 tests

use crypto::PublicKey;
use doli_core::consensus::UNBONDING_PERIOD;
use storage::ProducerStatus;

use super::inc_i_255_m2_common::{status, Rig, ACCUSED_TAG, HONEST_TAG};
use super::inc_i_255_m4_common::err_text;
use super::inc_i_256_m1_common::*;

const MID_EPOCH_TIP: u64 = 5;
const DUE_HEIGHT: u64 = 60_486;
const GATED_TIP: u64 = 21;

/// F on the subject must fail at its ghost transfer and leave the tip in place.
async fn fail_f(s: &mut Rig) {
    let before = tip(s).await;
    let f = failing_block(s).await;
    let ghost = f.transactions[2].hash().to_string();
    let r = s.apply(f).await;
    let text = err_text(&r);
    assert!(
        r.is_err() && text.contains(&ghost),
        "PRECONDITION: F must fail inside the tx loop at the ghost transfer {ghost}; got {text}"
    );
    assert_eq!(tip(s).await, before, "PRECONDITION: F did not move the tip");
}

async fn start_unbonding(r: &Rig, q: &PublicKey, started_at: u64) {
    let mut ps = r.node.producer_set.write().await;
    ps.request_exit(q, started_at)
        .expect("FIXTURE: q enters Unbonding");
    r.node
        .state_db
        .write_producer_set(&ps)
        .expect("FIXTURE: persist ps");
}

async fn exit_height(r: &Rig, q: &PublicKey) -> Option<u64> {
    let ps = r.node.producer_set.read().await;
    let key = crypto::hash::hash(q.as_bytes());
    ps.as_parts().1.get(&key).copied()
}

async fn reload_from_disk(r: &Rig) {
    let loaded = r.node.state_db.load_producer_set();
    *r.node.producer_set.write().await = loaded;
}

// R-1 (INC-I-256) — Decision: a failure means a failed apply at a height where another producer's unbonding completes leaves residue or an empty unbonding index, so that producer never exits on this node.
#[tokio::test]
async fn inc_i_256_m1_unbonding_due_height_matches_control() {
    let (mut s, mut c) = twins(MID_EPOCH_TIP).await;
    let p = p_of(&s);
    let q = *s.keys.delegator.public_key();
    let started_at = DUE_HEIGHT - UNBONDING_PERIOD;
    for r in [&s, &c] {
        start_unbonding(r, &q, started_at).await;
        park_tip(r, DUE_HEIGHT - 1).await;
    }
    assert_eq!(
        obs(&s.node, &p).await,
        obs(&c.node, &p).await,
        "PRECONDITION: twins agree with q unbonding"
    );
    fail_f(&mut s).await;
    let after_f = obs(&s.node, &p).await;
    report(&after_f, &obs(&c.node, &p).await);
    let next = c.next_block(Vec::new(), 0).await;
    c.apply(next.clone())
        .await
        .expect("control applies the empty block at the due height");
    assert_eq!(
        (status(&c.node, &q).await, exit_height(&c, &q).await),
        (Some(ProducerStatus::Exited), Some(DUE_HEIGHT)),
        "PRECONDITION: q completes its unbonding at {DUE_HEIGHT} on the control"
    );
    let r = s.apply(next).await;
    assert!(
        r.is_ok(),
        "the empty block at {DUE_HEIGHT} after F: {}",
        err_text(&r)
    );
    assert_eq!(
        (status(&s.node, &q).await, exit_height(&s, &q).await),
        (Some(ProducerStatus::Exited), Some(DUE_HEIGHT)),
        "q must complete its unbonding at {DUE_HEIGHT} after F failed at the same height"
    );
    assert_obs(
        &obs(&s.node, &p).await,
        &obs(&c.node, &p).await,
        "F failed at the unbonding-due height, then the empty block",
    );
}

// R-1 (INC-I-256) — Decision: a failure means a failed apply makes this node reject a valid canonical withdrawal at the same height that every other node accepts, a live validity split.
#[tokio::test]
async fn inc_i_256_m1_valid_withdrawal_after_failed_apply_accepted() {
    let (mut s, mut c) = twins(GATED_TIP).await;
    let p = p_of(&s);
    let n = GATED_TIP + 1;
    assert!(
        n >= c
            .node
            .config
            .network
            .params()
            .withdrawal_holdings_gate_activation_height,
        "PRECONDITION: the holdings gate is armed at {n}"
    );
    fail_f(&mut s).await;
    let wd = withdrawal(&c, p_key(&c), HONEST_TAG, 1, n);
    let next = c.next_block(vec![wd], 0).await;
    let rc = c.apply(next.clone()).await;
    assert!(
        rc.is_ok(),
        "PRECONDITION: the control accepts RequestWithdrawal(p, 1) at {n}: {}",
        err_text(&rc)
    );
    let rs = s.apply(next).await;
    assert!(
        rs.is_ok(),
        "a valid RequestWithdrawal(p, 1) at {n} after F failed must be accepted: {}",
        err_text(&rs)
    );
    assert_eq!(tip(&s).await, tip(&c).await, "both nodes hold the same tip");
    assert_obs(
        &obs(&s.node, &p).await,
        &obs(&c.node, &p).await,
        "F failed, then the valid withdrawal",
    );
}

// R-1 (INC-I-256) — Decision: a failure means the next block's undo snapshot captured the residue, so a restart plus a rollback brings it back and the node diverges after the undo.
#[tokio::test]
async fn inc_i_256_m1_restart_then_rollback_matches_pre_failure_control() {
    let (mut s, mut c) = twins(MID_EPOCH_TIP).await;
    let p = p_of(&s);
    let want = obs(&c.node, &p).await;
    fail_f(&mut s).await;
    let n = MID_EPOCH_TIP + 1;
    let wd = withdrawal(&c, &c.keys.accused, ACCUSED_TAG, 1, n);
    let b = c.next_block(vec![wd], 0).await;
    for (r, who) in [(&mut s, "subject"), (&mut c, "control")] {
        let res = r.apply(b.clone()).await;
        assert!(
            res.is_ok(),
            "PRECONDITION: {who} applies B at {n}: {}",
            err_text(&res)
        );
        let undo = r.node.state_db.get_undo(n).expect("undo of B");
        assert!(
            !undo.producer_snapshot.is_empty(),
            "PRECONDITION: B's undo carries a producer snapshot on the {who}"
        );
        reload_from_disk(r).await;
        r.rollback().await;
    }
    assert_eq!(
        obs(&c.node, &p).await,
        want,
        "PRECONDITION: the control's own undo of B is exact"
    );
    assert_obs(
        &obs(&s.node, &p).await,
        &want,
        "F failed, B applied, reload from disk, B rolled back",
    );
}
