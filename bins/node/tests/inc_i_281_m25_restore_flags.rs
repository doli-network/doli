// INC-I-281 M2.5 — SEC-STATE-001 / REV-281-REL-F1 (P2 Must): a refused reorg's restore leaves the
// Light-window flags as the refused branch's boundary block set them.
//
// covers: reorg_restore block_handling reorg_schedule door_epoch validation_checks mod init wedge_escape fork_walk force_reorg fork_recovery post_commit floor_window fork_cache reorg_refusals
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (post-genesis) with the Light window armed walks
//   [C_B, C'_{B+1}, C'_{B+2}]; the window door admits C'_{B+1} = L_new(D_B)[s1] under the current
//   list, C_B closes the window in post_commit, the in-reorg check refuses C'_{B+1}, restore runs.
// Functions: Node::handle_completed_fork_recovery -> execute_reorg -> restore_tip.
// OUTPUTS:
//   O1 return value (Ok)
//   O2 chain_state best_hash / best_height; O3 canonical hash at B..B+2; O4 persisted tip
//   O5 snap_sync_height; O6 floor_fallback_window; O7 floor_fallback_boundaries
// PATHS: window = snap_sync_height Some(F) | floor_fallback_window with 1 boundary held
// INPUT PARTITIONS: C_B's boundary closes the window (post_commit clears snap / Converged);
//   C'_{B+1} scheduled under L_new(D_B) only.
// MATRIX: snap O1-O5 (+ O6 O7 unchanged) | floor O1-O4 O6 O7 (+ O5 unchanged).

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::Logs;

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE restoreflags={cell} wrong={}", wrong as u8);
}

type Flags = (Option<u64>, bool, u8);

fn flags(fx: &Fx) -> Flags {
    (
        fx.node.snap_sync_height,
        fx.node.floor_fallback_window,
        fx.node.floor_fallback_boundaries,
    )
}

/// Walks the refused branch with the window armed by `arm`; returns what a full restore misses.
async fn refused_in_window(arm: fn(&mut Fx)) -> Vec<String> {
    let mut fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    arm(&mut fx);
    let br = fx.ineligible();
    let before = fx.state().await;
    let entry = flags(&fx);
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
    let st = fx.state().await;
    if st != before {
        wrong.push(format!("tip not restored: {st:?}"));
    }
    let now = flags(&fx);
    if now != entry {
        wrong.push(format!(
            "window flags (snap, floor, boundaries) {entry:?} -> {now:?} (refusals {}, restores {})",
            logs.mentioning("is not scheduled at its parent"),
            logs.mentioning("[REORG_RESTORE] restored")
        ));
    }
    wrong
}

// P2 / SEC-STATE-001 (b) (Must) — Decision: a failure means a node restored to its prior tip after a refused reorg has lost its post-snap Light window (snap_sync_height None), so it runs Full validation and rejects valid gossip on eligibility.
#[tokio::test]
async fn restore_in_snap_window_keeps_snap_sync_height() {
    let wrong = refused_in_window(|fx| fx.node.snap_sync_height = Some(F + fx.o)).await;
    probe("snap", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "restore inside the snap window: {wrong:?}"
    );
}

// P2 / SEC-STATE-001 (a) (Must) — Decision: a failure means a restore leaves the floor-fallback window closed or its boundary count reset, so the node drops to Full validation against the list the window exists to bypass.
#[tokio::test]
async fn restore_in_floor_window_keeps_floor_flags() {
    let wrong = refused_in_window(|fx| {
        fx.node.floor_fallback_window = true;
        fx.node.floor_fallback_boundaries = 1;
    })
    .await;
    probe("floor", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "restore inside the floor window: {wrong:?}"
    );
}
