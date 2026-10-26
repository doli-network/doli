//! INC-I-257 M2 — one verdict on mempool, builder and block validation for a
//! slash against an accused that exited by a full withdrawal: admitted and
//! burning while the payout is locked; refused once it unlocked (spent or not);
//! and no spend of the burnable payout inside the slash block.
//!
// covers: slash_admission pool assembly mod slash_state slash_burn tx_processing slash_withdrawal holdings
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) via the node's mempool, judged at tip + 1
//   O1 return (Ok | InvalidSlash message)
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O2 whether the resident slash is selected
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O3 return; O4 supply drop net of minting; O5 tip moved; O6 accused's Bond + Unbonding value; O7 payout liveness
// PATHS: P-LOCKED Exited accused, payout locked at the slash height | P-SPENT payout unlocked and spent
//        | P-UNSPENT payout unlocked, unspent | P-SPEND-IN-SLASH-BLOCK the slash block also spends the payout
// INPUT PARTITIONS:
//   slash height vs payout lock L: < L (after the exit boundary) | == L (unspent) | == L + 1 (spent at L)
//   block order: [Slash, Spend] | [Spend, Slash]
// MATRIX:
//   O1-O7 x P-LOCKED -> inc_i_257_m2_req_unbond_011_exited_accused_with_locked_payout_same_verdict_on_three_paths
//   O1-O5,O7 x P-SPENT -> inc_i_257_m2_req_unbond_012_slash_after_unlock_and_spend_refused_on_three_paths
//   O1-O5,O7 x P-UNSPENT -> inc_i_257_m2_req_unbond_012_slash_after_unlock_unspent_refused_on_three_paths
//   O3,O5,O7 x P-SPEND-IN-SLASH-BLOCK x order -> inc_i_257_m2_inv_slash_001_slash_block_cannot_spend_burnable_payout

use storage::ProducerStatus;

use super::inc_i_255_m2_common::{rig, status, Keys, EPOCH};
use super::inc_i_255_m6_common::{order_name, ordered, transfer};
use super::inc_i_257_m1_common::FEE;
use super::inc_i_257_m2_common::*;

// REQ-UNBOND-011 / REQ-UNBOND-002 — Decision: a failure means the slash that must burn an exited accused's locked payout is refused (or dropped) on one of mempool, builder or block validation, so the funds escape or the paths disagree (INC-I-173 class).
#[tokio::test]
async fn inc_i_257_m2_req_unbond_011_exited_accused_with_locked_payout_same_verdict_on_three_paths()
{
    let mut r = rig(&Keys::new()).await;
    let pk = r.accused_pk();
    let wd = exit_by_full_withdrawal(&mut r).await;
    assert_ne!(
        status(&r.node, &pk).await,
        Some(ProducerStatus::Active),
        "FIXTURE: the boundary applied the full withdrawal"
    );
    assert!(
        r.best().await.0 + 1 < wd.lock,
        "FIXTURE: payout still locked"
    );
    let slash = r.slash();
    let p = three_paths(&mut r, &slash).await;
    assert!(
        p.mempool.starts_with("Ok"),
        "mempool must admit (locked collateral); {p:?}"
    );
    assert!(
        p.selected,
        "the builder must select the resident slash; {p:?}"
    );
    assert_eq!(
        p.block.applied, "Ok",
        "a block holding it must apply; {p:?}"
    );
    assert_eq!(
        p.block.supply_drop,
        i128::from(wd.net),
        "the block burns the locked payout; {p:?}"
    );
    assert_eq!(
        collateral(&r.node, &pk).await,
        0,
        "the accused keeps nothing"
    );
    assert!(entry_of(&r.node, &wd.payout).await.is_none());
}

fn assert_refused(p: &Paths, case: &str) {
    assert!(
        p.mempool.contains(REFUSAL),
        "{case}: mempool must refuse with the target error; {p:?}"
    );
    assert!(!p.selected, "{case}: the builder must not select it; {p:?}");
    assert!(
        p.block.applied.contains(REFUSAL) && !p.block.tip_moved,
        "{case}: a block holding it must be invalid; {p:?}"
    );
    assert_eq!(p.block.supply_drop, 0, "{case}: nothing burned; {p:?}");
}

// REQ-UNBOND-012 — Decision: a failure means a slash against an exited accused with nothing left to burn is admitted, selected or mined (a mineable no-op), so the collateral flag over-admits.
#[tokio::test]
async fn inc_i_257_m2_req_unbond_012_slash_after_unlock_and_spend_refused_on_three_paths() {
    let mut r = rig(&Keys::new()).await;
    let wd = exit_by_full_withdrawal(&mut r).await;
    r.advance_to(wd.lock - 1).await;
    let spend = transfer(&r.keys.accused, wd.payout, wd.net, FEE);
    mine(&mut r, vec![spend], "the payout spend at lock_until").await;
    let slash = r.slash();
    let p = three_paths(&mut r, &slash).await;
    assert_refused(&p, "unlocked and spent");
}

// REQ-UNBOND-012 / REQ-UNBOND-010 — Decision: a failure means the slash still counts an Unbonding output as collateral at or after its lock_until, racing the owner's legal spend.
#[tokio::test]
async fn inc_i_257_m2_req_unbond_012_slash_after_unlock_unspent_refused_on_three_paths() {
    let mut r = rig(&Keys::new()).await;
    let wd = exit_by_full_withdrawal(&mut r).await;
    r.advance_to(wd.lock - 1).await;
    let slash = r.slash();
    let p = three_paths(&mut r, &slash).await;
    assert_refused(&p, "unlocked, unspent");
    assert!(
        entry_of(&r.node, &wd.payout).await.is_some(),
        "the unlocked payout stays with its owner"
    );
}

// INV-SLASH-001 / REQ-UNBOND-010 — Decision: a failure means one block both slashes the accused and spends the payout the slash burns (spend-and-burn race), in either order.
#[tokio::test]
async fn inc_i_257_m2_inv_slash_001_slash_block_cannot_spend_burnable_payout() {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    let wd = mine_withdrawal(&mut r, &[0, 1, 2], "the accused's full withdrawal").await;
    r.step(Vec::new()).await;
    let spend = transfer(&r.keys.accused, wd.payout, wd.net, FEE);
    let slash = r.slash();
    for slash_first in [true, false] {
        let case = order_name(slash_first, "Spend(payout)");
        let e = block_effect(&mut r, ordered(&slash, &spend, slash_first)).await;
        assert!(
            e.applied.starts_with("Err") && !e.tip_moved,
            "{case}: the block must be invalid; {e:?}"
        );
        assert!(
            entry_of(&r.node, &wd.payout).await.is_some(),
            "{case}: the refused block left the payout in place"
        );
    }
}
