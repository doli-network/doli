// INC-I-281 M2.2 — R-2 (Must), root cause RC-A: a re-apply of a block removed by a
// fork-remedy rollback counts as peer-confirmed progress for two 300 s suppressors,
// so a forked node is classified BEHIND and never reaches the rollback rung.
//
// OUTPUT CONTRACT: the post-rollback progress test of the two suppressors.
// Functions: SyncManager::note_orphan_gossip_block "BEHIND not forked" arm, and
//   RecoveryCoordinator::classify Gate 1 via SyncManager::classify_and_dispatch;
//   both driven by Node::handle_new_block after rollback_one_block
//   (CoordinatorApproved, the coordinator's ShallowRollback).
// OUTPUTS:
//   O1 count of "BEHIND not forked" events (the line R-2 says must never fire with
//      peer_confirmed=true for a block on the rolled-back branch)
//   O2 RecoveryAction returned by classify_and_dispatch
// PATHS: BEHIND arm (peers) | coordinator Gate 1 (recovery)
// INPUT PARTITIONS: the one apply since the rollback is
//   {a removed hash D1, re-adopted after the hold window with no sibling known,
//    a never-removed peer block C1 (control: real progress)}
// MATRIX:
//   BEHIND x D1 -> O1 = 0 (test 4)         BEHIND x C1 -> O1 >= 1 (test 4 control)
//   Gate1  x D1 -> O2 = ShallowRollback{1}, never HeaderFirstSync (test 5)
//   Gate1  x C1 -> O2 = HeaderFirstSync (test 5 control)
// refs: R-2, INC-I-281; INV-SYNC-010 is narrowed, never widened.
// covers: block_handling fork_cache rollback rollback_memory block_lifecycle peers recovery sync_manager periodic network_events
// covers: bins/node/src/node/block_handling.rs
// covers: bins/node/src/node/rollback.rs
// covers: crates/network/src/sync/manager/rollback_memory.rs
// covers: crates/network/src/sync/manager/block_lifecycle.rs
// covers: crates/network/src/sync/manager/peers.rs
// covers: crates/network/src/sync/manager/recovery.rs
// covers: crates/network/src/sync/manager/types.rs
// covers: crates/network/src/sync/manager/mod.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_m2_support;

use doli_core::Block;
use inc_i_281_m2_support::*;
use network::{RecoveryAction, RecoveryContext};

/// Rollback to F, then the one apply since it: the removed D1 after the hold
/// window (no sibling known), or the never-removed canonical C1.
async fn rolled_back_then_applied(removed: bool) -> Rig {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    let b = if removed {
        tokio::time::sleep(past_window(&r)).await;
        r.dead[0].clone()
    } else {
        r.canon[0].clone()
    };
    gossip(&mut r, &b).await;
    assert_on(
        &r,
        &b,
        F + 1,
        "precondition: the one apply since the rollback",
    )
    .await;
    r
}

/// Gossips orphans (parents unknown) and counts "BEHIND not forked" events.
async fn behind_events(r: &mut Rig, orphans: &[Block]) -> usize {
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(logs.subscriber());
    for o in orphans {
        gossip(r, o).await;
    }
    logs.mentioning("BEHIND not forked")
}

/// Gossips orphans as coordinator evidence, then asks the coordinator.
async fn coordinator_action(
    r: &mut Rig,
    orphans: &[Block],
) -> (RecoveryAction, Option<RecoveryContext>) {
    for o in orphans {
        gossip(r, o).await;
    }
    let count = r.node.shallow_rollback_count;
    r.node
        .sync_manager
        .write()
        .await
        .classify_and_dispatch(count)
}

// R-2 (Must) — Decision: a failure means re-applying a block the fork-remedy rollback removed still arms the "BEHIND not forked" suppressor, so orphan evidence from the canonical branch is discarded for 300 s on a node that is forked, not behind.
#[tokio::test]
async fn inc_i_281_removed_block_reapply_is_not_peer_progress() {
    let mut r = rolled_back_then_applied(true).await;
    let orphans = r.canon[1..4].to_vec();
    let dead_events = behind_events(&mut r, &orphans).await;

    let mut c = rolled_back_then_applied(false).await;
    let orphans = c.canon[2..5].to_vec();
    let control_events = behind_events(&mut c, &orphans).await;

    probe_suppressor("behind", dead_events > 0);
    assert!(
        control_events > 0,
        "control: after a never-removed peer block the BEHIND arm must still fire"
    );
    assert_eq!(
        dead_events, 0,
        "the re-apply of a removed dead block was counted as peer-confirmed progress"
    );
}

// R-2 (Must) — Decision: a failure means re-applying a removed dead block still opens coordinator Gate 1, which returns HeaderFirstSync ahead of the rollback rung for 300 s on a forked node.
#[tokio::test]
async fn inc_i_281_removed_block_reapply_does_not_open_gate1() {
    let mut r = rolled_back_then_applied(true).await;
    let orphans = r.canon[1..3].to_vec();
    let (dead_action, dead_ctx) = coordinator_action(&mut r, &orphans).await;

    let mut c = rolled_back_then_applied(false).await;
    let orphans = c.canon[2..4].to_vec();
    let (control_action, control_ctx) = coordinator_action(&mut c, &orphans).await;

    probe_suppressor("gate1", dead_action == RecoveryAction::HeaderFirstSync);
    assert_eq!(
        control_action,
        RecoveryAction::HeaderFirstSync,
        "control: Gate 1 must still hold after a never-removed peer block ({control_ctx:?})"
    );
    assert_ne!(
        dead_action,
        RecoveryAction::HeaderFirstSync,
        "Gate 1 counted the re-apply of a removed dead block as progress ({dead_ctx:?})"
    );
    assert_eq!(
        dead_action,
        RecoveryAction::ShallowRollback { depth: 1 },
        "without Gate 1 the orphan evidence reaches the rollback rung ({dead_ctx:?})"
    );
}
