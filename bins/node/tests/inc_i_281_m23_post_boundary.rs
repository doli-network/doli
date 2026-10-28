// INC-I-281 M2.3 — P1 (R-3a residual of RC-B, Must): the list that judges a block after a
// boundary block depends on that boundary block. A walked branch whose boundary block C_B
// is not in the node's chain rotates to L_new(C_B), which the node cannot derive before
// applying C_B; the doors judge C_{B+1} with the node's L_new(D_B) and Light-mode
// execute_reorg never checks the schedule at all.
//
// OUTPUT CONTRACT: recovery of a node on [.., F, D_B, D_{B+1}] onto a branch on F or D_B.
// Functions: Node::handle_completed_fork_recovery (walk door, fork choice, execute_reorg),
//   Node::handle_new_block -> retain_sibling_and_try_escape (wedge door),
//   Node::execute_reorg (per-block schedule check at the block's parent, INC-I-010 layer-3
//   Light gating); shared gate check_producer_eligibility_at -> door_epoch_list.
// OUTPUTS:
//   O1 return value (Ok on the walk / wedge paths; a door refusal is not an Err)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at B, B+1, B+2; body presence of a refused block
//   O4 state_db persisted chain-state tip
//   O5 reorg-handler weight of walked blocks (recorded only past the walk door)
//   O6 wedge_retained_tips membership (the wedge door verdict)
//   O7 node epoch list after landing (= L_new(C_B), re-derived by applying C_B)
// PATHS: walk door | wedge door | execute_reorg apply loop x {Full gate, Light gate}
// INPUT PARTITIONS (block x derivable from local state x producer):
//   C_{B+1}  after C_B (not the node's): L_new(C_B)[s1] != L_new(D_B)[s1] -> admitted, applied
//   C'_{B+1} after C_B: L_new(D_B)[s1], not L_new(C_B)[s1]        -> door admits, reorg refuses,
//            node back on D_{B+1} (SEC-FIXVERIFY-001)
//   X_{B+1}  after C_B: unregistered key                           -> door refuses (membership)
//   Y_B      parent F (derivable): not L_close[slot]                -> door refuses
//   Z_{B+1}  parent D_B (the node's own boundary, derivable): not L_new(D_B)[s1]
//            -> doors refuse; direct execute_reorg refuses and restores D_{B+1} (Full) | applies (Light gate)
// MATRIX: walk x {C, C', X, Y, Z}; wedge x {C, X, Y, Z}; execute_reorg x Z x {Full, Light}.
//   O2-O4 in every cell; O5 on walk cells; O6 on wedge cells; O7 in reorg_lands.
//   The execute_reorg schedule cells (C' walk, Z Full, Z Light) run at heights shifted past
//   the genesis phase, where a Full apply takes the round-robin arm.
//   C_{B+2} / C'_{B+2} / X_{B+2} / Z_{B+2}: slot s2 where both lists name the same leader,
//   so only the B+1 block discriminates; a 3-block branch outweighs the node's 2 blocks.
// covers: door_epoch validation_checks wedge_escape fork_walk force_reorg block_handling reorg_schedule
// covers: bins/node/src/node/validation_checks/door_epoch.rs
// covers: bins/node/src/node/validation_checks/mod.rs
// covers: bins/node/src/node/wedge_escape.rs
// covers: bins/node/src/node/fork_walk.rs
// covers: bins/node/src/node/force_reorg.rs
// covers: bins/node/src/node/block_handling.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;

use doli_core::Block;
use inc_i_236_m2_reorg_support::weight_recorded;
use inc_i_281_boundary_fx::*;

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE postboundary={cell} wrong={}", wrong as u8);
}

// P1 (Must) — Decision: a failure means the fork walk still judges the block after the walked branch's own boundary with the node's L_new(D_B), so a minority whose dead branch holds the other boundary block can never walk back.
#[tokio::test]
async fn walk_door_accepts_post_boundary_block() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &canon, f).await;
    let admitted = weight_recorded(&fx.node, &canon[1].hash()).await;
    probe("walk_door_accepts_post_boundary_block", !admitted);
    assert!(
        admitted,
        "walk: C_B+1 = L_new(C_B)[s1] was refused, judged with the node's L_new(D_B) ({result:?})"
    );
    result.expect("walk: Ok");
}

// P1 (Must) — Decision: a failure means the wedge escape still drops the canonical block after the canonical boundary (lab reproduction), so the minority stays wedged.
#[tokio::test]
async fn wedge_door_accepts_post_boundary_block() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    gossip(&mut fx, &canon[0]).await;
    let c_b_retained = fx.retained(&canon[0]);
    gossip(&mut fx, &canon[1]).await;
    let kept = fx.retained(&canon[1]) || fx.state().await.at_b1 == Some(canon[1].hash());
    gossip(&mut fx, &canon[2]).await;
    probe("wedge_door_accepts_post_boundary_block", !kept);
    assert!(
        c_b_retained,
        "precondition: the wedge door admits C_B under L_close (M2.1)"
    );
    assert!(
        kept,
        "wedge: C_B+1 = L_new(C_B)[s1] dropped as ineligible under the node's L_new(D_B)"
    );
    assert_landed(&fx, &canon, B, "wedge").await;
}

// P1 (Must) — Decision: a failure means a node on a dead branch that holds the other boundary block does not end on the canonical branch after the walk, on any of tip, index, persisted tip or epoch list.
#[tokio::test]
async fn reorg_lands_on_post_boundary_block() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &canon, f).await;
    let st = fx.state().await;
    probe(
        "reorg_lands_on_post_boundary_block",
        st.at_b1 != Some(canon[1].hash()),
    );
    result.expect("walk: Ok");
    assert_landed(&fx, &canon, B, "reorg").await;
    assert_eq!(
        active(&fx.node.epoch_state),
        fx.l_canon,
        "reorg: the node re-derived L_new(C_B) by applying C_B"
    );
}

// P1 / SEC-FIXVERIFY-001 (Must) — Decision: a failure means execute_reorg still applies a block after the walked boundary whose producer is not scheduled under L_new(C_B), or leaves the node on the lighter walked prefix instead of its own tip.
#[tokio::test]
async fn reorg_refuses_ineligible_post_boundary_block() {
    let mut fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    let br = fx.ineligible();
    let f = fx.f_hash;
    let before = fx.state().await;
    let result = run_walk(&mut fx, &br, f).await;
    let st = fx.state().await;
    let applied = fx.applied(&br[1]);
    probe(
        "reorg_refuses_ineligible_post_boundary_block",
        applied || st != before,
    );
    assert!(
        !applied,
        "C'_B+1 = L_new(D_B)[s1], not L_new(C_B)[s1], was applied by the reorg ({result:?}, {st:?})"
    );
    assert!(!fx.applied(&br[2]), "C'_B+2 applied on a refused parent");
    assert_eq!(
        st.best_hash,
        fx.d[1].hash(),
        "the refused reorg returns the node to its own tip D_B+1 ({result:?}, {st:?})"
    );
    assert_eq!(st.best_height, fx.b() + 1, "height of the restored tip");
    assert_eq!(
        (st.at_b, st.at_b1, st.at_b2),
        (Some(fx.d[0].hash()), Some(fx.d[1].hash()), None),
        "canonical D_B, D_B+1 restored, nothing above"
    );
    assert_eq!(st.persisted, Some(fx.d[1].hash()), "persisted tip = D_B+1");
    result.expect("walk: Ok");
}

// P1 (Must) — Decision: a failure means the fix skipped the schedule check for a block whose list IS derivable (parent F, or parent on the node's own boundary), the INC-I-010 fix-4 relaxation, or moved the refusal past a rollback.
#[tokio::test]
async fn door_refuses_derivable_ineligible_block() {
    type Make = fn(&Fx) -> Vec<Block>;
    let parts: [(&str, Make, bool); 2] = [
        ("closing", Fx::closing_ineligible, true),
        ("own_boundary", Fx::on_own_boundary, false),
    ];
    let mut admitted = Vec::new();
    for (label, make, on_f) in parts {
        let mut fx = fixture().await;
        let br = make(&fx);
        let cp = if on_f { fx.f_hash } else { fx.d[0].hash() };
        let before = fx.state().await;
        let result = run_walk(&mut fx, &br, cp).await;
        if fx.state().await != before || weight_recorded(&fx.node, &br[0].hash()).await {
            admitted.push(format!("walk/{label} ({result:?})"));
        }

        let mut fx = fixture().await;
        let br = make(&fx);
        let before = fx.state().await;
        gossip(&mut fx, &br[0]).await;
        let weighed = weight_recorded(&fx.node, &br[0].hash()).await;
        if fx.retained(&br[0]) || weighed || fx.state().await != before {
            admitted.push(format!("wedge/{label}"));
        }
    }
    probe(
        "door_refuses_derivable_ineligible_block",
        !admitted.is_empty(),
    );
    assert!(
        admitted.is_empty(),
        "doors that let a derivable, unscheduled block past the schedule check or moved the tip: {admitted:?}"
    );
}

// P1 (Must) — Decision: a failure means skipping the round-robin arm for a non-derivable block also dropped the membership arm, so any key can author a walked branch past the door.
#[tokio::test]
async fn nonderivable_block_unknown_producer_refused() {
    let mut admitted = Vec::new();
    let mut fx = fixture().await;
    let br = fx.unknown();
    let before = fx.state().await;
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &br, f).await;
    let weighed = weight_recorded(&fx.node, &br[1].hash()).await;
    if weighed || fx.applied(&br[1]) || fx.state().await != before {
        admitted.push(format!("walk ({result:?})"));
    }

    let mut fx = fixture().await;
    let br = fx.unknown();
    let before = fx.state().await;
    gossip(&mut fx, &br[0]).await;
    let c_b_retained = fx.retained(&br[0]);
    gossip(&mut fx, &br[1]).await;
    let weighed = weight_recorded(&fx.node, &br[1].hash()).await;
    if fx.retained(&br[1]) || weighed || fx.state().await != before {
        admitted.push("wedge".to_string());
    }
    probe(
        "nonderivable_block_unknown_producer_refused",
        !admitted.is_empty(),
    );
    assert!(
        c_b_retained,
        "precondition: the wedge door admits C_B, so X_B+1 reaches it"
    );
    assert!(
        admitted.is_empty(),
        "doors that let X_B+1 (unregistered, not in the GSet) past the membership check: {admitted:?}"
    );
}

// P1 / SEC-FIXVERIFY-001 (Must) — Decision: a failure means execute_reorg still applies an unscheduled block that reached it without a door (Light apply never checks the schedule), or the refusal leaves the node below the tip it had before the reorg.
#[tokio::test]
async fn reorg_refuses_unscheduled_block_without_door() {
    let mut fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    let br = fx.on_own_boundary();
    let before = fx.state().await;
    let result = run_direct_reorg(&mut fx, &br).await;
    let st = fx.state().await;
    let applied = fx.applied(&br[0]);
    probe(
        "reorg_refuses_unscheduled_block_without_door",
        applied || st != before,
    );
    assert!(
        !applied,
        "execute_reorg applied Z_B+1, not L_new(D_B)[s1] at its parent D_B ({result:?}, {st:?})"
    );
    assert!(!fx.applied(&br[1]), "Z_B+2 applied on a refused parent");
    assert_eq!(
        st.best_hash,
        fx.d[1].hash(),
        "the refused reorg re-applies the node's own D_B+1 ({result:?}, {st:?})"
    );
    assert_eq!(st.best_height, fx.b() + 1, "height of the restored tip");
    assert_eq!(
        (st.at_b1, st.at_b2),
        (Some(fx.d[1].hash()), None),
        "canonical D_B+1 restored, nothing above"
    );
    assert_eq!(st.persisted, Some(fx.d[1].hash()), "persisted tip = D_B+1");
}

// P1 (Should) — Decision: a failure means the reorg schedule check ignores the INC-I-010 layer-3 Light window, so a node after snap sync or in the floor-fallback window refuses valid blocks its divergent list misjudges.
#[tokio::test]
async fn reorg_light_gating_skips_schedule_check() {
    let mut refused = Vec::new();
    for gate in ["floor_fallback_window", "snap_sync_height"] {
        let mut fx = fixture_at(post_genesis_offset()).await;
        fx.assert_past_genesis();
        if gate == "floor_fallback_window" {
            fx.node.floor_fallback_window = true;
        } else {
            fx.node.snap_sync_height = Some(F + fx.o);
        }
        let br = fx.on_own_boundary();
        let result = run_direct_reorg(&mut fx, &br).await;
        let st = fx.state().await;
        let tip = br[1].hash();
        let landed = st.best_hash == tip
            && st.best_height == fx.b() + 2
            && (st.at_b1, st.at_b2) == (Some(br[0].hash()), Some(tip))
            && st.persisted == Some(tip);
        if !landed {
            refused.push(format!("{gate} ({result:?}, {st:?})"));
        }
    }
    probe(
        "reorg_light_gating_skips_schedule_check",
        !refused.is_empty(),
    );
    assert!(
        refused.is_empty(),
        "the reorg schedule check ran inside the INC-I-010 layer-3 Light window: {refused:?}"
    );
}
