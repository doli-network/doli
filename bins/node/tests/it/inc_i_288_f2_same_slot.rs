//! INC-I-288 F-2 AUDIT-P2-629-005: the weight door records a fork block's weight
//! before any slot-advance check (`SlotNotAdvancing` is only checked at apply).
//! One slot leader X signs D1 (a valid sibling on F) and D2..D5 at the SAME slot,
//! each on the previous one. Each re-enters the door as a child of a retained tip
//! and adds X's weight, until the D-branch outweighs ours and the node rolls back
//! toward it (D1 applies, D2 fails, restore). Live rig (M5 live).
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(Dk, peer) -> Result<()>, k = 1..=5,
//   Dk.slot == D1.slot, Dk.prev == D(k-1) (D1.prev == F)
//   O1: ReorgHandler chain_weight(Dk) right after Dk is delivered
//   O2: chain_state best_hash / best_height, canonical F+1, persisted, sync tip (assert_on)
//   O3: block_store holds D1 (applied at least once = the node rolled back toward it)
//   O4: return value — Ok(()) (asserted by the gossip helper)
// PATHS: P1 D1 through the HeightOccupied door; P2 D2..D5 through the Orphan arm
//   (parent in wedge_retained_tips) into the same door
// INPUT PARTITIONS:
//   P2a: at/above the AH, same-slot child of a retained sibling -> no weight gain, no rollback
//   BELOW-AH: the old door -> weight grows by X's weight per variant (old behavior kept)
//   (a slot-advancing child of a retained sibling gains weight: C-1 B1..B3 suites)
// MATRIX: P1 O1 · P2a O1-O4 · below-AH O1,O4
//
//! covers: wedge_escape.rs block_handling.rs doors.rs mod.rs reorg_restore.rs

use doli_core::Block;

use super::inc_i_288_f2_common::*;

const VARIANTS: usize = 5;

/// D1..D5 by one leader X (not the node's key), all at X's slot, chained on F.
fn same_slot_chain(r: &Rig) -> Vec<Block> {
    let own = *r.keys[0].public_key();
    let slot = (216..246)
        .find(|s| *leader(&r.keys, *s).public_key() != own)
        .expect("fixture: a slot led by another producer");
    let x = leader(&r.keys, slot).clone();
    let p = r.node.params.clone();
    let mut prev = r.f_hash;
    (1..=VARIANTS as u64)
        .map(|i| {
            let b = block(F + i, slot, prev, &x, &p);
            prev = b.hash();
            b
        })
        .collect()
}

/// Delivers D1..D5; returns the chain weight of each right after its delivery.
async fn deliver_chain(r: &mut Rig, d: &[Block]) -> Vec<u64> {
    let mut w = Vec::with_capacity(d.len());
    for b in d {
        gossip(r, b).await;
        w.push(chain_weight(r, &b.hash()).await);
    }
    w
}

// AUDIT-P2-629-005 INC-I-288 — Decision: a failure means one slot leader stacks same-slot variants that gain fork-choice weight and force a rollback plus restore on every node, once per variant.
#[tokio::test]
async fn f2_005_inc_i_288_same_slot_chain_gains_no_weight_and_no_rollback() {
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let d = same_slot_chain(&r);
    let w = deliver_chain(&mut r, &d).await;

    assert_eq!(w[0], F + 1, "precondition: D1 (a valid sibling) weighs F+1");
    for (k, wk) in w.iter().enumerate().skip(1) {
        assert!(
            *wk <= w[0],
            "F-2 005: D{} (same slot as its parent) gained weight: {wk} > D1 {} (weights {w:?})",
            k + 1,
            w[0]
        );
    }
    assert!(
        !stored(&r, &d[0]),
        "F-2 005: the node rolled back toward the same-slot branch (D1 was applied)"
    );
    assert_on(&r, &r.ours[2].clone(), F + 3, "F-2 005: tip A3 kept").await;
    assert_eq!(
        tip(&r).await.canonical_f1,
        Some(r.ours[0].hash()),
        "F-2 005: A1 stays canonical"
    );
}

// AUDIT-P2-629-005 INC-I-288 — Decision: a failure means the door's slot-advance refusal leaked below its activation height and changed pre-AH fork-block weights without a gate.
#[tokio::test]
async fn f2_005_inc_i_288_below_ah_same_slot_weight_still_grows() {
    if !in_below_ah_child(
        "inc_i_288_f2_same_slot::f2_005_inc_i_288_below_ah_same_slot_weight_still_grows",
    ) {
        return;
    }
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let d = same_slot_chain(&r);
    let w = deliver_chain(&mut r, &d).await;
    assert_eq!(
        w[..3],
        [F + 1, F + 2, F + 3],
        "F-2 005 below AH: the old door adds X's weight per same-slot variant"
    );
}
