//! INC-I-257 M2 — the slash burn reaches the accused's locked Unbonding
//! outputs on the apply path, leaves unlocked ones alone, and its undo record
//! restores every burned entry. Production RocksDB UTXO backend.
//!
// covers: slash_burn slash_state tx_processing rollback batch queries
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (the slash block)
//   O1 return; O2 supply drop net of coinbase minting; O3 Bond + Unbonding value owned by the accused
//   O4 Normal value owned by the accused; O5 liveness and entry bytes of each Unbonding payout
// OUTPUT CONTRACT: fn Node::rollback_one_block(&mut self, RollbackAuthority) -> Result<RollbackOutcome>
//   O6 restored entries (type, lock_until, amount, owner); O7 whole state and state root vs a twin
// PATHS: P-PARTIAL partial withdrawal, slash in the same epoch | P-LOCK two payouts, slash mined at the
//        first payout's lock_until | P-UNDO slash block undone
// INPUT PARTITIONS:
//   payout lock vs slash height H: lock > H (burned) | lock == H (not burned)
//   accused holdings at the slash: remaining Bonds + locked Unbonding | Bond + unlocked and locked Unbonding
//   Normal coin owned by the accused: present (never burned)
// MATRIX:
//   O1-O5 x P-PARTIAL -> inc_i_257_m2_req_unbond_010_partial_withdrawal_then_slash_burns_all_collateral
//   O1,O2,O5 x P-LOCK -> inc_i_257_m2_req_unbond_010_unlocked_payout_survives_slash_at_lock_until
//   O6,O7 x P-UNDO -> inc_i_257_m2_req_unbond_007_slash_block_rollback_restores_burned_unbonding
//   O5,O7 x P-REBUILD (rollback with no undo data, rebuild from blocks) -> inc_i_257_m2_req_unbond_010_rebuild_burns_locked_unbonding

use crypto::Hash;
use doli_core::transaction::{Input, Output, Transaction};
use storage::{Outpoint, ProducerStatus, UtxoEntry};

use super::inc_i_255_m2_common::{
    addr, assert_converged, assert_same, assert_same_root, outpoints, rig, rig_with, snap, status,
    Keys, ACCUSED_TAG, EPOCH,
};
use super::inc_i_257_m1_common::{gated_withdrawal, FEE};
use super::inc_i_257_m1_fx::{period, twins};
use super::inc_i_257_m2_common::*;

// REQ-UNBOND-010 — Decision: a failure means a slash in the epoch of a partial withdrawal burns the remaining Bonds but not the locked payout (the offender keeps the withdrawn part), or burns the accused's Normal coins.
#[tokio::test]
async fn inc_i_257_m2_req_unbond_010_partial_withdrawal_then_slash_burns_all_collateral() {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    let pk = r.accused_pk();
    let unit = r.unit();
    let wd = mine_withdrawal(&mut r, &[0], "the accused's one-bond withdrawal").await;
    r.step(Vec::new()).await;
    let held = collateral(&r.node, &pk).await;
    assert_eq!(held, 2 * unit + wd.net, "FIXTURE: two Bonds + the payout");
    assert_eq!(
        status(&r.node, &pk).await,
        Some(ProducerStatus::Active),
        "FIXTURE: the accused is Active inside the epoch"
    );
    let coins = normal_owned(&r.node, &pk).await;
    let slash = r.slash();
    let e = block_effect(&mut r, vec![slash]).await;
    assert_eq!(e.applied, "Ok", "the slash block must apply; {e:?}");
    assert_eq!(
        e.supply_drop,
        i128::from(held),
        "supply must drop by the remaining Bonds plus the locked payout; {e:?}"
    );
    assert_eq!(
        collateral(&r.node, &pk).await,
        0,
        "the accused keeps no Bond or Unbonding value"
    );
    assert!(
        entry_of(&r.node, &wd.payout).await.is_none(),
        "the locked payout is burned"
    );
    assert_eq!(
        normal_owned(&r.node, &pk).await,
        coins,
        "Normal coins are never burned"
    );
}

// REQ-UNBOND-010 — Decision: a failure means the burn takes an Unbonding output at or after its lock_until (racing a legal spend) or misses one still locked in the same slash.
#[tokio::test]
async fn inc_i_257_m2_req_unbond_010_unlocked_payout_survives_slash_at_lock_until() {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    let pk = r.accused_pk();
    let unit = r.unit();
    let first = mine_withdrawal(&mut r, &[0], "the first one-bond withdrawal").await;
    r.advance_to(2 * EPOCH).await;
    let second = mine_withdrawal(&mut r, &[1], "the second one-bond withdrawal").await;
    assert!(second.lock > first.lock, "FIXTURE: lock order");
    r.advance_to(first.lock - 1).await;
    let kept = entry_of(&r.node, &first.payout)
        .await
        .expect("FIXTURE: first payout live");
    let last_bond = outpoints(ACCUSED_TAG, 3)[2];
    assert!(
        entry_of(&r.node, &last_bond).await.is_some(),
        "FIXTURE: one Bond left"
    );
    let slash = r.slash();
    let e = block_effect(&mut r, vec![slash]).await;
    assert_eq!(e.applied, "Ok", "slash at H = first lock_until; {e:?}");
    assert_eq!(
        e.supply_drop,
        i128::from(unit + second.net),
        "burn = last Bond + the still-locked second payout; the unlocked first payout stays; {e:?}"
    );
    assert_eq!(
        entry_of(&r.node, &first.payout).await,
        Some(kept),
        "the payout unlocked at H is untouched"
    );
    assert!(
        entry_of(&r.node, &second.payout).await.is_none(),
        "the payout locked past H is burned"
    );
    assert!(
        entry_of(&r.node, &last_bond).await.is_none(),
        "the last Bond is burned"
    );
    assert_eq!(collateral(&r.node, &pk).await, first.net);
}

// REQ-UNBOND-007 / REQ-UNBOND-010 — Decision: a failure means the slash block's undo record lacks a burned Unbonding entry (or loses its lock), so a reorg across the slash forks the UTXO set and the state root.
#[tokio::test]
async fn inc_i_257_m2_req_unbond_007_slash_block_rollback_restores_burned_unbonding() {
    let (mut a, mut b) = twins(EPOCH).await;
    let wd = accused_withdrawal(&a, &[0]).await;
    mine(&mut a, vec![wd.tx.clone()], "withdrawal on A").await;
    mine(&mut b, vec![wd.tx.clone()], "withdrawal on B").await;
    a.step(Vec::new()).await;
    b.step(Vec::new()).await;
    assert_same_root(
        &snap(&a.node).await,
        &snap(&b.node).await,
        "PRECONDITION: twins agree before the slash",
    );
    let payout = entry_of(&a.node, &wd.payout)
        .await
        .expect("FIXTURE: payout live");
    let slash = a.slash();
    mine(&mut a, vec![slash], "the slash on A").await;
    assert!(
        entry_of(&a.node, &wd.payout).await.is_none(),
        "NON-VACUITY: the slash burned the locked payout"
    );
    a.rollback().await;
    let what = "slash block undone";
    assert_eq!(
        entry_of(&a.node, &wd.payout).await,
        Some(payout),
        "{what}: the burned Unbonding entry must come back with the same type, lock_until, amount and owner"
    );
    assert_same(&snap(&a.node).await, &snap(&b.node).await, what);
    assert_converged(&mut a, &mut b, what).await;
}

// REQ-UNBOND-010 / REQ-UNBOND-007 — Decision: a failure means a node that rebuilds from blocks (no undo data) keeps a locked payout the slash burned, a state-root split with nodes that applied the slash.
#[tokio::test]
async fn inc_i_257_m2_req_unbond_010_rebuild_burns_locked_unbonding() {
    let mut r = rig_with(&Keys::new(), 0).await;
    let (unit, pk) = (r.unit(), r.accused_pk());
    let fund = Outpoint::new(Hash::from_bytes([0xF8; 32]), 0);
    {
        let mut utxo = r.node.utxo_set.write().await;
        utxo.clear().expect("FIXTURE: empty UTXO set");
        let entry = UtxoEntry {
            output: Output::normal(2 * unit + unit / 100, addr(&pk)),
            height: 0,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(fund, entry).expect("FIXTURE: fund");
    }
    let mut inp = Input::new(fund.tx_hash, fund.index);
    inp.public_key = Some(pk);
    let mut ab = Transaction::new_add_bond(vec![inp], pk, 2, 2 * unit, u64::MAX);
    let msg = ab.signing_message_for_input(0);
    ab.inputs[0].signature = crypto::signature::sign_hash(&msg, r.keys.accused.private_key());
    mine(&mut r, vec![ab.clone()], "the AddBond").await;
    let lock = r.best().await.0 + 1 + period(&r);
    let bond = Outpoint::new(ab.hash(), 0);
    let wd = gated_withdrawal(&r.keys.accused, &[bond], unit - FEE, lock);
    mine(&mut r, vec![wd.clone()], "the gated withdrawal").await;
    let payout = Outpoint::new(wd.hash(), 0);
    assert!(
        entry_of(&r.node, &payout).await.is_some(),
        "FIXTURE: payout live"
    );
    let slash = r.slash();
    mine(&mut r, vec![slash], "the slash").await;
    assert!(
        entry_of(&r.node, &payout).await.is_none(),
        "NON-VACUITY: the slash burned the locked payout"
    );
    let want = snap(&r.node).await;
    r.step(Vec::new()).await;
    r.node.state_db.prune_undo_above(0);
    assert!(
        r.node.state_db.get_undo(want.height + 1).is_none(),
        "PRECONDITION: no undo data, so the UTXO set is rebuilt from blocks"
    );
    r.rollback().await;
    let got = snap(&r.node).await;
    assert!(
        entry_of(&r.node, &payout).await.is_none(),
        "the rebuild resurrected the burned payout"
    );
    assert_eq!((got.height, got.tip), (want.height, want.tip), "tip");
    assert_eq!(got.ops, want.ops, "rebuild vs apply: UTXO outpoints");
    assert_eq!(got.supply, want.supply, "rebuild vs apply: supply");
    assert_eq!(
        got.utxo_digest, want.utxo_digest,
        "rebuild vs apply: UTXO entries"
    );
}
