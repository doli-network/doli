//! INC-I-288 F-2b P2-630-001 / SEC-INTEGRITY-R2-001: the slot-advance rule
//! (`SlotNotAdvancing` before the weight input) guards only the wedge door, and
//! only when the parent body is found. (1) The fork-recovery walk
//! (`evaluate_recovered_fork`) records every walked block's weight before any slot
//! check. (2) The door's `slot_not_advancing` fails OPEN when the parent is in
//! neither the fork cache nor the store. One slot leader X signs D1 on F and
//! D2..D5 at the SAME slot, each on the previous one. Live rig (M5 live), every
//! producer at its production weight (1).
//!
// OUTPUT CONTRACT:
//   fn Node::handle_completed_fork_recovery(CompletedRecovery { D1..D5, F }) -> Result<()>
//   fn Node::handle_new_block(Dk, peer) -> Result<()> (door; D1's body evicted, then back)
//   O1: ReorgHandler chain_weight(Dk) after the event
//   O2: chain_state best_hash / best_height, canonical, persisted, sync tip (assert_on A3)
//   O3: block_store holds D1 (applied at least once = the node rolled back toward D)
//   O4: return value — Ok(())
// PATHS: P1 completed fork walk -> evaluate_recovered_fork
//        P2 door: D1 retained, its body evicted from the fork cache (never stored), D2
//           re-enters through the Orphan arm (parent in wedge_retained_tips), then D1
//           is delivered again
// INPUT PARTITIONS:
//   P1a: at/above the AH, walk D1..D5 (one slot) -> D2..D5 add no weight, no rollback
//   P2a: at/above the AH, D2 while D1 is in neither cache nor store, D1 later
//        -> D2 adds no weight once D1 is known
//   BELOW-AH: P1 walk -> weight grows per same-slot variant (old behavior kept)
//   (a slot-advancing walk gains weight: C-1 / INC-I-235 walk suites)
// MATRIX: P1a O1-O4 · P2a O1-O4 · below-AH P1 O1
//
//! covers: fork_recovery.rs fork_walk.rs wedge_escape.rs block_handling.rs fork_cache.rs doors.rs

use doli_core::Block;
use network::sync::CompletedRecovery;

use super::inc_i_288_f2_common::*;

const VARIANTS: u64 = 5;

/// D1..Dn by one leader X (not the node's key), all at X's slot, chained on F.
fn same_slot_chain(r: &Rig, n: u64) -> Vec<Block> {
    let own = *r.keys[0].public_key();
    let slot = (216..246)
        .find(|s| *leader(&r.keys, *s).public_key() != own)
        .expect("fixture: a slot led by another producer");
    let x = leader(&r.keys, slot).clone();
    let mut prev = r.f_hash;
    (1..=n)
        .map(|i| {
            let b = block(F + i, slot, prev, &x, &r.node.params);
            prev = b.hash();
            b
        })
        .collect()
}

/// A completed walk of `d` connected at F, as the network tracker hands it over.
fn walk(r: &Rig, d: &[Block]) -> CompletedRecovery {
    CompletedRecovery {
        blocks: d.to_vec(),
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    }
}

async fn weights(r: &Rig, d: &[Block]) -> Vec<u64> {
    let mut w = Vec::with_capacity(d.len());
    for b in d {
        w.push(chain_weight(r, &b.hash()).await);
    }
    w
}

/// Our branch A1..A3 is untouched and no D block was ever applied.
async fn assert_kept_a3(r: &Rig, d: &[Block], label: &str) {
    assert!(
        !stored(r, &d[0]),
        "{label}: the node rolled back toward the same-slot branch (D1 was applied)"
    );
    assert_on(
        r,
        &r.ours[2].clone(),
        F + 3,
        &format!("{label}: tip A3 kept"),
    )
    .await;
    assert_eq!(
        tip(r).await.canonical_f1,
        Some(r.ours[0].hash()),
        "{label}: A1 stays canonical"
    );
}

// P2-630-001 SEC-INTEGRITY-R2-001 INC-I-288 — Decision: a failure means a completed fork walk of one leader's same-slot variants gains weight per variant and forces a rollback plus restore on a wedged node, once per variant branch.
#[tokio::test]
async fn f2b_s1_inc_i_288_walked_same_slot_chain_gains_no_weight_and_no_rollback() {
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let d = same_slot_chain(&r, VARIANTS);

    let res = r.node.handle_completed_fork_recovery(walk(&r, &d)).await;
    let w = weights(&r, &d).await;

    for (k, wk) in w.iter().enumerate() {
        assert!(
            *wk <= F + 1,
            "F-2b S1: walked D{} (same slot as its parent) carries weight {wk} > F+1 \
             (weights {w:?}, result {res:?})",
            k + 1
        );
    }
    assert_kept_a3(&r, &d, "F-2b S1").await;
    assert!(res.is_ok(), "F-2b S1: the walk returns Ok — {res:?}");
}

// SEC-INTEGRITY-R2-001 INC-I-288 — Decision: a failure means the door's slot-advance check fails open when the retained parent's body was evicted, so a same-slot child keeps branch weight after its parent is known again.
#[tokio::test]
async fn f2b_s2_inc_i_288_door_parent_unknown_then_known_same_slot_adds_no_weight() {
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let d = same_slot_chain(&r, 2);
    let (d1, d2) = (d[0].clone(), d[1].clone());

    gossip(&mut r, &d1).await;
    assert_eq!(
        weighed(&r, &d1).await,
        (true, true, true),
        "precondition: the door retained D1 (recorded, cached, retained)"
    );
    // The fork cache (cap 100) evicts D1's body; D1 stays in wedge_retained_tips (cap 64).
    r.node.fork_block_cache.write().await.remove(&d1.hash());
    assert!(
        !r.node
            .fork_block_cache
            .read()
            .await
            .contains_key(&d1.hash())
            && !stored(&r, &d1)
            && r.node.wedge_retained_tips.contains(&d1.hash()),
        "precondition: D1 retained, its body in neither the fork cache nor the store"
    );

    gossip(&mut r, &d2).await;
    gossip(&mut r, &d1).await;
    assert!(
        r.node
            .fork_block_cache
            .read()
            .await
            .contains_key(&d1.hash()),
        "precondition: the parent D1 is known again (cached by its re-delivery)"
    );

    let (w1, w2) = (
        chain_weight(&r, &d1.hash()).await,
        chain_weight(&r, &d2.hash()).await,
    );
    assert!(
        w2 <= w1,
        "F-2b S2: D2 (same slot as D1) gained weight while D1 was unknown and kept it \
         once D1 is known: D2 {w2} > D1 {w1}"
    );
    assert_kept_a3(&r, &d, "F-2b S2").await;
}

// P2-630-001 INC-I-288 — Decision: a failure means the walk's slot-advance refusal leaked below its activation height and changed pre-AH fork-block weights without a gate.
#[tokio::test]
async fn f2b_s3_inc_i_288_below_ah_walked_same_slot_weight_still_grows() {
    if !in_below_ah_child(
        "inc_i_288_f2b_same_slot_walk::f2b_s3_inc_i_288_below_ah_walked_same_slot_weight_still_grows",
    ) {
        return;
    }
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let d = same_slot_chain(&r, VARIANTS);
    let _ = r.node.handle_completed_fork_recovery(walk(&r, &d)).await;
    let w = weights(&r, &d).await;
    assert_eq!(
        w[..3],
        [F + 1, F + 2, F + 3],
        "F-2b S3 below AH: the old walk adds X's weight per same-slot variant"
    );
}
