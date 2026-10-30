//! INC-I-288 F-2c (W) SEC-INTEGRITY-R3-001: at/above the AH the wedge door now
//! caches and retains a same-slot child at weight 0 (`wedge_escape.rs`), and the
//! fork walk seeds from the DEEPEST cached chain and gives up when that seed's
//! branch tip is rejected (`fork_walk.rs` `select_walk_seed`,
//! `try_trigger_fork_recovery`). One slot leader's same-slot burst D1..D6 on F is
//! then the deepest cached chain; its walk is "not heavier" and is marked, and the
//! real heavier branch B1..B4 (only its top B4 cached, as a gossip orphan) is never
//! walked. Live rig (M5 live), production weights (1 per producer).
//!
// OUTPUT CONTRACT:
//   fn Node::try_trigger_fork_recovery(&mut self) -> bool
//   fn Node::handle_completed_fork_recovery(CompletedRecovery) -> Result<()> (via the walk poll)
//   O1: chain_state best_hash / best_height (assert_on)
//   O2: block_store canonical entries at F+1 and at the tip height
//   O3: state_db persisted tip
//   O4: sync_manager local tip height
//   O5: sync_manager walk state (`is_fork_recovery_active`, `fork_recovery_current_parent`)
//   O6: return value of try_trigger_fork_recovery (a walk started)
// PATHS: P1 walk seeded, driven to its connection point, judged by fork choice;
//        P2 a later trigger after the 30 s RECOVERY_COOLDOWN (no injection seam)
// INPUT PARTITIONS:
//   W1 (P1+P2): at/above the AH, cache = same-slot burst D1..D6 (deepest, D2..D6 at 0)
//        + B4 (orphan of the heavier B1..B4) -> within two triggers the node walks
//        B4 and adopts B1..B4 (tip B4 at F+4)
//   W0 BELOW-AH (P1): burst D1..D3 at full weight + B4 -> the first walk still
//        seeds the deepest cached chain (fetch = D1): the seed rule is unchanged
//   CONTROL W2 (P1): at/above the AH, no burst, B4 alone -> one walk adopts B1..B4
//        (the W1 red is the burst's, not the fixture's)
// MATRIX: W1 O1-O6 · W2 O1-O2,O5-O6 · W0 O5,O6
//
//! covers: fork_walk.rs wedge_escape.rs fork_cache.rs fork_choice.rs mod.rs fork_recovery.rs doors.rs

use doli_core::Block;

use super::inc_i_288_e2_common::{cached, deliver};
use super::inc_i_288_f2c_common::*;

const OURS: [u32; 3] = [210, 211, 212];

struct Shape {
    burst: Vec<Block>,
    honest: Vec<Block>,
}

/// The burst D1..Dn on F (slot S, one leader, through the gossip door), then the
/// heavier honest B1..B4 on F (slots after S); only B4 arrives, as an orphan.
async fn stuck_with_burst_and_heavier_orphan(r: &mut Rig, n: u64) -> Shape {
    let burst = same_slot_burst(r, 213, r.f_hash, F + 1, n);
    let s = burst[0].header.slot;
    let honest = chain_on(r, r.f_hash, F + 1, &[s + 1, s + 2, s + 3, s + 4]);
    for d in &burst {
        gossip(r, d).await;
    }
    for (i, d) in burst.iter().enumerate() {
        assert_eq!(
            weighed(r, d).await,
            (true, true, true),
            "precondition: the door recorded, cached and retained D{}",
            i + 1
        );
    }
    let b4 = honest[3].clone();
    let peer = walk_peer(r, F + 4, &b4).await;
    deliver(r, &b4, peer).await;
    assert!(
        cached(r, &b4.hash()).await && !stored(r, &b4),
        "precondition: B4 is a cached orphan (parent B3 unknown)"
    );
    Shape { burst, honest }
}

/// One trigger; when a walk starts, drive it to its connection point.
async fn trigger_and_walk(r: &mut Rig, serve: &[Block]) -> (bool, Vec<crypto::Hash>) {
    let started = r.node.try_trigger_fork_recovery().await;
    let fetched = if started {
        drive_walk(r, serve, 8).await
    } else {
        Vec::new()
    };
    (started, fetched)
}

// SEC-INTEGRITY-R3-001 INC-I-288 — Decision: a failure means one slot leader's same-slot burst, cached at weight 0, takes the walk seed and blocks every later walk, so a wedged node never reaches the heavier branch (INV-FORK-009).
#[tokio::test]
async fn f2c_w1_inc_i_288_same_slot_burst_does_not_block_walk_to_heavier_branch() {
    let mut r = live_rig(&OURS, &[]).await;
    let sh = stuck_with_burst_and_heavier_orphan(&mut r, 6).await;
    for (i, d) in sh.burst.iter().enumerate().skip(1) {
        assert_eq!(
            recorded_weight(&r, &d.hash()).await,
            Some(0),
            "precondition: same-slot D{} is recorded at weight 0",
            i + 1
        );
    }
    let serve: Vec<Block> = sh.burst.iter().chain(&sh.honest).cloned().collect();
    let b4 = sh.honest[3].clone();

    let mut log = Vec::new();
    for round in 0..2 {
        if round == 1 {
            // RECOVERY_COOLDOWN (30 s) follows every completed walk; no seam.
            tokio::time::sleep(std::time::Duration::from_secs(31)).await;
        }
        let step = trigger_and_walk(&mut r, &serve).await;
        log.push(step);
        if tip(&r).await.best_hash == b4.hash() {
            break;
        }
    }

    let walked_b = log.iter().any(|(_, f)| f.contains(&sh.honest[2].hash()));
    assert!(
        walked_b,
        "F-2c W1: no walk fetched B3 (the heavier branch was never seeded); \
         triggers (started, fetched) = {log:?}, D1 = {:?}, B3 = {:?}",
        sh.burst[0].hash(),
        sh.honest[2].hash()
    );
    assert_on(
        &r,
        &b4,
        F + 4,
        "F-2c W1: the node adopts the heavier branch B1..B4",
    )
    .await;
    assert_eq!(
        tip(&r).await.canonical_f1,
        Some(sh.honest[0].hash()),
        "F-2c W1: B1 is canonical at F+1"
    );
    assert!(
        !r.node.sync_manager.read().await.is_fork_recovery_active(),
        "F-2c W1: the walk completed"
    );
}

// SEC-INTEGRITY-R3-001 INC-I-288 — Decision: a failure means this fixture cannot walk to and adopt B1..B4 at all, so the W1 red would not be the burst's.
#[tokio::test]
async fn f2c_w2_inc_i_288_control_without_burst_walk_adopts_heavier_branch() {
    let mut r = live_rig(&OURS, &[]).await;
    let s = same_slot_burst(&r, 213, r.f_hash, F + 1, 1)[0].header.slot;
    let honest = chain_on(&r, r.f_hash, F + 1, &[s + 1, s + 2, s + 3, s + 4]);
    let b4 = honest[3].clone();
    let peer = walk_peer(&r, F + 4, &b4).await;
    deliver(&mut r, &b4, peer).await;

    let (started, fetched) = trigger_and_walk(&mut r, &honest).await;

    assert!(
        started && fetched.contains(&honest[2].hash()),
        "F-2c W2 control: the walk seeds B4 — (started, fetched) = ({started}, {fetched:?})"
    );
    assert_on(&r, &b4, F + 4, "F-2c W2 control: tip B4").await;
    assert_eq!(
        tip(&r).await.canonical_f1,
        Some(honest[0].hash()),
        "F-2c W2 control: B1 canonical at F+1"
    );
}

// SEC-INTEGRITY-R3-001 INC-I-288 — Decision: a failure means the walk-seed change leaked below its activation height, so pre-AH seed choice (deepest cached chain first) changed without a gate.
#[tokio::test]
async fn f2c_w0_inc_i_288_below_ah_first_walk_seeds_deepest_chain() {
    if !in_below_ah_child(
        "inc_i_288_f2c_walk_seed::f2c_w0_inc_i_288_below_ah_first_walk_seeds_deepest_chain",
    ) {
        return;
    }
    let mut r = live_rig(&OURS, &[]).await;
    let sh = stuck_with_burst_and_heavier_orphan(&mut r, 3).await;
    assert_on(
        &r,
        &r.ours[2].clone(),
        F + 3,
        "precondition below AH: the burst (tie at 3) did not move the tip",
    )
    .await;

    let started = r.node.try_trigger_fork_recovery().await;

    let sm = r.node.sync_manager.read().await;
    assert!(
        started && sm.is_fork_recovery_active(),
        "F-2c W0 below AH: a walk starts"
    );
    assert_eq!(
        sm.fork_recovery_current_parent(),
        Some(sh.burst[0].hash()),
        "F-2c W0 below AH: the first walk seeds D2 (deepest cached chain, first fetch D1)"
    );
}
