//! INC-I-288 E-2(a): at the heal, the node on the HEAVIER branch receives gossip
//! blocks of the LIGHTER branch whose parents it lacks. Orphan intake reports
//! `need_height = best_height + 1` as the network tip, so the coordinator sees a
//! gap of 1 that no peer reported, takes Rule 1 (orphans >= 2, small gap, recent
//! apply) and rolls back the heavier tip. The node then re-applies that block (R4)
//! or loses it to a sibling at the same height (R5).
//! Contract at the gate (devnet 0): orphans whose height is unknown do not raise the
//! network tip, and the heavier tip is not rolled back. Control: a node whose tip is
//! LIGHTER, with a peer status one block higher, still walks down one block.
//! Below-gate behavior: `inc_i_288_e2_below_ah.rs`.
//!
//! OUTPUT CONTRACT: fn Node::handle_new_block(orphan, peer) (BlockClass::Orphan ->
//!   SyncManager::note_orphan_gossip_child -> note_orphan_gossip_block), then
//!   fn Node::run_periodic_tasks() (evidence report -> classify_and_dispatch -> the
//!   ShallowRollback executor rollback_one_block(CoordinatorApproved))
//!   O1: chain_state best_hash / best_height, canonical index, persisted tip, sync tip
//!   O2: Node rollback counters (shallow_rollback_count, cumulative_rollback_depth)
//!   O3: rollback memory: our old tip is not a removed hash (no removal, so no
//!       re-apply of it)
//!   O4: SyncManager network_tip_height after the intake and after the tick (the gap
//!       the coordinator reads = network_tip_height - local height)
//!   O5: fork_block_cache holds every orphan (kept for when its parent arrives)
//!   O6: return values: handle_new_block Ok, run_periodic_tasks Ok (fixture expect)
//! PATHS:
//!   P1: heavier tip; orphans of the lighter branch; every peer status <= local height
//!   P2: lighter tip (control); orphans of the heavier branch; a peer status = local+1
//! INPUT PARTITIONS:
//!   P1a: 2 orphans (the legacy 3-orphan branch in note_orphan_gossip_block not run)
//!   P1b: 3 orphans (the legacy branch runs start_sync, then the coordinator acts)
//!   P2a: 2 orphans, real gap 1 from peer status -> one-block walk-down
//! MATRIX: P1a O1-O6 · P1b O1-O6 · P2a O1-O4,O6 (O5 N/A: the rollback path owns the
//!   cache after a walk-down; not what E-2 changes)
//!
//! covers: block_handling.rs (Orphan need_height) peers.rs (note_orphan_gossip_block)
//! covers: rollback_memory.rs (note_orphan_gossip_child) recovery.rs (Rule 1)
//! covers: block_lifecycle.rs (classify_and_dispatch) periodic.rs (coordinator tick)
//! covers: types.rs init.rs (SyncConfig gate)

use super::inc_i_288_e2_common::*;

/// P1 assertions: heavier tip `top` at `height` kept, no rollback, gap 0.
async fn assert_heavier_tip_kept(r: &Rig, o: &HealObs, height: u64) {
    let top = r.ours.last().expect("own branch").clone();
    let t = tip(r).await;
    assert_eq!(
        o.rollbacks_after, o.rollbacks_before,
        "E-2(a): the coordinator rolled back the heavier tip after lighter-branch orphan \
         gossip (no peer has a block above our tip; network tip after intake = {}, local = \
         {height}) — {o:?} {t:?}",
        o.net_after_intake
    );
    assert_on(r, &top, height, "E-2(a): the heavier tip stays the tip").await;
    assert!(
        !o.old_tip_removed,
        "E-2(a): the heavier tip is in rollback memory (removed, so a later re-apply)"
    );
    assert!(
        o.net_after_intake <= height,
        "E-2(a): orphan intake raised the network tip to {} above local {height} with no \
         peer status above {height} (the false gap)",
        o.net_after_intake
    );
    assert!(
        o.net_after_tick <= height,
        "E-2(a): network tip {} above local {height} after the tick",
        o.net_after_tick
    );
    assert!(
        o.orphans_cached.iter().all(|c| *c),
        "E-2(a): every orphan stays cached for its parent ({:?})",
        o.orphans_cached
    );
}

// E-2(a) INC-I-288 — Decision: a failure means two lighter-branch orphans still make the coordinator roll back the heavier tip on a gap no peer reported, the trigger of the measured wrong-branch heal and re-apply.
#[tokio::test]
async fn e2_a_inc_i_288_heavier_tip_survives_two_lighter_orphans() {
    // P1a: ours A1..A4 (F+4), theirs B1..B3 (F+3); slots interleave as in a partition.
    let mut r = rig(&[210, 212, 214, 216], &[211, 213, 215]).await;
    let (a4, b3) = (r.ours[3].clone(), r.theirs[2].clone());
    let orphans = r.theirs[1..].to_vec();
    let o = heal_tick(&mut r, &[(F + 4, a4), (F + 3, b3)], &orphans).await;
    assert_heavier_tip_kept(&r, &o, F + 4).await;
}

// E-2(a) INC-I-288 — Decision: a failure means the 3-orphan legacy branch (start_sync) still lets the coordinator roll back the heavier tip in the same tick, the shape seen with three orphans.
#[tokio::test]
async fn e2_a_inc_i_288_heavier_tip_survives_three_lighter_orphans() {
    // P1b: ours A1..A5 (F+5), theirs B1..B4 (F+4).
    let mut r = rig(&[210, 212, 214, 216, 218], &[211, 213, 215, 217]).await;
    let (a5, b4) = (r.ours[4].clone(), r.theirs[3].clone());
    let orphans = r.theirs[1..].to_vec();
    let o = heal_tick(&mut r, &[(F + 5, a5), (F + 4, b4)], &orphans).await;
    assert_heavier_tip_kept(&r, &o, F + 5).await;
}

// E-2(a) INC-I-288 — Decision: a failure means the fix also blinded a lighter-tip node to a gap its peers really report, so a node on the minority branch no longer walks down (the INV-FORK-001 stuck-minority wedge).
#[tokio::test]
async fn e2_a_inc_i_288_control_lighter_tip_still_walks_down() {
    // P2a: ours C1..C3 (F+3, lighter), theirs D1..D4 (F+4); D3, D4 arrive as orphans.
    let mut r = rig(&[211, 213, 215], &[210, 212, 214, 216]).await;
    let (c2, c3, d4) = (r.ours[1].clone(), r.ours[2].clone(), r.theirs[3].clone());
    let orphans = r.theirs[2..].to_vec();
    let o = heal_tick(&mut r, &[(F + 3, c3.clone()), (F + 4, d4)], &orphans).await;

    let (srb, depth) = o.rollbacks_before;
    assert_eq!(
        o.rollbacks_after,
        (srb + 1, depth + 1),
        "E-2(a) control: the lighter tip must walk down one block on a real gap — {o:?}"
    );
    assert_on(&r, &c2, F + 2, "E-2(a) control: walked down to C2").await;
    assert!(
        o.old_tip_removed,
        "E-2(a) control: C3 is remembered as removed by the fork-remedy rollback"
    );
    assert!(
        o.net_after_intake > F + 3 && o.net_after_tick > F + 2,
        "E-2(a) control: the peer-reported tip F+4 stays visible — {o:?}"
    );
}
