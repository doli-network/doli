//! INC-I-293 F3 (run 647 Q-2, trial .4 n6): the wedge door weighs only the
//! arriving block and its ancestors. When the heavier branch's children arrive
//! FIRST (cached as orphans, parent unknown) and its root R1 arrives LAST, the root
//! is judged alone ("retained (not strictly heavier)") and the cached children are
//! never re-checked: the node stays on its lighter branch. Root FIRST (controls C0,
//! C2) reroutes each child through the door ("descends from a validated retained
//! sibling") and reorgs. Contract: arrival order does not decide the tip; root LAST
//! reaches the heavier tip within N_TICKS = 5 ticks.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (HeightOccupied arm ->
//   retain_sibling_and_try_escape; Orphan arm -> reroute when the parent is a
//   retained sibling, else cache only), then fn Node::run_periodic_tasks() up to
//   N_TICKS times
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: per block (weight recorded in the reorg handler, body cached, retained)
//   O3: return values Ok(()) (gossip / tick helpers expect)
// PATHS: P1 root first, then the children (Orphan reroute into the door per child);
//   P2 the children first (cached orphans), root last (door weighs the root alone)
// INPUT PARTITIONS: X1 tip D (1 producer) vs R1..R3 (3), later-slot root;
//   X2 n6 shape: D1..D4 (2 producers) vs R1..R6 (3), earlier-slot root, heavier
//   only at depth 4; at/above the INC-I-288 AH (devnet 0) and below it (env override)
// MATRIX: C0 P1xX1 O1-O3 · A1 P2xX1 O1-O3 · C2 P1xX2 O1-O3 · A2 P2xX2 O1-O3 ·
//   B1 P1,P2xX1 below AH O1-O3 · B2 P1,P2xX2 below AH O1-O3
//! covers: block_handling.rs (HeightOccupied + Orphan arms) wedge_escape.rs fork_cache.rs

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::KeyPair;
use doli_core::Block;
use inc_i_289_common::*;

/// The stated bound: the heavier branch is the tip within this many ticks.
const N_TICKS: usize = 5;
const S: u32 = 210;

/// Fixed seeds (the d3cbd5c1 K1C fixture): every hash is a function of keys and slots.
fn keys() -> Vec<KeyPair> {
    (1..=3u8).map(|i| KeyPair::from_seed([i; 32])).collect()
}

/// X1: tip D (slot 210, leader 0) at F+1. R1 slot 212 (leader 2, later slot than
/// D), R2 213 (0), R3 214 (1). Root alone = 1 vs 1 (tie, retained); R1,R2 = 2 > 1.
async fn x1() -> (Rig, Block, Vec<Block>) {
    let r = rig_with_keys(keys(), &[S]).await;
    let root = chain_on(&r, r.f_hash, F + 1, &[S + 2]).remove(0);
    let kids = chain_on(&r, root.hash(), F + 2, &[S + 3, S + 4]);
    (r, root, kids)
}

/// X2 (n6): tip D4 at F+4, slots 210,211,213,214 (leaders 0,1,0,1 -> 2 producers).
/// R1 slot 209 (leader 2, EARLIER than D1), R2..R6 slots 212,216,217,218,219
/// (leaders 2,0,1,2,0). Distinct producers: R1 = 1, R1..R2 = 1, R1..R3 = 2 (tie),
/// R1..R4 = 3 > 2. A fix that reroutes only the direct child (R2) still loses.
async fn x2() -> (Rig, Block, Vec<Block>) {
    let r = rig_with_keys(keys(), &[210, 211, 213, 214]).await;
    let root = chain_on(&r, r.f_hash, F + 1, &[209]).remove(0);
    let kids = chain_on(&r, root.hash(), F + 2, &[212, 216, 217, 218, 219]);
    (r, root, kids)
}

/// Children first, then the root. Asserts the children are cached orphans (body
/// cached, no weight, not retained) and the tip is unchanged before the root.
async fn children_then_root(r: &mut Rig, root: &Block, kids: &[Block], label: &str) {
    let top = r.ours.last().unwrap().clone();
    let top_h = F + r.ours.len() as u64;
    for k in kids {
        gossip(r, k).await;
    }
    for (i, k) in kids.iter().enumerate() {
        let w = weighed(r, k).await;
        assert_eq!(
            w,
            (false, true, false),
            "{label} precondition: R{} is a cached, unweighed orphan before its root",
            i + 2
        );
    }
    assert_on(r, &top, top_h, &format!("{label} precondition: tip D")).await;
    gossip(r, root).await;
    eprintln!(
        "[{label}] root R1 (recorded, cached, retained)={:?}",
        weighed(r, root).await
    );
}

async fn root_then_children(r: &mut Rig, root: &Block, kids: &[Block]) {
    gossip(r, root).await;
    for k in kids {
        gossip(r, k).await;
    }
}

// C0 INC-I-293 — Decision: a failure means the root-first reroute (the PASS order of trial .3) broke, so A1 no longer isolates arrival order as the only difference.
#[tokio::test]
async fn c0_inc_i_293_control_root_first_later_slot_root_reorgs() {
    let (mut r, root, kids) = x1().await;
    root_then_children(&mut r, &root, &kids).await;
    let ticks = tick_until(&mut r, &kids[1].hash(), N_TICKS).await;
    eprintln!("[C0] ticks={ticks}");
    assert_on(
        &r,
        &kids[1],
        F + 3,
        "C0: root first, heavier R1..R3 adopted",
    )
    .await;
}

// A1 INC-I-293 F3 — Decision: a failure means a heavier branch whose root arrives after its children is never weighed, so the node stays on its lighter branch (n6, trial .4).
#[tokio::test]
async fn a1_inc_i_293_root_last_cached_children_are_weighed() {
    let (mut r, root, kids) = x1().await;
    children_then_root(&mut r, &root, &kids, "A1").await;
    let ticks = tick_until(&mut r, &kids[1].hash(), N_TICKS).await;
    eprintln!(
        "[A1] ticks={ticks} R2 (recorded, cached, retained)={:?}",
        weighed(&r, &kids[0]).await
    );
    assert_on(
        &r,
        &kids[1],
        F + 3,
        "A1: root R1 last — heavier R1..R3 (3) not adopted over D (1) within N=5 ticks",
    )
    .await;
}

// C2 INC-I-293 — Decision: a failure means the X2 weights differ from the derivation (not heavier only at depth 4), so A2 does not discriminate a direct-child-only fix.
#[tokio::test]
async fn c2_inc_i_293_control_root_first_n6_shape_heavier_only_at_depth_four() {
    let (mut r, root, kids) = x2().await;
    let d4 = r.ours[3].clone();
    root_then_children(&mut r, &root, &kids[..2]).await;
    tick_until(&mut r, &kids[1].hash(), N_TICKS).await;
    assert_on(
        &r,
        &d4,
        F + 4,
        "C2: R1..R3 ties D1..D4 (2 vs 2), tip stays D4",
    )
    .await;
    for k in &kids[2..] {
        gossip(&mut r, k).await;
    }
    let ticks = tick_until(&mut r, &kids[4].hash(), N_TICKS).await;
    eprintln!("[C2] ticks={ticks}");
    assert_on(
        &r,
        &kids[4],
        F + 6,
        "C2: root first, R4 makes it heavier, R6 adopted",
    )
    .await;
}

// A2 INC-I-293 F3 — Decision: a failure means the cached subtree of a retained root is not walked to depth 4, so the n6 minority (2 producers vs 3) is never reorged.
#[tokio::test]
async fn a2_inc_i_293_root_last_n6_shape_heavier_only_at_depth_four() {
    let (mut r, root, kids) = x2().await;
    children_then_root(&mut r, &root, &kids, "A2").await;
    let ticks = tick_until(&mut r, &kids[4].hash(), N_TICKS).await;
    eprintln!(
        "[A2] ticks={ticks} R4 (recorded, cached, retained)={:?}",
        weighed(&r, &kids[2]).await
    );
    assert_on(
        &r,
        &kids[4],
        F + 6,
        "A2: earlier-slot root R1 last — heavier R1..R6 (3) not adopted over D1..D4 (2) within N=5 ticks",
    )
    .await;
}

/// The tip is the fork point F or a minority block: no door reorg moved it to the
/// heavier branch. Below the gate the coordinator's blind rollback (recovery.rs Rule
/// 1b) may walk it back toward F within the ticks; that exit is not pinned here.
async fn on_minority_or_fork_point(r: &Rig) -> bool {
    let tip = tip_hash(r).await;
    tip == r.f_hash || r.ours.iter().any(|b| b.hash() == tip)
}

// B1 INC-I-293 — Decision: a failure means the root-last behavior changed below the INC-I-288 gate, so the fix's gating (rule #0b / INV-12) must be decided before this pin moves.
// Shows: below the gate a later-slot root reaches the door (block_handling.rs:178 is
// false for it), and root-last has the same door defect: root first reorgs to R3;
// root last leaves the root retained alone, R2,R3 cached unweighed, the tip on D,
// and no door reorg to R3 within the N_TICKS bound.
#[tokio::test]
async fn b1_inc_i_293_below_gate_later_slot_root_last_pins_the_same_door_defect() {
    if !in_below_ah_child("b1_inc_i_293_below_gate_later_slot_root_last_pins_the_same_door_defect")
    {
        return;
    }
    let (mut r, root, kids) = x1().await;
    root_then_children(&mut r, &root, &kids).await;
    tick_until(&mut r, &kids[1].hash(), N_TICKS).await;
    assert_on(&r, &kids[1], F + 3, "B1 below AH: root first reorgs to R3").await;

    let (mut r, root, kids) = x1().await;
    let d = r.ours[0].clone();
    children_then_root(&mut r, &root, &kids, "B1").await;
    assert_eq!(
        weighed(&r, &root).await,
        (true, true, true),
        "B1 below AH: later-slot root reached the door and is retained alone"
    );
    for k in &kids {
        assert_eq!(
            weighed(&r, k).await,
            (false, true, false),
            "B1 below AH: root last, child not re-checked"
        );
    }
    assert_on(&r, &d, F + 1, "B1 below AH: root last, door keeps D").await;
    tick_until(&mut r, &kids[1].hash(), N_TICKS).await;
    let h = r.node.chain_state.read().await.best_height;
    eprintln!("[B1] root last after {N_TICKS} ticks: tip h={h}");
    assert!(
        on_minority_or_fork_point(&r).await,
        "B1 below AH: root last, no door reorg to the heavier branch within N=5 ticks (h={h})"
    );
}

// B2 INC-I-293 — Decision: a failure means an earlier-slot root now reaches the wedge door below the gate, so the fix opened a door that block_handling.rs:178 keeps shut on mainnet.
// Shows: below the gate an earlier-slot root never reaches the door in either order
// ([FORK_GUARD] Better block dropped: not cached, not retained, no weight); the tip
// stays on the minority and the exit is the coordinator, not the door.
#[tokio::test]
async fn b2_inc_i_293_below_gate_earlier_slot_root_never_reaches_the_door() {
    if !in_below_ah_child("b2_inc_i_293_below_gate_earlier_slot_root_never_reaches_the_door") {
        return;
    }
    for root_last in [false, true] {
        let (mut r, root, kids) = x2().await;
        let d4 = r.ours[3].clone();
        if root_last {
            children_then_root(&mut r, &root, &kids, "B2").await;
        } else {
            root_then_children(&mut r, &root, &kids).await;
        }
        let label = format!("B2 below AH (root_last={root_last})");
        let dropped = format!("{label}: earlier-slot root dropped at block_handling.rs:178");
        assert_eq!(weighed(&r, &root).await, (false, false, false), "{dropped}");
        assert_on(&r, &d4, F + 4, &format!("{label}: door never ran, tip D4")).await;
        tick_until(&mut r, &kids[4].hash(), N_TICKS).await;
        let h = r.node.chain_state.read().await.best_height;
        eprintln!("[B2] root_last={root_last} after {N_TICKS} ticks: tip h={h}");
        assert_eq!(weighed(&r, &root).await, (false, false, false), "{dropped}");
        assert!(
            on_minority_or_fork_point(&r).await,
            "{label}: no door reorg to the heavier branch within N=5 ticks (h={h})"
        );
    }
}
