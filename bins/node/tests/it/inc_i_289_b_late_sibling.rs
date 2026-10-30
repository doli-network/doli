//! INC-I-289 door (b): our tip D (slot s) at F+1. A late sibling S1 (slot s+2, on
//! F) and its child S2 (slot s+3) arrive; the wedge-escape door reorgs on delta=1
//! (2 blocks vs 1) at S2's arrival, ten seconds before canonical E1 (slot s+4, on
//! D) arrives. Contract at the gate: no adoption on arrival; a canonical reply that
//! ties the sibling branch keeps D's branch; with no reply a tick-driven weighed
//! door still adopts the heavier sibling (liveness). Below the gate: old rule.
//! B1 (INC-I-289 K-1 rule: support = distinct producers + held attestations, not
//! block count): the other producers' verified attestations for D arrive first, so
//! D's branch has 3 of 3 supporters and S1,S2 has 2 of 3.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (HeightOccupied / Orphan
//   reroute -> retain_sibling_and_try_escape -> plan_reorg -> execute_reorg), then
//   fn Node::run_periodic_tasks(); B1 also fn Node::on_new_attestation(data, peer)
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: block_store holds S2 (S2 was applied = was the tip at some point)
//   O3: return values Ok(()) (gossip / tick helpers expect); B1 Verdict::Accept
// PATHS: P0 (B1) gossip attestations for D from both other producers; P1 S1 then
//   S2 by gossip (no adoption on arrival); P2 E1 by gossip + tick; P3 no E1, up to 5 ticks
// INPUT PARTITIONS: B1 at/above AH, D attested 3 of 3, S 2 of 3, E1 arrives -> D,E1 kept;
//   B2 at/above AH, no reply -> S2 adopted within 5 ticks;
//   B0 below AH (child re-exec) -> S2 adopted at arrival (old rule)
// MATRIX: B1 O1-O3 · B2 O1,O3 · B0 O1,O3
//
//! covers: block_handling.rs (Orphan reroute) wedge_escape.rs (door) periodic.rs

use doli_core::{Attestation, Block};
use network::service::Verdict;
use network::PeerId;

use super::inc_i_289_common::*;

const S: u32 = 210;

/// Tip D at F+1 (slot s); S1 (F+1, s+2, on F), S2 (F+2, s+3), E1 (F+2, s+4, on D).
async fn late_sibling() -> (Rig, Block, Vec<Block>, Block) {
    let r = rig(&[S]).await;
    let d = r.ours[0].clone();
    let sib = chain_on(&r, r.f_hash, F + 1, &[S + 2, S + 3]);
    let e1 = chain_on(&r, d.hash(), F + 2, &[S + 4]).remove(0);
    (r, d, sib, e1)
}

// B1 INC-I-289 — Decision: a failure means a two-block sibling of two producers still wins over a block every producer attested, by block count (door b of the h=835,607 fork).
#[tokio::test]
async fn b1_inc_i_289_two_block_sibling_does_not_win_before_canonical_reply() {
    let (mut r, d, sib, e1) = late_sibling().await;
    // The mainnet fact: the bond majority's verified gossip attestations for D.
    for k in r
        .keys
        .iter()
        .filter(|k| *k.public_key() != d.header.producer)
    {
        let att = Attestation::new(
            d.hash(),
            d.header.slot,
            F + 1,
            1,
            k.private_key(),
            *k.public_key(),
        );
        let v = r
            .node
            .on_new_attestation(att.to_bytes(), PeerId::random())
            .await;
        assert_eq!(
            v,
            Verdict::Accept,
            "B1 precondition: attestation for D admitted"
        );
    }
    gossip(&mut r, &sib[0]).await;
    gossip(&mut r, &sib[1]).await;

    assert_on(&r, &d, F + 1, "B1: S1,S2 adopted at arrival (delta=1)").await;
    assert!(!stored(&r, &sib[1]), "B1: S2 was applied at arrival");

    gossip(&mut r, &e1).await;
    tick(&mut r).await;

    assert_on(&r, &e1, F + 2, "B1: canonical E1 must be the tip").await;
    assert!(!stored(&r, &sib[1]), "B1: the sibling S2 became the tip");
}

// B2 INC-I-289 — Decision: a failure means the arrival hold also blocked liveness, so a truly heavier sibling branch is never adopted when no canonical reply comes.
#[tokio::test]
async fn b2_inc_i_289_control_two_block_sibling_wins_when_no_canonical_reply() {
    let (mut r, _d, sib, _e1) = late_sibling().await;
    gossip(&mut r, &sib[0]).await;
    gossip(&mut r, &sib[1]).await;

    let ticks = tick_until(&mut r, &sib[1].hash(), 5).await;
    eprintln!("[B2] ticks to adopt S2: {ticks}");

    assert_on(
        &r,
        &sib[1],
        F + 2,
        "B2: heavier S1,S2 adopted within 5 ticks",
    )
    .await;
}

// B0 INC-I-289 — Decision: a failure means the arrival hold leaked below its activation height, so the pre-gate wedge door changed without a gate.
#[tokio::test]
async fn b0_inc_i_289_below_ah_two_block_sibling_adopted_on_arrival() {
    if !in_below_ah_child(
        "inc_i_289_b_late_sibling::b0_inc_i_289_below_ah_two_block_sibling_adopted_on_arrival",
    ) {
        return;
    }
    let (mut r, _d, sib, _e1) = late_sibling().await;
    gossip(&mut r, &sib[0]).await;
    gossip(&mut r, &sib[1]).await;

    assert_on(&r, &sib[1], F + 2, "B0 below AH: S2 adopted at arrival").await;
}
