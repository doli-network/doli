//! INC-I-288 F-2 AUDIT-P2-629-004: the INC-I-281 hold (`hold_removed_block`)
//! holds on `Heavier | TieSwitch`, while the weight door needs a STRICTLY heavier
//! branch. At/above the AH the rollback keeps fork-choice memory, so an
//! equal-weight, lower-hash sibling S the door refused makes the hold refuse our
//! own rolled-back tip T. Live rig (M5 live, so the tie verdict is TieSwitch).
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(T, peer) -> Result<()>, T = our tip
//   removed by a coordinator depth-1 rollback, its tied lower-hash sibling S cached
//   O1: chain_state best_hash / best_height (assert_on)
//   O2: block_store canonical entry at F+3 and F+1
//   O3: state_db persisted tip
//   O4: sync_manager local_tip height
//   O5: ReorgHandler memory of S after the rollback (kept above the AH, cleared below)
//   O6: return value — Ok(()) (asserted by the gossip helper)
// PATHS: P1 ExtendsTip re-delivery of T through hold_removed_block
// INPUT PARTITIONS:
//   P1a: at/above the AH, S weight == T weight, hash(S) < hash(T) -> T applies at once
//   BELOW-AH: memory cleared, S weighs 0 -> T applies at once (old behavior kept)
//   (a strictly heavier S is the hold's purpose and is held: C-1 / INC-I-281 suites)
// MATRIX: P1a O1-O6 · below-AH O1-O6
//
//! covers: fork_cache.rs wedge_escape.rs fork_choice.rs block_handling.rs block_lifecycle.rs

use crypto::Hash;
use doli_core::Block;

use super::inc_i_288_f2_common::*;

/// The first slot after T whose leader's block on T's parent hashes below T.
fn lower_hash_sibling(r: &Rig, t: &Block, parent: Hash) -> Block {
    let p = r.node.params.clone();
    (t.header.slot + 1..t.header.slot + 200)
        .map(|s| block(F + 3, s, parent, leader(&r.keys, s), &p))
        .find(|b| b.hash().as_bytes() < t.hash().as_bytes())
        .expect("fixture: a lower-hash sibling within 200 slots")
}

/// Tip T = A3; S tied with T, lower hash, retained by the door; then the
/// coordinator rolls T back. Returns (rig, T, S weight after the rollback).
async fn tie_then_rollback() -> (Rig, Block, u64) {
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let (a2, t) = (r.ours[1].clone(), r.ours[2].clone());
    let s = lower_hash_sibling(&r, &t, a2.hash());
    gossip(&mut r, &s).await;
    assert_on(&r, &t, F + 3, "precondition: the door refused the tied S").await;
    assert_eq!(
        weighed(&r, &s).await,
        (true, true, true),
        "precondition: S retained through the weight door"
    );
    assert_eq!(
        (
            chain_weight(&r, &s.hash()).await,
            chain_weight(&r, &t.hash()).await
        ),
        (F + 3, F + 3),
        "precondition: S and T weigh the same"
    );

    rollback_depth_one(&mut r).await;
    assert_on(
        &r,
        &a2,
        F + 2,
        "precondition: depth-1 rollback left A2 as tip",
    )
    .await;
    assert!(
        r.node
            .sync_manager
            .read()
            .await
            .removed_by_rollback(&t.hash()),
        "precondition: T is remembered as removed by a fork-remedy rollback"
    );
    let w_s = chain_weight(&r, &s.hash()).await;
    (r, t, w_s)
}

// AUDIT-P2-629-004 INC-I-288 — Decision: a failure means the hold switches on a tie the weight door refused, so our own rolled-back tip is held up to 300 s and one honest block can be orphaned.
#[tokio::test]
async fn f2_004_inc_i_288_tied_lower_hash_sibling_does_not_hold_own_tip() {
    let (mut r, t, w_s) = tie_then_rollback().await;
    assert_eq!(
        w_s,
        F + 3,
        "precondition: the rollback kept S's weight (above the AH)"
    );

    gossip(&mut r, &t).await;
    let after = tip(&r).await;

    assert_eq!(
        after.best_hash,
        t.hash(),
        "F-2 004: our tip T was held by a TIED lower-hash sibling (the door needs strictly \
         heavier) — {after:?}"
    );
    assert_on(&r, &t, F + 3, "F-2 004: T re-applied at once").await;
    assert_eq!(
        after.canonical_f1,
        Some(r.ours[0].hash()),
        "F-2 004: A1 stays canonical"
    );
}

// AUDIT-P2-629-004 INC-I-288 — Decision: a failure means the hold change leaked below its activation height, so pre-AH re-apply of a rolled-back tip changed without a gate.
#[tokio::test]
async fn f2_004_inc_i_288_below_ah_own_tip_reapplies() {
    if !in_below_ah_child("inc_i_288_f2_tie_hold::f2_004_inc_i_288_below_ah_own_tip_reapplies") {
        return;
    }
    let (mut r, t, w_s) = tie_then_rollback().await;
    assert_eq!(
        w_s, 0,
        "F-2 004 below AH: the rollback still clears S's weight"
    );

    gossip(&mut r, &t).await;
    assert_on(
        &r,
        &t,
        F + 3,
        "F-2 004 below AH: T re-applies at once (old behavior)",
    )
    .await;
}
