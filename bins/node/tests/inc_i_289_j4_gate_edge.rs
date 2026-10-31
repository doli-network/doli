//! QA-639-003: the INC-I-288 fork-choice gate edge for the INC-I-289 rule set.
//! Every gated site turns its rule ON when the height it compares is `>=` the
//! gate. Each mechanism is pinned with the gate AT the compared height (ON), one
//! above it (the height is gate-1: OFF), one below it (the height is gate+1: ON),
//! and across a restart (state written below the gate, node reopened at it).
//! Each test re-execs itself in a child with the gate env override
//! (`DOLI_INC_I_288_FORK_CHOICE_ACTIVATION_HEIGHT`; NetworkParams is a
//! per-process OnceLock). Devnet rig: four producers of bond 1, so "bond" is the
//! count of distinct producers and "depth" is the block count.
//!
//! Sites: doors `reorg/mod.rs:140` (local tip); finality persist
//! `production_gate.rs:544` (applied tip); finality restore `init.rs:869-879`
//! (stored tip at start); hold, drain order and walk seed via `branch_rank.rs:66-78`
//! (sync local tip).

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "inc_i_289_j4_support/mod.rs"]
mod j4;

use j4::scenarios::*;
use j4::*;

// ================================================================ fork choice
// OUTPUT CONTRACT (fork choice): fn Node::handle_new_block(B1, B2) -> gossip door
//   (bond union vs per-block weight), then run_periodic_tasks() x <= 5
//   O1 tip hash/height, canonical entry, persisted tip, sync tip (assert_on)
//   O2 block_store holds B2 (OFF: never) | O3 Ok(()) returns (helpers)
//   INPUT PARTITIONS: A = 3 blocks by 1 producer (tip FC_TIP); B = 2 blocks by 2
//   producers on F. ON -> B2 tip; OFF -> A3 kept, B2 not stored.

// QA-639-003 FC-AT — Decision: a failure means the doors do not weigh by bond at a local tip equal to the gate (the `>=` became `>`), so the first block of the activation height still decides by block count.
#[tokio::test]
async fn j4_fork_choice_at() {
    if !in_gate_child("j4_fork_choice_at", FC_TIP) {
        return;
    }
    let (mut r, b) = fork_choice_rig().await;
    fork_choice_decide(&mut r, &b, FC_TIP, Gate::On, "FC-AT").await;
}

// QA-639-003 FC-BELOW — Decision: a failure means the bond rule leaks one block before the gate (the doors switch at tip = gate-1).
#[tokio::test]
async fn j4_fork_choice_below() {
    if !in_gate_child("j4_fork_choice_below", FC_TIP + 1) {
        return;
    }
    let (mut r, b) = fork_choice_rig().await;
    fork_choice_decide(&mut r, &b, FC_TIP, Gate::Off, "FC-BELOW").await;
}

// QA-639-003 FC-ABOVE — Decision: a failure means the bond rule does not hold one block past the gate.
#[tokio::test]
async fn j4_fork_choice_above() {
    if !in_gate_child("j4_fork_choice_above", FC_TIP - 1) {
        return;
    }
    let (mut r, b) = fork_choice_rig().await;
    fork_choice_decide(&mut r, &b, FC_TIP, Gate::On, "FC-ABOVE").await;
}

// OUTPUT CONTRACT (fork choice restart): gate FC_TIP+1. Before: door at tip
//   FC_TIP (OFF). Raw apply A4 (tip = gate), Node::new on the data dir. After:
//   O1 tip restored A4 | O2 B1, B2 not in the fork cache, no recorded weight
//   O3 A5,A6 (2 producers) applied after the restart; C (3 blocks, 1 producer)
//   on A4 gossiped: door at tip gate+2 keeps A6, C3 never stored (ON; the count
//   rule would reorg to C3). A keep cell, not a reorg cell: in this rig every
//   reorg after the reopen is refused with `[FORK_GUARD_BACKFILL_REQUIRED] ...
//   missing canonical block at height 1` (observed), which is not this gate.
// QA-639-003 FC-RESTART — Decision: a failure means fork-choice state from below the gate survives a restart onto it, or the restarted node still weighs by block count at the gate.
#[tokio::test]
async fn j4_fork_choice_restart() {
    if !in_gate_child("j4_fork_choice_restart", FC_TIP + 1) {
        return;
    }
    let (mut r, b) = fork_choice_rig().await;
    fork_choice_decide(&mut r, &b, FC_TIP, Gate::Off, "FC-RESTART before").await;
    extend(&mut r, 1).await;
    let mut r = reopen(r).await;
    let a4 = r.ours[3].clone();
    assert_on(&r, &a4, FC_TIP + 1, "FC-RESTART: reopen restores A4").await;
    for blk in &b {
        assert!(
            !cached(&r, &blk.hash()).await && !recorded(&r, &blk.hash()).await,
            "FC-RESTART: B is still cached or weighed after the restart"
        );
    }
    let more = chain_by(&r, a4.hash(), FC_TIP + 2, &[2, 3], a4.header.slot);
    for blk in &more {
        apply(&mut r, blk).await;
    }
    let c = chain_by(&r, a4.hash(), FC_TIP + 2, &[1, 1, 1], more[1].header.slot);
    assert_rules_disagree(&r, &c, &more, FC_TIP + 2, "FC-RESTART after").await;
    for blk in &c {
        gossip(&mut r, blk).await;
    }
    for _ in 0..3 {
        tick(&mut r).await;
    }
    let l = "FC-RESTART after: ON — A5,A6 (2 producers) must keep the tip against C's 3 blocks";
    assert_on(&r, &more[1], FC_TIP + 3, l).await;
    assert!(!stored(&r, &c[2]), "FC-RESTART after: C3 was applied");
}

// =================================================================== finality
// OUTPUT CONTRACT (finality): fn SyncManager::add_attestation_weight -> finality
//   sink (persist) at applied tip FIN_TIP; fn Node::new (reopen) -> restore
//   O1 state_db finality checkpoint (height, hash) | O2 last_finalized_height
//   after reopen | O3 tip after reopen
//   INPUT PARTITIONS: A1 finalized at applied tip FIN_TIP (= stored tip at the
//   reopen). ON -> written (FIN_TIP-2, A1) and restored; OFF -> none, None.

// QA-639-003 FIN-AT — Decision: a failure means a block finalized at an applied tip equal to the gate is not persisted or not restored, so a restart at the activation height forgets finality.
#[tokio::test]
async fn j4_finality_at() {
    if !in_gate_child("j4_finality_at", FIN_TIP) {
        return;
    }
    finality(Gate::On).await;
}

// QA-639-003 FIN-BELOW — Decision: a failure means the checkpoint is persisted or restored one block before the gate.
#[tokio::test]
async fn j4_finality_below() {
    if !in_gate_child("j4_finality_below", FIN_TIP + 1) {
        return;
    }
    finality(Gate::Off).await;
}

// QA-639-003 FIN-ABOVE — Decision: a failure means the checkpoint is not persisted and restored one block past the gate.
#[tokio::test]
async fn j4_finality_above() {
    if !in_gate_child("j4_finality_above", FIN_TIP - 1) {
        return;
    }
    finality(Gate::On).await;
}

// OUTPUT CONTRACT (finality restart): gate FIN_TIP+1. A1 finalized at tip
//   FIN_TIP (below): O1 no checkpoint; raw apply A4 (tip = gate): O1 still none;
//   reopen: O2 None; A5..A7 applied, A5 finalized at tip gate+3: O1 (A5) written;
//   reopen: O2 Some(A5 height).
// QA-639-003 FIN-RESTART — Decision: a failure means a finality checkpoint reached below the gate is restored after an upgrade restart onto it, or one reached above it is lost.
#[tokio::test]
async fn j4_finality_restart() {
    if !in_gate_child("j4_finality_restart", FIN_TIP + 1) {
        return;
    }
    let mut r = rig(&[1, 1, 1]).await;
    finalize(&r, F + 1).await;
    assert_eq!(
        persisted_cp(&r),
        None,
        "FIN-RESTART: written below the gate"
    );
    extend(&mut r, 1).await;
    assert_eq!(
        persisted_cp(&r),
        None,
        "FIN-RESTART: crossing apply wrote one"
    );
    let mut r = reopen(r).await;
    assert_eq!(tip(&r).await.1, FIN_TIP + 1, "reopen restores the tip");
    assert_eq!(
        finalized(&r).await,
        None,
        "FIN-RESTART: finality reached below the gate came back after the restart"
    );
    extend(&mut r, 3).await;
    finalize(&r, F + 5).await;
    let want = Some((F + 5, r.ours[4].hash()));
    assert_eq!(persisted_cp(&r), want, "FIN-RESTART: not written above");
    let r = reopen(r).await;
    assert_eq!(
        finalized(&r).await,
        Some(F + 5),
        "FIN-RESTART: not restored"
    );
}

// OUTPUT CONTRACT (finality restore edge): gate FIN_TIP. A1 finalized at tip
//   gate: O1 written; rollback to gate-1, reopen: O2 None, O1 kept; raw re-apply
//   A3 (tip = gate), reopen: O2 Some(A1 height).
// QA-639-003 FIN-RESTART-ROLLED-BACK — Decision: a failure means the restore compares the stored tip with the wrong operator (it restores at gate-1 or skips at gate), or a restart below the gate deletes the key.
#[tokio::test]
async fn j4_finality_restart_rolled_back() {
    if !in_gate_child("j4_finality_restart_rolled_back", FIN_TIP) {
        return;
    }
    let mut r = rig(&[1, 1, 1]).await;
    finalize(&r, F + 1).await;
    let want = Some((F + 1, r.ours[0].hash()));
    assert_eq!(persisted_cp(&r), want, "fixture: written at the gate");
    rollback_depth_one(&mut r).await;
    let mut r = reopen(r).await;
    assert_eq!(tip(&r).await.1, FIN_TIP - 1, "reopen at gate-1");
    assert_eq!(
        finalized(&r).await,
        None,
        "FIN-RB: restored at stored tip gate-1"
    );
    assert_eq!(
        persisted_cp(&r),
        want,
        "FIN-RB: the key was dropped below the gate"
    );
    let a3 = r.ours[2].clone();
    apply(&mut r, &a3).await;
    let r = reopen(r).await;
    assert_eq!(tip(&r).await.1, FIN_TIP, "reopen at the gate");
    assert_eq!(
        finalized(&r).await,
        Some(F + 1),
        "FIN-RB: not restored at the gate"
    );
}

// ======================================================================= hold
// OUTPUT CONTRACT (hold): fn Node::handle_new_block(D) -> hold_removed_block
//   (Bond arm: union from the tip; Depth arm: recorded weight) -> apply/drain
//   O1 block_store canonical entry at HOLD_TIP+1 (D or none) | O2 tip (OFF: stays)
//   INPUT PARTITIONS: D (1 producer) + cached child E1 (another) vs cached
//   sibling S1 (1 producer, recorded HEAVY). ON -> D released; OFF -> D held.

// QA-639-003 HOLD-AT — Decision: a failure means the hold still ranks by recorded per-block weight at a tip equal to the gate.
#[tokio::test]
async fn j4_hold_at() {
    if !in_gate_child("j4_hold_at", HOLD_TIP) {
        return;
    }
    let mut r = rig(&[1, 1]).await;
    hold_at(&mut r, Gate::On, "HOLD-AT").await;
}

// QA-639-003 HOLD-BELOW — Decision: a failure means the bond hold leaks one block before the gate.
#[tokio::test]
async fn j4_hold_below() {
    if !in_gate_child("j4_hold_below", HOLD_TIP + 1) {
        return;
    }
    let mut r = rig(&[1, 1]).await;
    hold_at(&mut r, Gate::Off, "HOLD-BELOW").await;
}

// QA-639-003 HOLD-ABOVE — Decision: a failure means the bond hold does not hold one block past the gate.
#[tokio::test]
async fn j4_hold_above() {
    if !in_gate_child("j4_hold_above", HOLD_TIP - 1) {
        return;
    }
    let mut r = rig(&[1, 1]).await;
    hold_at(&mut r, Gate::On, "HOLD-ABOVE").await;
}

// OUTPUT CONTRACT (hold restart): gate HOLD_TIP+1. Before: hold at tip HOLD_TIP
//   (OFF, D held). Raw apply S1 (tip = gate), reopen: O3 tip S1; O4 D not
//   removed, S1 not cached, no recorded weight. After: hold at tip = gate (ON).
// QA-639-003 HOLD-RESTART — Decision: a failure means rollback memory, cached siblings or recorded weights from below the gate survive a restart, or the restarted node holds by recorded weight at the gate.
#[tokio::test]
async fn j4_hold_restart() {
    if !in_gate_child("j4_hold_restart", HOLD_TIP + 1) {
        return;
    }
    let mut r = rig(&[1, 1]).await;
    let (d, s1) = hold_at(&mut r, Gate::Off, "HOLD-RESTART before").await;
    apply(&mut r, &s1).await;
    let mut r = reopen(r).await;
    assert_on(&r, &s1, HOLD_TIP + 1, "HOLD-RESTART: reopen restores S1").await;
    assert!(
        !removed(&r, &d.hash()).await,
        "HOLD-RESTART: D still removed"
    );
    assert!(
        !cached(&r, &s1.hash()).await && !recorded(&r, &s1.hash()).await,
        "HOLD-RESTART: S1 still cached or weighed"
    );
    hold_at(&mut r, Gate::On, "HOLD-RESTART after").await;
}

// ====================================================================== drain
// OUTPUT CONTRACT (drain order): fn Node::handle_new_block(X) -> apply X at
//   DRAIN_TIP -> drain_cached_children in drain_order (two children of X)
//   O1 canonical entry at DRAIN_TIP+1 (S1 or L1) | O2 tip (S2 at +2 / L4 at +4)
//   INPUT PARTITIONS: S = 2 blocks by 2 producers, L = 4 blocks by 1, both on X.
//   ON -> S first; OFF -> L first.

// QA-639-003 DRAIN-AT — Decision: a failure means the drain still applies the deepest branch first at a tip equal to the gate.
#[tokio::test]
async fn j4_drain_at() {
    if !in_gate_child("j4_drain_at", DRAIN_TIP) {
        return;
    }
    let mut r = rig(&[3]).await;
    drain_at(&mut r, Gate::On, "DRAIN-AT").await;
}

// QA-639-003 DRAIN-BELOW — Decision: a failure means the bond drain order leaks one block before the gate.
#[tokio::test]
async fn j4_drain_below() {
    if !in_gate_child("j4_drain_below", DRAIN_TIP + 1) {
        return;
    }
    let mut r = rig(&[3]).await;
    drain_at(&mut r, Gate::Off, "DRAIN-BELOW").await;
}

// QA-639-003 DRAIN-ABOVE — Decision: a failure means the bond drain order does not hold one block past the gate.
#[tokio::test]
async fn j4_drain_above() {
    if !in_gate_child("j4_drain_above", DRAIN_TIP - 1) {
        return;
    }
    let mut r = rig(&[3]).await;
    drain_at(&mut r, Gate::On, "DRAIN-ABOVE").await;
}

// OUTPUT CONTRACT (drain restart): gate DRAIN_TIP. Before (tip gate-1): S, L
//   cached orphans on X; raw apply X (tip = gate, nothing drains). Reopen: O3 tip
//   X; O4 S, L not cached and not stored. After: drain at gate+1 (ON).
// QA-639-003 DRAIN-RESTART — Decision: a failure means cached children from below the gate survive a restart and drain without fork choice, or the restarted node drains by depth.
#[tokio::test]
async fn j4_drain_restart() {
    if !in_gate_child("j4_drain_restart", DRAIN_TIP) {
        return;
    }
    let mut r = rig(&[3]).await;
    let (x, s, l) = drain_set(&r).await;
    cache_orphans(&mut r, &s, &l, "DRAIN-RESTART before").await;
    apply(&mut r, &x).await;
    let mut r = reopen(r).await;
    assert_on(&r, &x, DRAIN_TIP, "DRAIN-RESTART: reopen restores X").await;
    for b in s.iter().chain(&l) {
        assert!(
            !cached(&r, &b.hash()).await && !stored(&r, b),
            "DRAIN-RESTART: {:.8} survived the restart",
            b.hash()
        );
    }
    drain_at(&mut r, Gate::On, "DRAIN-RESTART after").await;
}

// ======================================================================= walk
// OUTPUT CONTRACT (walk seed): fn Node::try_trigger_fork_recovery() -> bool
//   O1 return value (a walk started) | O2 fork_recovery_current_parent (S1 / L1)
//   INPUT PARTITIONS: cached S2,S3 (2 producers) vs cached L2..L5 (1 producer),
//   node on A1 at WALK_TIP. ON -> seeds S2; OFF -> seeds L2.

// QA-639-003 WALK-AT — Decision: a failure means the walk still seeds the deepest cached branch at a tip equal to the gate.
#[tokio::test]
async fn j4_walk_seed_at() {
    if !in_gate_child("j4_walk_seed_at", WALK_TIP) {
        return;
    }
    let mut r = rig(&[3]).await;
    let (s, l) = walk_set(&r);
    walk_at(&mut r, &s, &l, Gate::On, "WALK-AT").await;
}

// QA-639-003 WALK-BELOW — Decision: a failure means the bond seed rule leaks one block before the gate.
#[tokio::test]
async fn j4_walk_seed_below() {
    if !in_gate_child("j4_walk_seed_below", WALK_TIP + 1) {
        return;
    }
    let mut r = rig(&[3]).await;
    let (s, l) = walk_set(&r);
    walk_at(&mut r, &s, &l, Gate::Off, "WALK-BELOW").await;
}

// QA-639-003 WALK-ABOVE — Decision: a failure means the bond seed rule does not hold one block past the gate.
#[tokio::test]
async fn j4_walk_seed_above() {
    if !in_gate_child("j4_walk_seed_above", WALK_TIP - 1) {
        return;
    }
    let mut r = rig(&[3]).await;
    let (s, l) = walk_set(&r);
    walk_at(&mut r, &s, &l, Gate::On, "WALK-ABOVE").await;
}

// OUTPUT CONTRACT (walk restart): gate WALK_TIP+1. Before: walk at tip WALK_TIP
//   (OFF, seeds L2). Raw apply A2 (tip = gate), reopen: O3 tip A2; O4 no walk
//   active, S/L not cached, no recorded weight. After: walk at the gate (ON).
// QA-639-003 WALK-RESTART — Decision: a failure means a walk or its cached seeds from below the gate survive a restart, or the restarted node seeds by depth at the gate.
#[tokio::test]
async fn j4_walk_seed_restart() {
    if !in_gate_child("j4_walk_seed_restart", WALK_TIP + 1) {
        return;
    }
    let mut r = rig(&[3]).await;
    let (s, l) = walk_set(&r);
    walk_at(&mut r, &s, &l, Gate::Off, "WALK-RESTART before").await;
    let a1 = r.ours[0].clone();
    let a2 = chain_by(&r, a1.hash(), WALK_TIP + 1, &[3], l[4].header.slot).remove(0);
    apply(&mut r, &a2).await;
    let mut r = reopen(r).await;
    assert_on(&r, &a2, WALK_TIP + 1, "WALK-RESTART: reopen restores A2").await;
    assert!(
        !r.node.sync_manager.read().await.is_fork_recovery_active(),
        "WALK-RESTART: the walk survived the restart"
    );
    for b in s[1..].iter().chain(&l[1..]) {
        assert!(
            !cached(&r, &b.hash()).await && !recorded(&r, &b.hash()).await,
            "WALK-RESTART: a seed survived the restart"
        );
    }
    walk_at(&mut r, &s, &l, Gate::On, "WALK-RESTART after").await;
}
