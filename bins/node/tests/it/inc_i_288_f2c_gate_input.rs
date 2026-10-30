//! INC-I-288 F-2c (G) SEC-LOGIC-R3-001 / SEC-AUTH3-005: the slot rule
//! (`record_fork_block_at_slot`, `fork_choice.rs`) is gated on the RECORD's
//! height; every other INC-I-288 site (doors, hold, wedge short-circuit, sync
//! reset) is gated on the LOCAL TIP height. One gate input for all: the local tip.
//!   Mix B: tip L = H-1, a fork block recorded at h_r = H gets 0 from the slot rule,
//!          so below the gate the result is no longer f6ce5c67's.
//!   Mix A: tip L = H, a walked block at h_r < H gets no slot rule, so a same-slot
//!          variant still adds weight at/above the gate.
//! H is set per test through the AH env override, in a child process.
//! Live rig (M5 live), production weights (1 per producer), ours A1..A3 (L = F+3).
//!
// OUTPUT CONTRACT:
//   fn Node::handle_new_block(D2, peer) -> Result<()> (wedge door, Orphan reroute)
//   fn Node::handle_completed_fork_recovery(CompletedRecovery { D1, D2 }, F) -> Result<()>
//   O1: ReorgHandler recorded producer weight of D2 (`get_block_weight`)
//   O2: ReorgHandler chain_weight(D2) against chain_weight(D1)
//   O3: chain_state / canonical / persisted / sync tip (assert_on A3: no reorg either way)
//   O4: block_store holds D1 (applied at least once = a rollback toward D)
//   O5: return value — Ok(())
// PATHS: P1 wedge door: D1 (sibling of A1, slot S) retained, then D2 (slot S, on D1)
//           arrives as an orphan of a retained tip; the door records it at h_r = L+1
//        P2 completed fork walk D1, D2 connected at F (cp = F = H-3), h_r = F+1, F+2
// INPUT PARTITIONS:
//   G1 (P1, BELOW the gate): H = L+1 = F+4, D2 recorded at h_r = H -> as f6ce5c67
//       below the gate: D2 weighs its production weight (1), acc(D2) = acc(D1) + 1
//   G2 (P2, AT the gate): H = L = F+3, D2 recorded at h_r = F+2 < H -> the slot rule
//       applies: D2 weighs 0, acc(D2) = acc(D1)
//   (record and tip both above, or both below: f2b_s1 / f2b_s2 / f2b_s3)
// MATRIX: G1 O1-O5 · G2 O1-O5
//
//! covers: fork_choice.rs wedge_escape.rs fork_recovery.rs block_lifecycle.rs doors.rs

use doli_core::Block;
use network::sync::CompletedRecovery;

use super::inc_i_288_f2c_common::*;

const OURS: [u32; 3] = [210, 211, 212];

async fn assert_weights(r: &Rig, d: &[Block], want_d2: u64, label: &str) {
    let (w1, w2) = (
        chain_weight(r, &d[0].hash()).await,
        chain_weight(r, &d[1].hash()).await,
    );
    assert_eq!(
        recorded_weight(r, &d[1].hash()).await,
        Some(want_d2),
        "{label}: D2's recorded weight (acc D1 {w1}, acc D2 {w2})"
    );
    assert_eq!(
        w2,
        w1 + want_d2,
        "{label}: D2 accumulates on D1 by its recorded weight"
    );
}

async fn assert_kept_a3(r: &Rig, d: &[Block], label: &str) {
    assert!(
        !stored(r, &d[0]),
        "{label}: the node rolled back toward the same-slot branch (D1 applied)"
    );
    assert_on(r, &r.ours[2].clone(), F + 3, &format!("{label}: tip A3")).await;
}

// SEC-LOGIC-R3-001 SEC-AUTH3-005 INC-I-288 — Decision: a failure means the slot rule reads the record's height, so a node one block below the gate already zeroes a fork block and below-gate fork choice is not f6ce5c67's.
#[tokio::test]
async fn f2c_g1_inc_i_288_tip_below_gate_record_at_gate_keeps_old_weight() {
    if !in_ah_child(
        "inc_i_288_f2c_gate_input::f2c_g1_inc_i_288_tip_below_gate_record_at_gate_keeps_old_weight",
        F + 4,
    ) {
        return;
    }
    let mut r = live_rig(&OURS, &[]).await;
    let d = same_slot_burst(&r, 216, r.f_hash, F + 1, 2);
    gossip(&mut r, &d[0]).await;
    assert_eq!(
        weighed(&r, &d[0]).await,
        (true, true, true),
        "precondition: the door retained D1"
    );

    let res = r
        .node
        .handle_new_block(d[1].clone(), network::PeerId::random())
        .await;

    assert_eq!(
        weighed(&r, &d[1]).await,
        (true, true, true),
        "precondition: D2 went through the door (recorded, cached, retained)"
    );
    let w = production_weight(&r, &d[1], F + 4).await;
    assert_eq!(w, 1, "precondition: production weight 1");
    assert_weights(&r, &d, w, "F-2c G1 (L = H-1, h_r = H)").await;
    assert_kept_a3(&r, &d, "F-2c G1").await;
    assert!(res.is_ok(), "F-2c G1: Ok — {res:?}");
}

// SEC-LOGIC-R3-001 SEC-AUTH3-005 INC-I-288 — Decision: a failure means a node at/above the gate leaves a walked same-slot block below the gate unruled, so the closed same-slot stacking is open again for forks that cross the gate.
#[tokio::test]
async fn f2c_g2_inc_i_288_tip_at_gate_walked_record_below_gate_gets_slot_rule() {
    if !in_ah_child(
        "inc_i_288_f2c_gate_input::f2c_g2_inc_i_288_tip_at_gate_walked_record_below_gate_gets_slot_rule",
        F + 3,
    ) {
        return;
    }
    let mut r = live_rig(&OURS, &[]).await;
    let d = same_slot_burst(&r, 216, r.f_hash, F + 1, 2);
    let walk = CompletedRecovery {
        blocks: d.clone(),
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    };

    let res = r.node.handle_completed_fork_recovery(walk).await;

    assert_weights(&r, &d, 0, "F-2c G2 (L = H, h_r = H-1)").await;
    assert_kept_a3(&r, &d, "F-2c G2").await;
    assert!(res.is_ok(), "F-2c G2: Ok — {res:?}");
}
