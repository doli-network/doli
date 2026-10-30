//! INC-I-288 E-2 below-AH lock: with `inc_i_288_fork_choice_activation_height` above
//! every test height, the old coordinator behavior is kept: lighter-branch orphans
//! still raise the network tip to best+1 and Rule 1 still rolls back the heavier
//! tip (E-2(a)), and the node's own block at h+1 still opens Gate 1 (E-2(b)).
//! `NetworkParams` is cached per process, so each body runs in a re-exec'd child
//! with the AH env var set (the C-1 below-AH pattern).
//!
//! OUTPUT CONTRACT: (a) fn Node::handle_new_block(orphan, peer) x2, then
//!   fn Node::run_periodic_tasks(); (b) fn SyncManager::classify_and_dispatch(n)
//!   after a coordinator rollback and an own-block apply at h+1
//!   O1: chain_state tip, canonical index, persisted tip, sync tip
//!   O2: Node rollback counters (shallow_rollback_count, cumulative_rollback_depth)
//!   O3: rollback memory: the old tip is a removed hash
//!   O4: SyncManager network_tip_height after the intake (= best+1)
//!   O5: return action of the dispatch (b)
//! PATHS (both below the AH):
//!   P1: heavier tip, 2 lighter-branch orphans, peer status <= local (E-2(a) P1a)
//!   P2: own block at h+1 after a coordinator rollback (E-2(b) P1)
//! INPUT PARTITIONS: one per path (the E-2 partitions that change behavior at the AH).
//! MATRIX: P1 O1-O4 · P2 O1 (precondition), O5 (O2-O4 N/A: (b) changes only O5)
//!
//! covers: recovery.rs peers.rs block_handling.rs block_lifecycle.rs rollback_memory.rs
//! covers: types.rs init.rs defaults.rs env_loader.rs

use doli_core::{Network, NetworkParams};
use network::RecoveryAction;

use super::inc_i_288_e2_common::*;

const AH_ENV: &str = "DOLI_INC_I_288_FORK_CHOICE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_288_E2_BELOW_AH_CHILD";
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

// E-2(a) INC-I-288 — Decision: a failure means the orphan-gap change leaked below its activation height, so pre-AH coordinator decisions changed without a gate.
#[tokio::test]
async fn e2_d_inc_i_288_below_ah_heavier_tip_still_rolls_back() {
    if !in_child("inc_i_288_e2_below_ah::e2_d_inc_i_288_below_ah_heavier_tip_still_rolls_back") {
        return;
    }
    let mut r = rig(&[210, 212, 214, 216], &[211, 213, 215]).await;
    let (a3, a4, b3) = (r.ours[2].clone(), r.ours[3].clone(), r.theirs[2].clone());
    let orphans = r.theirs[1..].to_vec();
    let o = heal_tick(&mut r, &[(F + 4, a4), (F + 3, b3)], &orphans).await;

    let (srb, depth) = o.rollbacks_before;
    assert_eq!(
        o.rollbacks_after,
        (srb + 1, depth + 1),
        "E-2 below AH: Rule 1 still rolls back the heavier tip — {o:?}"
    );
    assert_on(&r, &a3, F + 3, "E-2 below AH: walked down to A3").await;
    assert!(o.old_tip_removed, "E-2 below AH: A4 is a removed hash");
    assert_eq!(
        o.net_after_intake,
        F + 5,
        "E-2 below AH: orphan intake still sets the network tip to best+1"
    );
}

// E-2(b) INC-I-288 — Decision: a failure means the own-block Gate 1 change leaked below its activation height, so pre-AH post-rollback decisions changed without a gate.
#[tokio::test]
async fn e2_d_inc_i_288_below_ah_own_block_still_opens_gate1() {
    if !in_child("inc_i_288_e2_below_ah::e2_d_inc_i_288_below_ah_own_block_still_opens_gate1") {
        return;
    }
    let (mut r, peer) = after_coordinator_rollback().await;
    let own = block_on_a1(&r, true);
    apply_as_produced(&mut r, &own).await;
    assert_on(&r, &own, F + 2, "precondition: own block is the tip").await;
    empty_headers(&r, peer, 2).await;

    let (action, _) = dispatch(&mut r).await;

    assert_eq!(
        action,
        RecoveryAction::HeaderFirstSync,
        "E-2 below AH: the own block still opens Gate 1"
    );
}
