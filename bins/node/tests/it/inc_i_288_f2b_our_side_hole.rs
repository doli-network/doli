//! INC-I-288 F-2b SEC-INTEGRITY-R2-003 / REL2-288-P3-003: at/above the AH both
//! branches are weighed from the common ancestor. A block of OUR chain the reorg
//! handler does not track is weighed from the store (`wedge_escape::stored_weight`:
//! header + `CF_HASH_TO_HEIGHT`), and a miss adds 0 (`branch_weight` `unwrap_or(0)`),
//! so our side under-counts and a lighter candidate wins.
//!
//! The hole is built through the store API, as a damaged canonical index leaves it
//! (the INC-I-253 class): a restart empties the handler, then
//! `BlockStore::remove_canonical_entry(h, hash)` deletes the height-index and
//! hash-to-height rows of A2 and A3. Their headers and bodies stay, so our chain
//! still walks back to F by `prev_hash`. Ours A1..A3 (3) then reads 1; the lighter
//! S1, S2 (2) reads 2.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(S1 then S2, peer) -> Result<()> (wedge door)
//                  fn Node::handle_completed_fork_recovery({S1, S2}, F) -> Result<()> (walk)
//   O1: chain_state best_hash / best_height
//   O2: block_store canonical entry at F+1
//   O3: state_db persisted tip
//   O4: sync_manager local tip height
//   O5: block_store holds S1 (applied at least once = the node rolled back toward it)
//   O6: return value — Ok(())
// PATHS: P1 wedge door (S1 HeightOccupied, S2 Orphan reroute) · P2 completed fork walk
// INPUT PARTITIONS:
//   P1a/P2a: at/above the AH, restart + index holes at A2, A3 -> no reorg (A3 kept)
//   CONTROL: at/above the AH, restart, no hole -> no reorg (the red is the hole's)
//   BELOW-AH: P1 with the holes -> old rule (untracked tip weighs 0) -> rolls back toward S
// MATRIX: P1a O1-O6 · P2a O1-O6 · control P1,P2 O1,O5,O6 · below-AH P1 O5
//
//! covers: wedge_escape.rs fork_recovery.rs doors.rs mod.rs block_handling.rs

use doli_core::Block;
use network::{sync::CompletedRecovery, PeerId};

use super::inc_i_288_f2_common::*;

const OURS: [u32; 3] = [210, 211, 212];
const THEIRS: [u32; 2] = [216, 217];

#[derive(Clone, Copy)]
enum Door {
    Gossip,
    Walk,
}

/// Restart (empty handler), then a canonical-index hole at A2 and A3 when `hole`.
async fn restarted(hole: bool) -> Rig {
    let mut r = live_rig(&OURS, &THEIRS).await;
    restart(&mut r).await;
    if !hole {
        return r;
    }
    for (i, a) in r.ours.iter().enumerate().skip(1) {
        let h = F + 1 + i as u64;
        r.node
            .block_store
            .remove_canonical_entry(h, a.hash())
            .expect("fixture: remove_canonical_entry");
        assert!(
            r.node.block_store.get_header(&a.hash()).unwrap().is_some()
                && r.node
                    .block_store
                    .get_height_by_hash(&a.hash())
                    .unwrap()
                    .is_none(),
            "precondition: A{} keeps its header and has no hash-to-height row",
            i + 1
        );
    }
    r
}

/// Delivers the lighter branch S1, S2 through `door`; returns every result.
async fn deliver(r: &mut Rig, door: Door) -> Vec<anyhow::Result<()>> {
    let s: Vec<Block> = r.theirs.clone();
    match door {
        Door::Gossip => {
            let mut out = Vec::new();
            for b in &s {
                out.push(r.node.handle_new_block(b.clone(), PeerId::random()).await);
            }
            out
        }
        Door::Walk => {
            let done = CompletedRecovery {
                blocks: s,
                connection_point: r.f_hash,
                suppliers: Vec::new(),
            };
            vec![r.node.handle_completed_fork_recovery(done).await]
        }
    }
}

/// The heavier branch A1..A3 is kept and the lighter S was never applied.
async fn assert_kept_heavier(r: &Rig, res: &[anyhow::Result<()>], label: &str) {
    let (a1, a3) = (r.ours[0].hash(), r.ours[2].hash());
    let after = tip(r).await;
    assert!(
        !stored(r, &r.theirs[0]),
        "{label}: the node rolled back toward the lighter S1, S2 (S1 applied) — {after:?}"
    );
    assert_eq!(
        (after.best_hash, after.best_height),
        (a3, F + 3),
        "{label}: tip A3 kept"
    );
    assert_eq!(after.canonical_f1, Some(a1), "{label}: A1 stays canonical");
    assert_eq!(after.persisted, Some(a3), "{label}: persisted tip A3");
    assert_eq!(after.sync_height, F + 3, "{label}: sync manager tip");
    assert!(
        res.iter().all(|x| x.is_ok()),
        "{label}: every delivery returns Ok — {res:?}"
    );
}

// SEC-INTEGRITY-R2-003 REL2-288-P3-003 INC-I-288 — Decision: a failure means a canonical-index hole makes our stored blocks weigh 0, so the wedge door rolls a node back onto a LIGHTER branch.
#[tokio::test]
async fn f2b_z1_inc_i_288_index_hole_lighter_sibling_branch_does_not_win_door() {
    let mut r = restarted(true).await;
    let res = deliver(&mut r, Door::Gossip).await;
    assert_kept_heavier(&r, &res, "F-2b Z1 door").await;
}

// SEC-INTEGRITY-R2-003 INC-I-288 — Decision: a failure means a canonical-index hole makes our stored blocks weigh 0, so a completed fork walk rolls a node back onto a LIGHTER branch.
#[tokio::test]
async fn f2b_z2_inc_i_288_index_hole_lighter_walked_branch_does_not_win() {
    let mut r = restarted(true).await;
    let res = deliver(&mut r, Door::Walk).await;
    assert_kept_heavier(&r, &res, "F-2b Z2 walk").await;
}

// SEC-INTEGRITY-R2-003 INC-I-288 — Decision: a failure means Z1/Z2 do not isolate the index hole, because a restarted node already loses to the lighter branch with an intact index.
#[tokio::test]
async fn f2b_z0_inc_i_288_restart_without_hole_keeps_heavier_branch() {
    for door in [Door::Gossip, Door::Walk] {
        let mut r = restarted(false).await;
        let res = deliver(&mut r, door).await;
        let a3 = r.ours[2].hash();
        assert!(
            !stored(&r, &r.theirs[0]) && tip(&r).await.best_hash == a3,
            "F-2b Z0 control: with an intact index the restarted node keeps A3"
        );
        assert!(res.iter().all(|x| x.is_ok()), "F-2b Z0: Ok — {res:?}");
    }
}

// SEC-INTEGRITY-R2-003 INC-I-288 — Decision: a failure means the our-side refusal leaked below its activation height and changed pre-AH fork choice without a gate.
#[tokio::test]
async fn f2b_z3_inc_i_288_below_ah_index_hole_old_rule_kept() {
    if !in_below_ah_child(
        "inc_i_288_f2b_our_side_hole::f2b_z3_inc_i_288_below_ah_index_hole_old_rule_kept",
    ) {
        return;
    }
    let mut r = restarted(true).await;
    let _ = deliver(&mut r, Door::Gossip).await;
    assert!(
        stored(&r, &r.theirs[0]),
        "F-2b Z3 below AH: the old door (untracked tip weighs 0) still rolls back toward S1"
    );
}
