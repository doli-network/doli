//! INC-I-248 REL-M2 — start-time reset of a maintainer root not derived from replayed history.
//!
// covers: bins/node/src/node/maintainer_sanitize.rs, bins/node/src/run.rs (start-time sanitize)
//!
// OUTPUT CONTRACT: Node::sanitize_retired_maintainer_set(&self) -> bool (async, start-time entry)
//   O1: none
//   O2: maintainer_state.set / last_derived_height; REL-M1 eligibility; maintainer_probe_phase()
//   O3: true iff it reset
//   O4: maintainer_state.bin (EMPTY@tip on reset, bytes unchanged otherwise)
//   O6: update recheck Notify (on the later quorum Adopt only)
// PATHS: R1 non-empty, D >= 1, tip > D, block D present, D+1 absent -> EMPTY@tip, eligible,
//   startup probe arms, 3 honest answers -> Adopt(canonical) at the window end;
//   K1 D and D+1 pruned (pruned replay node); K2 D present, D+1 present (hole below D);
//   K3 tip == D; K4 D == 0 (block 0 present, block 1 absent); K5 EMPTY root with the hole;
//   K6 no trust root attached (O3 false, O4 no file written).
// INPUT PARTITIONS: D = 0 | 1 | 48 | 49 | 50 | 100 | 1_000 | S+1 | TIP;
//   store = {1} + S+1..=TIP | 0..=100 + S+1..=TIP | 0 + 50..=2_050 (pruned at 50);
//   set = producer seed (real seed path) | producer keys stamped at D | EMPTY.
// MATRIX: R1 x 2 partitions x (O2 set, O2 ldh, O2 eligible, O3, O4) + R1 probe phase, digest,
//   last_change_block, release, O6 + K1-K5 x 10 cases x (O2 set, O2 ldh, O3, O4 bytes, phase).

use std::time::{Duration, Instant};

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m1_fixture as fx;
use crate::inc_i_248_rel_m2_fixture as m2;
use doli_core::MaintainerSet;

// REQ-248-008 (Must) — Decision: a failure means a pre-fix snap node keeps its stale producer-seed root after the upgrade and never converges to the fleet set (live drill gap).
#[tokio::test]
async fn inc_i_248_unreplayed_root_repaired_on_start() {
    let (mut node, _t) = m2::stale_snap_node().await;
    let notify = fx::attach_notify(&mut node);
    assert!(
        !m2::eligible(&node).await,
        "fixture: the stale seed is usable"
    );

    assert!(
        node.sanitize_retired_maintainer_set().await,
        "the start-time check must reset a root whose history after D was skipped"
    );
    fx::assert_empty_at(&node, m2::TIP, "after the start-time check").await;
    assert!(
        m2::eligible(&node).await,
        "EMPTY@tip must be quorum-eligible"
    );
    assert!(!h::resolved_root(&node).await.is_usable());
    assert!(
        !node.sanitize_retired_maintainer_set().await,
        "second call is a no-op"
    );
    fx::assert_empty_at(&node, m2::TIP, "after the second call").await;

    fx::add_sync_peer(&node, m2::TIP).await;
    let t0 = Instant::now();
    node.maintainer_quorum_tick_at(t0 + fx::STARTUP).await;
    assert_eq!(
        node.maintainer_probe_phase(),
        "collecting",
        "the startup probe arms"
    );
    let canonical = fx::canonical(h::last_rotation_height());
    let good = fx::frame(&node, &canonical, m2::TIP);
    h::deliver(&mut node, &good, 3).await;
    fx::assert_empty_at(&node, m2::TIP, "before the window end").await;
    node.maintainer_quorum_tick_at(t0 + fx::WINDOW_END).await;

    fx::assert_state(&node, &canonical, m2::TIP, "window end").await;
    assert_eq!(node.maintainer_probe_phase(), "done");
    let st = h::state_of(&node).await;
    assert_eq!(
        h::node_digest(&node, &st.set),
        h::node_digest(&node, &canonical),
        "resolved digest == canonical digest"
    );
    assert_eq!(
        st.set.last_updated, canonical.last_updated,
        "last_change_block == canonical"
    );
    assert_eq!(
        h::verdict_name(&h::verify_rotated_release(&node).await),
        "Ok(3)"
    );
    assert!(
        fx::fired(&notify, Duration::from_secs(1)).await,
        "Adopt must fire the recheck"
    );
}

// REQ-248-008 (Must) — Decision: a failure means the check keys on a fixed height or the first gap instead of D, so a node that applied blocks up to D before its snapshot keeps a stale root.
#[tokio::test]
async fn inc_i_248_rel_m2_root_reset_when_history_after_d_skipped() {
    const D: u64 = 100;
    let (node, _t) = m2::node_with(&fx::state(m2::producer_set_at(D), D)).await;
    m2::put_heights(&node, &[0..=D, m2::S + 1..=m2::TIP]);
    m2::set_tip_to_store(&node, m2::TIP).await;
    assert!(
        m2::has_block(&node, D) && !m2::has_block(&node, D + 1),
        "fixture: block D present, D+1 absent"
    );

    assert!(
        node.sanitize_retired_maintainer_set().await,
        "D = 100 with D+1 absent must reset"
    );
    fx::assert_empty_at(&node, m2::TIP, "D = 100").await;
    assert!(
        m2::eligible(&node).await,
        "EMPTY@tip must be quorum-eligible"
    );
}

// REQ-248-004 (Must) — Decision: a failure means a pruned replay node loses its replayed root to the peer quorum (the release-gate P1 reopened through pruning).
#[tokio::test]
async fn inc_i_248_rel_m2_pruned_replay_node_keeps_root() {
    const CUTOFF: u64 = 50;
    const PRUNED_TIP: u64 = CUTOFF + 2_000;
    let (mut node, _t) = m2::node_with(&fx::state(m2::producer_set_at(1), 1)).await;
    m2::put_heights(&node, std::slice::from_ref(&(0..=PRUNED_TIP)));
    let pruned = node
        .block_store
        .prune_blocks_below(CUTOFF, PRUNED_TIP)
        .expect("fixture: prune_blocks_below");
    assert_eq!(pruned, (CUTOFF - 1, CUTOFF), "fixture: pruned 1..CUTOFF");
    m2::set_tip_to_store(&node, PRUNED_TIP).await;
    assert!(
        m2::has_block(&node, 0)
            && !m2::has_block(&node, 1)
            && !m2::has_block(&node, CUTOFF - 1)
            && m2::has_block(&node, CUTOFF)
            && m2::has_block(&node, PRUNED_TIP),
        "fixture: store = genesis + CUTOFF..=tip"
    );

    let cases: [(&str, u64); 6] = [
        ("D = 0, block 0 present, block 1 pruned", 0),
        ("D = 1, D and D+1 pruned", 1),
        ("D = cutoff - 2, D and D+1 pruned", CUTOFF - 2),
        ("D = cutoff - 1, D pruned, D+1 present", CUTOFF - 1),
        ("D = cutoff, D and D+1 present", CUTOFF),
        ("D = 1_000, D and D+1 present", 1_000),
    ];
    for (ctx, d) in cases {
        m2::install_state(&mut node, &fx::state(m2::producer_set_at(d), d));
        m2::assert_start_keeps(&node, ctx).await;
    }
}

// REQ-248-008 (Must) — Decision: a failure means the start-time check also resets a fresh adoption or doli snap write (tip == D), a root above the hole, or touches an EMPTY root.
#[tokio::test]
async fn inc_i_248_rel_m2_snap_store_guards_keep_root() {
    let (mut node, _t) = m2::stale_snap_node().await;
    let cases: [(&str, MaintainerSet, u64); 4] = [
        (
            "tip == D, block D present, D+1 absent",
            m2::producer_set_at(m2::TIP),
            m2::TIP,
        ),
        (
            "canonical adopted at the tip",
            fx::canonical(h::last_rotation_height()),
            m2::TIP,
        ),
        (
            "D = S+1 above the hole, D+1 present, last_updated below D",
            m2::producer_set_at(1),
            m2::S + 1,
        ),
        ("EMPTY@1 with the hole", MaintainerSet::new(), 1),
    ];
    for (ctx, set, d) in cases {
        m2::install_state(&mut node, &fx::state(set, d));
        m2::assert_start_keeps(&node, ctx).await;
    }
}

// REQ-248-008 (Must) — Decision: a failure means the start-time check writes a trust root on a node that holds none.
#[tokio::test]
async fn inc_i_248_rel_m2_no_trust_root_is_noop() {
    let temp = tempfile::TempDir::new().unwrap();
    let node = doli_node::node::Node::new_for_test(temp.path().to_path_buf(), h::producer_keys())
        .await
        .expect("fixture: Node::new_for_test");
    assert!(node.maintainer_state.is_none(), "fixture: no trust root");
    m2::put_heights(&node, &[1..=1, m2::S + 1..=m2::TIP]);
    m2::set_tip_to_store(&node, m2::TIP).await;

    assert!(
        !node.sanitize_retired_maintainer_set().await,
        "no trust root: nothing to reset"
    );
    assert!(node.maintainer_state.is_none(), "no trust root attached");
    assert_eq!(fx::file_bytes(&node), None, "maintainer_state.bin written");
}
