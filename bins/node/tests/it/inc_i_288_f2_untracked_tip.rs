//! INC-I-288 F-2 AUDIT-P2-629-001 / AUDIT-P2-629-002: an untracked tip reads as
//! weight 0 (`ReorgHandler::chain_weight` `unwrap_or(0)`), so the weight door
//! sees ANY sibling as heavier and `plan_reorg` gives `weight_delta = w - 0 > 0`.
//! 001 makes the tip untracked by a restart (a new SyncManager, only
//! `update_local_tip`); 002 by re-recording one sibling until the LRU (no dedup)
//! evicts the tip. Live rig: M5 live, every producer weighs 1.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) -> Result<()>, sibling S of
//   a lighter branch at an occupied height, routed to retain_sibling_and_try_escape
//   O1: chain_state best_hash / best_height (assert_on)
//   O2: block_store canonical entries at F+1 and at the tip height
//   O3: state_db persisted tip
//   O4: sync_manager local_tip height
//   O5: ReorgHandler memory of our tip (knows_block, chain_weight) after the event
//   O6: return value — Ok(()) (asserted by the gossip helper)
// PATHS: P1 restart then one sibling (001) | P2 the same sibling re-recorded past the cap (002)
// INPUT PARTITIONS:
//   P1a: worse-slot sibling at F+1, ours F+6 (door on both sides of the AH)
//   P1b: better-slot sibling at F+1, ours F+6 (door only at/above the AH)
//   P2a: worse-slot sibling at F+1 delivered max_tracked+64 times, ours F+6
//   BELOW-AH: P1a and P2a with the AH above every height -> the old reorg onto S stays
// MATRIX: P1a/P1b/P2a O1-O6 · below-AH P1a/P2a O1,O2,O6
//
//! covers: doors.rs fork_choice.rs mod.rs wedge_escape.rs fork_cache.rs block_handling.rs block_lifecycle.rs init.rs apply_block reorg_restore.rs rollback_memory.rs

use super::inc_i_288_f2_common::*;

/// `ReorgHandler::new()`'s `max_tracked` (private to the network crate).
const MAX_TRACKED: usize = 10_000;
const FLOOD: usize = MAX_TRACKED + 64;
const OURS: [u32; 6] = [210, 211, 212, 213, 214, 215];

async fn restart_then_sibling(their_slot: u32) -> (Rig, Tip) {
    let mut r = live_rig(&OURS, &[their_slot]).await;
    let a6 = r.ours[5].clone();
    assert_eq!(
        chain_weight(&r, &a6.hash()).await,
        F + 6,
        "precondition: A6 weighs F+6 before the restart"
    );
    restart(&mut r).await;
    assert!(
        !knows(&r, &a6.hash()).await && chain_weight(&r, &a6.hash()).await == 0,
        "precondition: after the restart the handler holds no entry for the stored tip"
    );
    assert_on(&r, &a6, F + 6, "precondition: restart kept tip A6").await;
    let s = r.theirs[0].clone();
    gossip(&mut r, &s).await;
    let after = tip(&r).await;
    (r, after)
}

async fn assert_kept_heavier(r: &Rig, after: &Tip, label: &str) {
    let (a1, a6) = (r.ours[0].clone(), r.ours[5].clone());
    assert_eq!(
        after.best_hash,
        a6.hash(),
        "{label}: a one-block sibling (F+1) won against our stored F+6 branch — {after:?}"
    );
    assert_on(r, &a6, F + 6, label).await;
    assert_eq!(
        after.canonical_f1,
        Some(a1.hash()),
        "{label}: A1 stays canonical"
    );
}

// AUDIT-P2-629-001 INC-I-288 — Decision: a failure means a restarted node takes a lighter worse-slot sibling and rolls back its heavier stored branch, because its untracked tip weighs 0.
#[tokio::test]
async fn f2_001_inc_i_288_restart_untracked_tip_keeps_heavier_branch() {
    let (r, after) = restart_then_sibling(216).await;
    assert_kept_heavier(&r, &after, "F-2 001 P1a").await;
}

// AUDIT-P2-629-001 INC-I-288 — Decision: a failure means the widened better-slot carrier lets a restarted node adopt a lighter sibling at/above the AH.
#[tokio::test]
async fn f2_001_inc_i_288_restart_better_slot_sibling_not_adopted() {
    let (r, after) = restart_then_sibling(205).await;
    assert_kept_heavier(&r, &after, "F-2 001 P1b").await;
}

// AUDIT-P2-629-001 INC-I-288 — Decision: a failure means the restart fix leaked below its activation height and changed pre-AH fork choice without a gate.
#[tokio::test]
async fn f2_001_inc_i_288_below_ah_restart_old_reorg_kept() {
    if !in_below_ah_child(
        "inc_i_288_f2_untracked_tip::f2_001_inc_i_288_below_ah_restart_old_reorg_kept",
    ) {
        return;
    }
    let (r, after) = restart_then_sibling(216).await;
    let s = r.theirs[0].clone();
    assert_on(
        &r,
        &s,
        F + 1,
        "F-2 001 below AH: the old reorg onto S is kept",
    )
    .await;
    assert_eq!(
        after.canonical_f1,
        Some(s.hash()),
        "F-2 001 below AH: S canonical"
    );
}

/// Delivers S up to FLOOD times; returns (record # at which A6 left the
/// handler, record # at which the tip moved).
async fn flood(r: &mut Rig) -> (Option<usize>, Option<usize>) {
    let (a6, s) = (r.ours[5].hash(), r.theirs[0].clone());
    let (mut evicted_at, mut moved_at) = (None, None);
    for i in 1..=FLOOD {
        gossip(r, &s).await;
        if evicted_at.is_none() && !knows(r, &a6).await {
            evicted_at = Some(i);
        }
        if r.node.chain_state.read().await.best_hash != a6 {
            moved_at = Some(i);
            break;
        }
    }
    (evicted_at, moved_at)
}

// AUDIT-P2-629-002 INC-I-288 — Decision: a failure means re-records of one sibling hash evict our tip's weight, so a lighter sibling wins the weight door without a rollback or restart.
#[tokio::test]
async fn f2_002_inc_i_288_re_recorded_sibling_does_not_evict_tip_weight() {
    let mut r = live_rig(&OURS, &[216]).await;
    let a6 = r.ours[5].clone();
    let (evicted_at, moved_at) = flood(&mut r).await;
    let after = tip(&r).await;

    assert_eq!(
        moved_at, None,
        "F-2 002: the lighter sibling S won after {moved_at:?} records of S (our tip left the \
         ReorgHandler at record {evicted_at:?}) — {after:?}"
    );
    assert_kept_heavier(&r, &after, "F-2 002 P2a").await;
    assert!(
        knows(&r, &a6.hash()).await,
        "F-2 002: our tip A6 was evicted by re-records of one hash"
    );
    assert_eq!(
        chain_weight(&r, &a6.hash()).await,
        F + 6,
        "F-2 002: our tip keeps its weight F+6"
    );
    assert_eq!(
        chain_weight(&r, &r.theirs[0].hash()).await,
        F + 1,
        "F-2 002: S keeps its weight F+1"
    );
}

// AUDIT-P2-629-002 INC-I-288 — Decision: a failure means the LRU fix leaked below its activation height and changed pre-AH eviction and fork choice without a gate.
#[tokio::test]
async fn f2_002_inc_i_288_below_ah_flood_old_eviction_kept() {
    if !in_below_ah_child(
        "inc_i_288_f2_untracked_tip::f2_002_inc_i_288_below_ah_flood_old_eviction_kept",
    ) {
        return;
    }
    let mut r = live_rig(&OURS, &[216]).await;
    let s = r.theirs[0].clone();
    let (evicted_at, moved_at) = flood(&mut r).await;
    assert!(
        matches!((evicted_at, moved_at), (Some(e), Some(m)) if e <= m),
        "F-2 002 below AH: the old eviction then reorg is kept (evicted {evicted_at:?}, moved {moved_at:?})"
    );
    assert_on(
        &r,
        &s,
        F + 1,
        "F-2 002 below AH: the old reorg onto S is kept",
    )
    .await;
}
