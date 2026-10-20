//! INC-I-248 REL-M1 — node-level quorum eligibility, the fixed snap-height vote, the
//! window-end decision and the snapshot-install reset.
//!
// covers: crates/core/src/maintainer/quorum.rs, crates/core/src/maintainer/retired.rs, crates/core/src/maintainer/mod.rs, crates/core/src/network_params/mod.rs, crates/core/src/network_params/defaults.rs, crates/core/src/network_params/env_loader.rs, crates/core/src/network_params/chainspec_loader.rs, crates/core/src/consensus/params.rs, crates/core/src/consensus/stress.rs, bins/node/src/node/maintainer_quorum.rs, bins/node/src/node/maintainer_sanitize.rs, bins/node/src/node/maintainer_probe_routing.rs, bins/node/src/node/mod.rs, bins/node/src/node/init.rs, bins/node/src/node/startup.rs, bins/node/src/node/periodic.rs, bins/node/src/node/fork_recovery.rs, bins/node/src/node/network_events.rs, bins/node/src/node/event_loop.rs, bins/node/src/node/maintainer_state_serve.rs, bins/node/src/node/state_session_serve.rs, bins/node/src/updater/mod.rs, bins/node/src/run.rs, bins/cli/src/cmd_snap.rs, bins/cli/src/cmd_snap_maintainer.rs, bins/cli/src/main.rs, bins/cli/src/lib.rs
//!
//! REQ-248-007 (Must) — Decision: a failure means peer answers still replace a usable or replayed root, an EMPTY@0 replay node adopts early, honest answers are dropped after a block, or a quorum-sized minority wins.
//!
// OUTPUT CONTRACT: Node::maintainer_quorum_tick_at(now) + handle_network_event(SyncResponse::MaintainerStateAt) + apply_maintainer_quorum_answers(answers) + apply_snap_snapshot(snapshot)
//   O1: none
//   O2: maintainer_state.set / last_derived_height; maintainer_probe_phase()
//   O3: apply_maintainer_quorum_answers -> QuorumVerdict; apply_snap_snapshot -> Ok
//   O4: maintainer_state.bin (bytes unchanged | absent | == memory)
//   O6: update recheck Notify (Adopt only)
// PATHS: P1 usable replay root + 5 colluding -> never armed, Keep(NotEligible);
//   P2 EMPTY@0 replay node + 7 honest + 3 forged -> never armed, Keep(NotEligible);
//   P3 EMPTY@S + 3 forged then 7 honest + block inside the window -> Adopt(honest) at window end;
//   P4 EMPTY@S + 3 forged + 3 honest -> Keep, retry-wait; P5 3 agreeing -> nothing before the
//   window end, Adopt at it; P6 install over a producer seed -> EMPTY@S, armed, then Adopt.
// INPUT PARTITIONS: local root = canonical usable | EMPTY@0 | EMPTY@S | producer seed@1;
//   answer order = forged first | honest only; tip at evaluation = S | S+1;
//   ticks = startup (+11 s) | mid-window (+10 s) | window end (+30 s).
// MATRIX: P1-P6 x (O2 set, O2 ldh, O2 phase, O4, O6) + O3 on P1/P2 = 32 cells.

use std::time::{Duration, Instant};

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m1_fixture as fx;
use doli_core::maintainer::{KeepReason, QuorumVerdict};

const REPLAY_TIP: u64 = 30_000;
const COLLUDER_TIP: u64 = 31_000;

// REQ-248-007 — Decision: a failure means 5 colluding peers with forged tips and last_updated replace a healthy replay node's canonical root (SEC-LOGIC-001).
#[tokio::test]
async fn inc_i_248_rel_m1_healthy_replay_root_ignores_colluding_quorum() {
    let canonical = fx::canonical(h::last_rotation_height());
    let (mut node, _t) =
        fx::node_with_state(fx::state(canonical.clone(), REPLAY_TIP), REPLAY_TIP).await;
    let notify = fx::attach_notify(&mut node);
    let before = fx::file_bytes(&node);
    fx::add_sync_peer(&node, REPLAY_TIP).await;
    let t0 = Instant::now();

    node.maintainer_quorum_tick_at(t0 + fx::STARTUP).await;
    assert_eq!(
        node.maintainer_probe_phase(),
        "idle",
        "a usable replay root never arms"
    );
    let forged = fx::frame(&node, &fx::attacker(1), COLLUDER_TIP);
    h::deliver(&mut node, &forged, 5).await;
    node.maintainer_quorum_tick_at(t0 + fx::WINDOW_END).await;

    assert_eq!(node.maintainer_probe_phase(), "idle");
    fx::assert_state(&node, &canonical, REPLAY_TIP, "after the window").await;
    assert_eq!(
        fx::file_bytes(&node),
        before,
        "maintainer_state.bin rewritten"
    );
    assert!(!fx::fired(&notify, fx::QUIET).await, "recheck fired");

    let answers = vec![fx::answer(&node, &fx::attacker(1), COLLUDER_TIP); 5];
    assert_eq!(
        node.apply_maintainer_quorum_answers(answers).await,
        QuorumVerdict::Keep(KeepReason::NotEligible)
    );
    fx::assert_state(&node, &canonical, REPLAY_TIP, "after the direct call").await;
    assert_eq!(fx::file_bytes(&node), before);
}

// REQ-248-004 (Must) — Decision: a failure means a replay node below the gate adopts a forged set before it seeds (QA-248-R01).
#[tokio::test]
async fn inc_i_248_rel_m1_fresh_replay_empty_at_zero_never_adopts() {
    let below_gate = h::GATE - 100;
    let (mut node, _t) = h::testnet_node(&h::producer_keys()).await;
    fx::set_tip(&node, below_gate).await;
    let notify = fx::attach_notify(&mut node);
    fx::add_sync_peer(&node, fx::S).await;
    let t0 = Instant::now();

    node.maintainer_quorum_tick_at(t0 + fx::STARTUP).await;
    assert_eq!(node.maintainer_probe_phase(), "idle", "EMPTY@0 never arms");
    let honest = fx::canonical(h::last_rotation_height());
    let (forged, good) = (
        fx::frame(&node, &fx::attacker(0), fx::S),
        fx::frame(&node, &honest, fx::S),
    );
    h::deliver(&mut node, &forged, 3).await;
    h::deliver(&mut node, &good, 7).await;
    node.maintainer_quorum_tick_at(t0 + fx::WINDOW_END).await;

    let st = h::state_of(&node).await;
    assert!(
        st.set.members.is_empty(),
        "EMPTY@0 adopted {} members",
        st.set.members.len()
    );
    assert_eq!(st.last_derived_height, 0);
    assert!(
        !fx::file_path(&node).exists(),
        "no maintainer_state.bin may be written"
    );
    assert_eq!(node.maintainer_probe_phase(), "idle");
    assert!(!fx::fired(&notify, fx::QUIET).await, "recheck fired");

    let mut answers = vec![fx::answer(&node, &fx::attacker(0), fx::S); 3];
    answers.extend(vec![fx::answer(&node, &honest, fx::S); 7]);
    assert_eq!(
        node.apply_maintainer_quorum_answers(answers).await,
        QuorumVerdict::Keep(KeepReason::NotEligible)
    );

    node.maybe_bootstrap_maintainer_set(h::GATE).await;
    assert!(
        h::is_producer_seed(&h::state_of(&node).await.set),
        "control: the replay node still seeds its producers at the gate"
    );
}

struct Joined {
    a: h::ReplayNode,
    view: h::PeerView,
    b: doli_node::node::Node,
    _tb: tempfile::TempDir,
}

async fn joined() -> Joined {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let (b, _tb) = h::snap_joined_node_b(h::snapshot_of(&a.node).await).await;
    Joined { a, view, b, _tb }
}

// REQ-248-007 — Decision: a failure means a snap node drops the honest answers after applying a block in the window, or the first 3 forged answers win (SEC-LOGIC-003).
#[tokio::test]
async fn inc_i_248_rel_m1_snap_node_adopts_honest_majority_after_block_in_window() {
    let mut j = joined().await;
    let notify = fx::attach_notify(&mut j.b);
    fx::assert_empty_at(&j.b, fx::S, "fixture: snap install").await;
    assert_eq!(j.b.maintainer_probe_phase(), "collecting");
    let honest = j.a.node.serve_maintainer_state().await;
    let forged = fx::frame(&j.b, &fx::attacker(0), fx::S);

    h::deliver(&mut j.b, &forged, 3).await;
    h::deliver(&mut j.b, &honest, 7).await;
    let applied = h::apply_next_block(&mut j.b, &j.a.producers).await;
    assert_eq!(applied, fx::S + 1, "fixture: one block inside the window");
    j.b.maintainer_quorum_tick_at(Instant::now() + fx::WINDOW_END)
        .await;

    fx::assert_state(&j.b, &j.view.set, fx::S + 1, "7 honest vs 3 forged").await;
    assert_eq!(j.b.maintainer_probe_phase(), "done");
    assert!(
        fx::fired(&notify, Duration::from_secs(1)).await,
        "Adopt must fire the recheck"
    );
    assert_eq!(
        h::verdict_name(&h::verify_rotated_release(&j.b).await),
        "Ok(3)"
    );
}

// REQ-248-007 — Decision: a failure means a tie between honest and forged answers installs either set.
#[tokio::test]
async fn inc_i_248_rel_m1_snap_node_tie_keeps_empty() {
    let mut j = joined().await;
    let notify = fx::attach_notify(&mut j.b);
    let honest = j.a.node.serve_maintainer_state().await;
    let forged = fx::frame(&j.b, &fx::attacker(0), fx::S);

    h::deliver(&mut j.b, &forged, 3).await;
    h::deliver(&mut j.b, &honest, 3).await;
    j.b.maintainer_quorum_tick_at(Instant::now() + fx::WINDOW_END)
        .await;

    fx::assert_empty_at(&j.b, fx::S, "3 honest vs 3 forged").await;
    assert_eq!(
        j.b.maintainer_probe_phase(),
        "retry-wait",
        "a tie is transient"
    );
    assert!(!fx::fired(&notify, fx::QUIET).await, "recheck fired");
}

// REQ-248-007 — Decision: a failure means the first quorum of answers decides before a dissenting peer can answer (SEC-LOGIC-004).
#[tokio::test]
async fn inc_i_248_rel_m1_decision_only_at_window_end() {
    let mut j = joined().await;
    let notify = fx::attach_notify(&mut j.b);
    let honest = j.a.node.serve_maintainer_state().await;

    h::deliver(&mut j.b, &honest, 3).await;
    fx::assert_empty_at(&j.b, fx::S, "3 agreeing answers before the window end").await;
    assert_eq!(j.b.maintainer_probe_phase(), "collecting");
    j.b.maintainer_quorum_tick_at(Instant::now() + Duration::from_secs(10))
        .await;
    fx::assert_empty_at(&j.b, fx::S, "mid-window tick").await;
    assert!(
        !fx::fired(&notify, fx::QUIET).await,
        "no recheck before the window end"
    );

    j.b.maintainer_quorum_tick_at(Instant::now() + fx::WINDOW_END)
        .await;
    fx::assert_state(&j.b, &j.view.set, fx::S, "window end").await;
    assert_eq!(j.b.maintainer_probe_phase(), "done");
    assert!(
        fx::fired(&notify, Duration::from_secs(1)).await,
        "Adopt must fire the recheck"
    );
}

// REQ-248-007 — Decision: a failure means a set the node held before the snapshot (a producer seed) survives the install and blocks the repair.
#[tokio::test]
async fn inc_i_248_rel_m1_snapshot_install_resets_pre_snapshot_set() {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let snapshot = h::snapshot_of(&a.node).await;
    let (mut b, _tb) = h::testnet_node(&h::producer_keys()).await;
    b.maybe_bootstrap_maintainer_set(1).await;
    let seeded = h::state_of(&b).await;
    assert!(
        h::is_producer_seed(&seeded.set),
        "fixture: producer seed at h=1"
    );
    assert_eq!(seeded.last_derived_height, 1, "fixture: seed stamped at 1");

    b.apply_snap_snapshot(snapshot)
        .await
        .expect("apply_snap_snapshot must succeed");
    fx::assert_empty_at(&b, fx::S, "right after the install").await;
    assert_eq!(
        b.maintainer_probe_phase(),
        "collecting",
        "the install arms the probe"
    );
    assert!(!h::resolved_root(&b).await.is_usable());

    h::deliver(&mut b, &a.node.serve_maintainer_state().await, 3).await;
    b.maintainer_quorum_tick_at(Instant::now() + fx::WINDOW_END)
        .await;
    fx::assert_state(&b, &view.set, fx::S, "after the window").await;
    assert_eq!(
        h::verdict_name(&h::verify_rotated_release(&b).await),
        "Ok(3)"
    );
}
