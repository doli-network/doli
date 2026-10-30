//! INC-I-288 F-2c (R) QA P3-630R3-009: after a restart the reorg handler is empty,
//! so the slot of a walked block's parent is unknown unless the walk passes the
//! connection point's slot from the store (`fork_recovery.rs` `cp_slot`). Without
//! that hint the first walked block B1 (parent = the connection point F) records 0,
//! the heavier branch B1..B3 (3) ties our A1, A2 (2) and is lost. This guards the
//! hint: a heavier branch reached by the fork walk after a restart is adopted.
//! Live rig (M5 live), production weights (1 per producer).
//!
// OUTPUT CONTRACT: fn Node::handle_completed_fork_recovery(CompletedRecovery { B1..B3 }, F)
//   -> Result<()>, after a process restart (`restart`: new SyncManager, empty fork cache)
//   O1: ReorgHandler recorded producer weight of B1, B2, B3
//   O2: chain_state best_hash / best_height, canonical, persisted, sync tip (assert_on B3)
//   O3: block_store canonical entry at F+1 (= B1)
//   O4: return value — Ok(())
// PATHS: P1 completed fork walk connected at F -> evaluate_recovered_fork -> reorg
// INPUT PARTITIONS:
//   R1: at/above the AH, restart, walk B1..B3 (slots advance; B1's parent F is stored,
//       not tracked) -> weights [1, 1, 1], tip B3 at F+3
//   R0 BELOW-AH: R1 shape -> the old walk also adopts the heavier branch (tip B3)
//   (an index hole on our side after a restart: f2b_z1/f2b_z2; no hole: f2b_z0)
// MATRIX: R1 O1-O4 · R0 O2-O4
//
//! covers: fork_recovery.rs fork_choice.rs doors.rs

use network::sync::CompletedRecovery;

use super::inc_i_288_f2c_common::*;

const OURS: [u32; 2] = [210, 211];

/// B1 (216), B2 (217) and B3 at the first slot from 218 whose hash is ABOVE our
/// tip A2: a lost hint makes B1..B3 tie A1, A2, and the tie must then keep A2, so
/// the hint is the only thing that can move the tip (no tie-break luck).
fn heavier_branch(r: &Rig) -> Vec<doli_core::Block> {
    let a2 = r.ours[1].hash();
    let b = chain_on(r, r.f_hash, F + 1, &[216, 217]);
    // B3 by the third producer: B has 3 distinct supporters (INC-I-289 K-1).
    let b3 = (218..418)
        .filter(|s| s % 3 == 2)
        .map(|s| block(F + 3, s, b[1].hash(), leader(&r.keys, s), &r.node.params))
        .find(|x| x.hash().as_bytes() > a2.as_bytes())
        .expect("fixture: a B3 hashing above A2 within 200 slots");
    vec![b[0].clone(), b[1].clone(), b3]
}

/// Restart, then the completed walk of B1..B3 connected at F.
async fn restart_then_walk() -> (Rig, anyhow::Result<()>) {
    let mut r = live_rig(&OURS, &[]).await;
    r.theirs = heavier_branch(&r);
    restart(&mut r).await;
    assert!(
        !knows(&r, &r.f_hash).await,
        "precondition: after the restart the reorg handler does not track F"
    );
    let walk = CompletedRecovery {
        blocks: r.theirs.clone(),
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    };
    let res = r.node.handle_completed_fork_recovery(walk).await;
    (r, res)
}

async fn assert_adopted_b3(r: &Rig, res: &anyhow::Result<()>, label: &str) {
    let b3 = r.theirs[2].clone();
    assert_on(r, &b3, F + 3, &format!("{label}: tip B3")).await;
    assert_eq!(
        tip(r).await.canonical_f1,
        Some(r.theirs[0].hash()),
        "{label}: B1 canonical at F+1"
    );
    assert!(res.is_ok(), "{label}: Ok — {res:?}");
}

// P3-630R3-009 INC-I-288 — Decision: a failure means the walk lost the connection point's slot hint, so after a restart the first walked block weighs 0 and a heavier walked branch is not adopted.
#[tokio::test]
async fn f2c_r1_inc_i_288_restart_walk_adopts_heavier_branch() {
    let (r, res) = restart_then_walk().await;
    let mut w = Vec::new();
    for b in &r.theirs {
        w.push(recorded_weight(&r, &b.hash()).await);
    }
    assert_eq!(
        w,
        vec![Some(1); 3],
        "F-2c R1: walked B1..B3 weights after a restart (B1's parent slot from the store)"
    );
    assert_adopted_b3(&r, &res, "F-2c R1").await;
}

// P3-630R3-009 INC-I-288 — Decision: a failure means the restart walk rule leaked below its activation height, so pre-AH adoption of a heavier walked branch changed without a gate.
#[tokio::test]
async fn f2c_r0_inc_i_288_below_ah_restart_walk_adopts_heavier_branch() {
    if !in_below_ah_child(
        "inc_i_288_f2c_restart_walk::f2c_r0_inc_i_288_below_ah_restart_walk_adopts_heavier_branch",
    ) {
        return;
    }
    let (r, res) = restart_then_walk().await;
    assert_adopted_b3(&r, &res, "F-2c R0 below AH").await;
}
