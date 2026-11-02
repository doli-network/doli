//! INC-I-288 C-1 (a)-(c): fork choice after a coordinator depth-1 rollback, and a
//! better-slot sibling at an occupied height. Both must be decided by branch WEIGHT.
//!
//! Defect 1: `reset_sync_for_rollback` clears the ReorgHandler, so after a depth-1
//! rollback our tip weighs 0 (any first sibling wins with delta=1) and a retained
//! sibling loses its parent links (plan_reorg finds no common ancestor).
//! Defect 2: the FORK_GUARD `is_better` arm drops a better-slot sibling without
//! weighing it, so a heavier branch whose fork-point block has the better slot is
//! never adopted.
//! The fix is gated by `inc_i_288_fork_choice_activation_height` (devnet 0). These
//! tests run on the devnet `Node::new_for_test` node, so they need the gate active
//! in BOTH the node (FORK_GUARD) and the SyncConfig that new_for_test builds.
//! Below-AH behavior: `inc_i_288_c1_below_ah.rs` (enabled with the new param).
//!
//! OUTPUT CONTRACT: fn Node::rollback_one_block(CoordinatorApproved { depth: 1 })
//!   -> SyncManager::reset_sync_for_rollback, then fn Node::handle_new_block(block, peer)
//!   (HeightOccupied arm: FORK_GUARD or retain_sibling_and_try_escape)
//!   O1: chain_state best_hash / best_height — the branch fork choice picked
//!   O2: block_store canonical entry at F+1 — which branch the index serves
//!   O3: state_db persisted chain-state tip — survives a restart
//!   O4: sync_manager local_tip height — what the sync side believes
//!   O5: ReorgHandler memory after the rollback — current_weight, chain_weight(new
//!       tip) (REQ-HEAL-006: equal to its pre-rollback value), parent links of our
//!       tip and of a retained sibling
//!   O6: weight-door record of a better-slot sibling — weight recorded, body
//!       cached, eligibility-validated retained set
//!   O7: stuck-fork signal — N/A for C-1: the weight door and the old FORK_GUARD
//!       both raise it; its policy is not what C-1 changes
//!   O8: return values — handle_new_block Ok(()), rollback RolledBack (fixture)
//! PATHS:
//!   P1: depth-1 rollback, then a worse-slot sibling reaches the wedge door
//!   P2: no rollback, a better-slot sibling at an occupied height (is_better=true)
//! INPUT PARTITIONS (unit producer weights, so weight = blocks above genesis):
//!   P1a: own branch F+5 (after rollback) vs one-block sibling F+1 -> lighter, keep
//!   P1b: sibling B1,B2 retained before the rollback; B3 makes it F+3 vs our F+2
//!        after the rollback -> heavier, reorg onto B3 in the same call
//!   P2a: better-slot branch B1..B3 (F+3) vs ours F+2 -> heavier, adopt B3
//!   P2b: better-slot branch B1,B2 (F+2) vs ours F+3 -> lighter, weighed, keep
//! MATRIX: P1a O1-O5 · P1b O1-O5 · P2a O1-O4,O6 · P2b O1-O4,O6 (O7 N/A, O8 fixture)
//!
//! covers: block_lifecycle.rs (reset_sync_for_rollback) block_handling.rs (FORK_GUARD)
//! covers: wedge_escape.rs rollback.rs init.rs (new_for_test SyncConfig)
//! covers: defaults.rs env_loader.rs types.rs (SyncConfig) mod.rs (NetworkParams, SyncManager, ReorgHandler)
//! covers: fork_choice.rs doors.rs recovery.rs

use super::inc_i_288_c1_common::*;

// REQ-HEAL-003 REQ-HEAL-006 C-1(a) INC-I-288 — Decision: a failure means a depth-1 rollback still zeroes our tip weight, so a node on the heavier branch jumps onto a lighter one-block sibling (the measured wrong-branch heal).
#[tokio::test]
async fn c1_a_inc_i_288_depth1_rollback_keeps_heavier_branch_against_lighter_sibling() {
    // P1a: own branch A1..A6 (F+1..F+6); sibling S on F with a worse slot than A1.
    let mut r = rig(&[210, 211, 212, 213, 214, 215], &[216]).await;
    let (a1, a4, a5, s) = (
        r.ours[0].clone(),
        r.ours[3].clone(),
        r.ours[4].clone(),
        r.theirs[0].clone(),
    );
    let w_a5_before = chain_weight(&r, &a5.hash()).await;
    assert_eq!(
        w_a5_before,
        F + 5,
        "precondition: A5 weighs F+5 before the rollback"
    );

    rollback_depth_one(&mut r).await;
    assert_on(
        &r,
        &a5,
        F + 5,
        "precondition: depth-1 rollback left A5 as tip",
    )
    .await;
    let (cur, w_a5, link_a5) = (
        current_weight(&r).await,
        chain_weight(&r, &a5.hash()).await,
        parent_link(&r, &a5.hash()).await,
    );

    gossip(&mut r, &s).await;
    let after = tip(&r).await;

    assert_ne!(
        after.best_hash,
        s.hash(),
        "C-1(a): the node reorged onto the lighter one-block sibling S (F+1) after a \
         depth-1 rollback, abandoning its own heavier branch (F+5) — {after:?}"
    );
    assert_on(&r, &a5, F + 5, "C-1(a): own heavier branch keeps the tip").await;
    assert_eq!(
        after.canonical_f1,
        Some(a1.hash()),
        "C-1(a): A1 stays canonical at F+1"
    );
    assert_eq!(
        w_a5, w_a5_before,
        "REQ-HEAL-006: chain_weight(new tip) after the rollback must equal its value before"
    );
    assert_eq!(
        cur,
        F + 5,
        "C-1(a): our current weight is the new tip's recorded weight"
    );
    assert_eq!(
        link_a5,
        Some(a4.hash()),
        "C-1(a): our tip keeps its parent link"
    );
}

// REQ-HEAL-003 C-1(a) INC-I-288 — Decision: a failure means a depth-1 rollback still wipes a retained sibling's parent links, so its heavier descendant hits plan_reorg None and the node walks down one block per cooldown.
#[tokio::test]
async fn c1_a_inc_i_288_depth1_rollback_keeps_retained_sibling_links() {
    // P1b: own A1..A3; sibling B1..B3 on F, every slot worse than A1.
    let mut r = rig(&[210, 211, 212], &[216, 217, 218]).await;
    let (a2, b1, b2, b3) = (
        r.ours[1].clone(),
        r.theirs[0].clone(),
        r.theirs[1].clone(),
        r.theirs[2].clone(),
    );
    gossip(&mut r, &b1).await;
    gossip(&mut r, &b2).await;
    assert_on(
        &r,
        &r.ours[2].clone(),
        F + 3,
        "precondition: lighter B1,B2 not adopted",
    )
    .await;
    assert_eq!(
        weighed(&r, &b2).await,
        (true, true, true),
        "precondition: B2 retained through the wedge door"
    );

    rollback_depth_one(&mut r).await;
    assert_on(
        &r,
        &a2,
        F + 2,
        "precondition: depth-1 rollback left A2 as tip",
    )
    .await;
    let (link_b2, link_b1, w_b2) = (
        parent_link(&r, &b2.hash()).await,
        parent_link(&r, &b1.hash()).await,
        chain_weight(&r, &b2.hash()).await,
    );

    gossip(&mut r, &b3).await;
    let after = tip(&r).await;

    assert_eq!(
        after.best_hash,
        b3.hash(),
        "C-1(a): heavier descendant B3 (F+3 vs our F+2) of the retained sibling was not \
         adopted after a depth-1 rollback (plan_reorg lost the branch) — {after:?}"
    );
    assert_on(
        &r,
        &b3,
        F + 3,
        "C-1(a): reorged onto the retained heavier branch",
    )
    .await;
    assert_eq!(
        after.canonical_f1,
        Some(b1.hash()),
        "C-1(a): B1 canonical at F+1"
    );
    assert_eq!(
        link_b2,
        Some(b1.hash()),
        "C-1(a): B2 keeps its link to B1 after the rollback"
    );
    assert_eq!(
        link_b1,
        Some(r.f_hash),
        "C-1(a): B1 keeps its link to F after the rollback"
    );
    assert_eq!(
        w_b2,
        F + 2,
        "C-1(a): retained B2 keeps its accumulated weight F+2"
    );
}

// C-1(control) INC-I-288 — Decision: a failure means the wedge door cannot land a heavier sibling in this rig (or the fix broke it), so the (a)/(b) reds are not caused by the rollback clear or the slot rule.
#[tokio::test]
async fn c1_control_inc_i_288_worse_slot_heavier_branch_adopted_without_rollback() {
    // P0 (green at 6cd0aba6): as P2a, but B1 has a WORSE slot than A1 and no rollback.
    let mut r = rig(&[210, 211], &[216, 217, 218]).await;
    let (b1, b2, b3) = (
        r.theirs[0].clone(),
        r.theirs[1].clone(),
        r.theirs[2].clone(),
    );
    for b in [&b1, &b2, &b3] {
        gossip(&mut r, b).await;
    }
    assert_on(
        &r,
        &b3,
        F + 3,
        "control: heavier worse-slot branch adopted by the door",
    )
    .await;
    assert_eq!(
        tip(&r).await.canonical_f1,
        Some(b1.hash()),
        "control: B1 canonical at F+1"
    );
}

// C-1(b) INC-I-288 — Decision: a failure means the slot rule, not weight, still decides a better-slot sibling, so a behind node never adopts the heavier branch whose fork point has the better slot.
#[tokio::test]
async fn c1_b_inc_i_288_better_slot_heavier_branch_is_weighed_and_adopted() {
    // P2a: own A1,A2 (slots 210, 211); their B1 has the BETTER slot 205, then B2, B3.
    // B3 at 215: three distinct producers vs our two (INC-I-289 K-1 weighs supporters).
    let mut r = rig(&[210, 211], &[205, 213, 215]).await;
    let (b1, b2, b3) = (
        r.theirs[0].clone(),
        r.theirs[1].clone(),
        r.theirs[2].clone(),
    );
    assert!(
        b1.header.slot < r.ours[0].header.slot,
        "precondition: B1 is the better-slot block at the occupied height F+1"
    );

    gossip(&mut r, &b1).await;
    let door_b1 = weighed(&r, &b1).await;
    gossip(&mut r, &b2).await;
    gossip(&mut r, &b3).await;
    let after = tip(&r).await;

    assert_eq!(
        after.best_hash,
        b3.hash(),
        "C-1(b): heavier branch B1..B3 (F+3 vs our F+2) with a better-slot fork point was \
         not adopted (B1 weighed={door_b1:?}) — {after:?}"
    );
    assert_on(
        &r,
        &b3,
        F + 3,
        "C-1(b): tip on the heavier better-slot branch",
    )
    .await;
    assert_eq!(
        after.canonical_f1,
        Some(b1.hash()),
        "C-1(b): B1 canonical at F+1"
    );
    assert_eq!(
        door_b1,
        (true, true, true),
        "C-1(b): B1 must reach the weight door (recorded, cached, retained)"
    );
}

// C-1(c) INC-I-288 — Decision: a failure means a lighter better-slot sibling is either dropped unweighed (the heavier extension can never win later) or adopted on its slot alone (lighter branch wins).
#[tokio::test]
async fn c1_c_inc_i_288_better_slot_lighter_sibling_is_weighed_not_adopted() {
    // P2b: own A1..A3 (slots 210-212); their B1 has the BETTER slot 205, then B2.
    let mut r = rig(&[210, 211, 212], &[205, 213]).await;
    let (a1, a3, b1, b2) = (
        r.ours[0].clone(),
        r.ours[2].clone(),
        r.theirs[0].clone(),
        r.theirs[1].clone(),
    );
    assert!(
        b1.header.slot < a1.header.slot,
        "precondition: B1 is the better-slot block at the occupied height F+1"
    );

    gossip(&mut r, &b1).await;
    let door_b1 = weighed(&r, &b1).await;
    gossip(&mut r, &b2).await;
    let door_b2 = weighed(&r, &b2).await;
    let after = tip(&r).await;

    assert_eq!(
        door_b1,
        (true, true, true),
        "C-1(c): better-slot sibling B1 was dropped without reaching the weight door \
         (recorded, cached, retained) — {after:?}"
    );
    assert_eq!(
        door_b2,
        (true, true, true),
        "C-1(c): B2 (child of the weighed B1) must be weighed too"
    );
    assert_on(
        &r,
        &a3,
        F + 3,
        "C-1(c): lighter better-slot branch (F+2) not adopted",
    )
    .await;
    assert_eq!(
        after.canonical_f1,
        Some(a1.hash()),
        "C-1(c): A1 stays canonical at F+1"
    );
}
