// INC-I-281 M2.4 — SEC-FIXVERIFY-002 (P2, R-3a Must): inside the INC-I-010 layer-3 Light window
// (snap_sync_height set, or floor_fallback_window) execute_reorg skips its schedule check, so the
// door must judge a walked block after a branch-only boundary with the node's current list.
//
// covers: block_handling reorg_schedule reorg_restore fork_recovery fork_walk wedge_escape force_reorg door_epoch validation_checks mod init network_events rollback_memory fork_cache
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (post-genesis) with the Light window armed
//   handles a heavier walked branch [C_B, X_{B+1}, X_{B+2}] connected at F.
// Functions: Node::handle_completed_fork_recovery (walk door -> check_producer_eligibility_at
//   -> door_judges_schedule).
// OUTPUTS:
//   O1 return value (Ok: a door refusal is not an Err)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at B, B+1, B+2; body presence of X_{B+1}
//   O4 state_db persisted chain-state tip
//   O5 reorg-handler weight of X_{B+1} (recorded only past the walk door)
//   O6 window flags (snap_sync_height, floor_fallback_window) unchanged
//   O7 logs: no rollback / fork-apply / restore line
// PATHS: walk door round-robin arm x window {snap_sync_height, floor_fallback_window}
// INPUT PARTITIONS: X_{B+1} by a registered producer that is neither L_new(D_B)[s1] (the node's
//   current list) nor L_new(C_B)[s1]; C_B = L_close[slot] (door admits it); X_{B+2} scheduled.
// MATRIX: walk x {snap, floor}: O1-O7.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use crypto::PublicKey;
use inc_i_236_m2_reorg_support::weight_recorded;
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::Logs;

const ROLLBACK_LINES: [&str; 5] = [
    "Rolling back from height",
    "Undo-based rollback",
    "new blocks from fork",
    "is not scheduled at its parent",
    "[REORG_RESTORE]",
];

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE lightwindow={cell} wrong={}", wrong as u8);
}

/// A producer of L_new(C_B) scheduled at s1 under neither list.
fn unscheduled(fx: &Fx) -> PublicKey {
    let (cur, branch) = (at(&fx.l_dead, fx.s1), at(&fx.l_canon, fx.s1));
    *fx.l_canon
        .iter()
        .find(|pk| **pk != cur && **pk != branch)
        .expect("fixture: a third producer")
}

/// Runs the walk with the window armed; returns the outputs that show X_{B+1} passed the door.
async fn walk_in_window(snap: bool) -> Vec<String> {
    let mut fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    if snap {
        fx.node.snap_sync_height = Some(F + fx.o);
    } else {
        fx.node.floor_fallback_window = true;
    }
    let x = unscheduled(&fx);
    assert!(
        fx.node
            .producer_set
            .read()
            .await
            .get_by_pubkey(&x)
            .is_some(),
        "precondition: X_B+1's producer is registered (membership arm passes)"
    );
    let br = fx.after_c_b(fx.key(&x));
    let before = fx.state().await;
    let flags = (fx.node.snap_sync_height, fx.node.floor_fallback_window);

    let logs = Logs::default();
    let result = {
        let _g = tracing::subscriber::set_default(logs.subscriber());
        let f = fx.f_hash;
        run_walk(&mut fx, &br, f).await
    };
    let mut wrong = Vec::new();
    if result.is_err() {
        wrong.push(format!("result {result:?}"));
    }
    if weight_recorded(&fx.node, &br[1].hash()).await {
        wrong.push("X_B+1 weight recorded (passed the door)".to_string());
    }
    let st = fx.state().await;
    if st != before {
        wrong.push(format!("tip moved: {st:?}"));
    }
    if fx.applied(&br[1]) {
        wrong.push("X_B+1 applied".to_string());
    }
    let now = (fx.node.snap_sync_height, fx.node.floor_fallback_window);
    if now != flags {
        wrong.push(format!("window flags {flags:?} -> {now:?}"));
    }
    for l in ROLLBACK_LINES {
        let n = logs.mentioning(l);
        if n > 0 {
            wrong.push(format!("{n} x `{l}`"));
        }
    }
    wrong
}

// R-3a / SEC-FIXVERIFY-002 (Must) — Decision: a failure means a node in the post-snap Light window lets an unscheduled post-boundary block past the door, and execute_reorg skips its check there, so the node rolls back for (or adopts) an unscheduled branch.
#[tokio::test]
async fn light_window_walk_door_refuses_unscheduled_post_boundary_block() {
    let wrong = walk_in_window(true).await;
    probe("walk", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "snap_sync_height window: the walk door did not refuse X_B+1: {wrong:?}"
    );
}

// R-3a / SEC-FIXVERIFY-002 (Should) — Decision: a failure means the floor-fallback Light window lets an unscheduled post-boundary block past the door, with no schedule check left in execute_reorg.
#[tokio::test]
async fn floor_window_walk_door_refuses_unscheduled_post_boundary_block() {
    let wrong = walk_in_window(false).await;
    probe("walk_floor", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "floor_fallback_window: the walk door did not refuse X_B+1: {wrong:?}"
    );
}
