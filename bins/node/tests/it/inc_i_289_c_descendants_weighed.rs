//! INC-I-289 door (b) aftermath: after the node reorged from D to the sibling
//! branch S1,S2, canonical E1 (on D) arrives. D is no longer canonical and is not
//! in `wedge_retained_tips`, so E1 takes the generic Orphan arm and is cached
//! unweighed forever; the heavier canonical branch D,E1,E2 (3 vs 2) is never
//! adopted. Contract at the gate: descendants of a reorged-out tip are weighed and
//! the heavier branch wins within a few ticks. Control: descendants of a RETAINED
//! sibling are weighed today (the reroute works for retained roots).
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (Orphan arm: reroute to
//   retain_sibling_and_try_escape when the parent is weighed, else cache only), then
//   fn Node::run_periodic_tasks()
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: the descendant's weight is recorded in the reorg handler or it is retained
//   O3: return values Ok(()) (gossip / tick helpers expect)
// PATHS: P1 descendants of a reorged-out tip by gossip + up to 5 ticks;
//   P2 descendants of a retained sibling by gossip + up to 5 ticks
// INPUT PARTITIONS: C1 at/above AH, parent = reorged-out tip D (3 vs 2) -> E2 wins;
//   C0 at/above AH, parent = retained sibling S1 (3 vs 1) -> S3 wins
// MATRIX: C1 O1-O3 · C0 O1-O3
//
//! covers: block_handling.rs (Orphan arm) wedge_escape.rs fork_cache.rs periodic.rs

use super::inc_i_289_common::*;

const S: u32 = 210;

// C1 INC-I-289 — Decision: a failure means canonical blocks on a reorged-out tip are still cached unweighed, so the node stays on the lighter sibling branch forever (the h=835,607 lock).
#[tokio::test]
async fn c1_inc_i_289_descendants_of_reorged_out_tip_are_weighed() {
    let mut r = rig(&[S]).await;
    let d = r.ours[0].clone();
    let sib = chain_on(&r, r.f_hash, F + 1, &[S + 2, S + 3]);
    gossip(&mut r, &sib[0]).await;
    gossip(&mut r, &sib[1]).await;
    tick_until(&mut r, &sib[1].hash(), 5).await;
    assert_on(&r, &sib[1], F + 2, "precondition: on the sibling S2").await;

    let e = chain_on(&r, d.hash(), F + 2, &[S + 4, S + 5]);
    gossip(&mut r, &e[0]).await;
    gossip(&mut r, &e[1]).await;
    let ticks = tick_until(&mut r, &e[1].hash(), 5).await;
    let w = weighed(&r, &e[0]).await;
    eprintln!("[C1] ticks={ticks} E1 (recorded, cached, retained)={w:?}");

    assert_on(
        &r,
        &e[1],
        F + 3,
        "C1: heavier D,E1,E2 (3) not adopted over S1,S2 (2)",
    )
    .await;
    assert!(w.0 || w.2, "C1: E1 is cached but never weighed — {w:?}");
}

// C0 INC-I-289 — Decision: a failure means the fix broke the existing reroute of a retained sibling's descendants into the weighed door.
#[tokio::test]
async fn c0_inc_i_289_control_descendants_of_retained_sibling_are_weighed() {
    let mut r = rig(&[S]).await;
    let d = r.ours[0].clone();
    let s1 = chain_on(&r, r.f_hash, F + 1, &[S + 2]).remove(0);
    gossip(&mut r, &s1).await;
    assert_on(&r, &d, F + 1, "precondition: S1 tie retained, tip D").await;
    assert!(weighed(&r, &s1).await.2, "precondition: S1 is retained");

    let more = chain_on(&r, s1.hash(), F + 2, &[S + 3, S + 4]);
    gossip(&mut r, &more[0]).await;
    gossip(&mut r, &more[1]).await;
    let ticks = tick_until(&mut r, &more[1].hash(), 5).await;
    let w = weighed(&r, &more[0]).await;
    eprintln!("[C0] ticks={ticks} S2 (recorded, cached, retained)={w:?}");

    assert_on(&r, &more[1], F + 3, "C0: heavier S1,S2,S3 adopted").await;
    assert!(
        w.0 || w.2,
        "C0: S2 (child of retained S1) not weighed — {w:?}"
    );
}
