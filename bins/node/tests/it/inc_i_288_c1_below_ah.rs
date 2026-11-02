//! INC-I-288 C-1(d) below-AH lock: with `inc_i_288_fork_choice_activation_height`
//! above every test height, the old fork-choice behavior is kept bit-identical:
//! the depth-1 rollback still clears the ReorgHandler, and FORK_GUARD still drops a
//! better-slot sibling. `NetworkParams` is cached per process, so each body runs in
//! a re-exec'd child with the AH env var set.
//!
//! NOT DECLARED in `it/main.rs` at 6cd0aba6: it names the new param, so its red is a
//! compile error (no field `inc_i_288_fork_choice_activation_height`). The developer
//! adds `mod inc_i_288_c1_below_ah;` with the param. The env override must reach
//! BOTH gates: the node's `network.params()` (FORK_GUARD) and the SyncConfig that
//! `Node::new_for_test` builds (reset_sync_for_rollback).
//!
//! OUTPUT CONTRACT: fn Node::rollback_one_block(CoordinatorApproved { depth: 1 })
//!   -> SyncManager::reset_sync_for_rollback, then fn Node::handle_new_block(block, peer)
//!   O1: chain_state best_hash / best_height
//!   O2: block_store canonical entry at F+1
//!   O3: state_db persisted tip
//!   O4: sync_manager local_tip height
//!   O5: ReorgHandler memory after the rollback (current_weight, chain_weight(new tip))
//!   O6: weight-door record of the better-slot sibling (recorded, cached, retained)
//!   O7: stuck-fork signal raised by FORK_GUARD
//! PATHS (both below the AH):
//!   P3: depth-1 rollback, then a worse-slot sibling reaches the wedge door
//!   P4: better-slot sibling at an occupied height (FORK_GUARD is_better arm)
//! INPUT PARTITIONS:
//!   P3a: own F+5 vs one-block sibling F+1 -> clear ran (O5 = 0), sibling adopted
//!   P4a: better-slot branch F+3 vs ours F+2 -> B1 dropped unweighed, O7 raised, keep
//! MATRIX: P3a O1-O5 · P4a O1-O4,O6,O7
//!
//! covers: block_lifecycle.rs block_handling.rs defaults.rs env_loader.rs init.rs types.rs

use doli_core::{Network, NetworkParams};

use super::inc_i_288_c1_common::*;

const AH_ENV: &str = "DOLI_INC_I_288_FORK_CHOICE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_288_C1_BELOW_AH_CHILD";
const BELOW_AH: u64 = 1_000_000_000;

/// Parent: re-exec `name` alone with the AH override; returns true in the child.
fn in_child(name: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        assert_eq!(
            NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height,
            BELOW_AH,
            "child must pick up the AH env override"
        );
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(AH_ENV, BELOW_AH.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "below-AH child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

// C-1(d) INC-I-288 — Decision: a failure means the rollback-memory change leaked below its activation height, so fork choice on pre-AH heights changed without a gate.
#[tokio::test]
async fn c1_d_inc_i_288_below_ah_depth1_rollback_still_clears() {
    if !in_child("inc_i_288_c1_below_ah::c1_d_inc_i_288_below_ah_depth1_rollback_still_clears") {
        return;
    }
    // P3a: the C-1(a) P1a rig, below the AH.
    let mut r = rig(&[210, 211, 212, 213, 214, 215], &[216]).await;
    let (a5, s) = (r.ours[4].clone(), r.theirs[0].clone());
    assert_eq!(
        chain_weight(&r, &a5.hash()).await,
        F + 5,
        "precondition: A5 weighs F+5"
    );

    rollback_depth_one(&mut r).await;
    assert_on(
        &r,
        &a5,
        F + 5,
        "precondition: depth-1 rollback left A5 as tip",
    )
    .await;
    let (cur, w_a5) = (current_weight(&r).await, chain_weight(&r, &a5.hash()).await);

    gossip(&mut r, &s).await;
    let after = tip(&r).await;

    assert_eq!(
        (cur, w_a5),
        (0, 0),
        "C-1(d): below the AH the rollback still clears the ReorgHandler"
    );
    assert_on(
        &r,
        &s,
        F + 1,
        "C-1(d): below the AH the old weight-blind reorg is kept",
    )
    .await;
    assert_eq!(
        after.canonical_f1,
        Some(s.hash()),
        "C-1(d): S canonical at F+1"
    );
}

// C-1(d) INC-I-288 — Decision: a failure means the FORK_GUARD routing change leaked below its activation height, so a better-slot sibling is weighed on pre-AH heights without a gate.
#[tokio::test]
async fn c1_d_inc_i_288_below_ah_fork_guard_still_drops_better_slot() {
    if !in_child(
        "inc_i_288_c1_below_ah::c1_d_inc_i_288_below_ah_fork_guard_still_drops_better_slot",
    ) {
        return;
    }
    // P4a: the C-1(b) P2a rig, below the AH.
    let mut r = rig(&[210, 211], &[205, 213, 214]).await;
    let (a1, a2, b1) = (r.ours[0].clone(), r.ours[1].clone(), r.theirs[0].clone());
    let _ = stuck_fork_raised(&r).await;

    gossip(&mut r, &b1).await;
    let (door_b1, raised) = (weighed(&r, &b1).await, stuck_fork_raised(&r).await);
    let rest = r.theirs[1..].to_vec();
    for b in &rest {
        gossip(&mut r, b).await;
    }
    let after = tip(&r).await;

    assert_eq!(
        door_b1,
        (false, false, false),
        "C-1(d): below the AH FORK_GUARD drops B1 unweighed"
    );
    assert!(
        raised,
        "C-1(d): below the AH FORK_GUARD still signals stuck fork"
    );
    assert_on(
        &r,
        &a2,
        F + 2,
        "C-1(d): below the AH the better-slot branch is not adopted",
    )
    .await;
    assert_eq!(
        after.canonical_f1,
        Some(a1.hash()),
        "C-1(d): A1 stays canonical at F+1"
    );
}
