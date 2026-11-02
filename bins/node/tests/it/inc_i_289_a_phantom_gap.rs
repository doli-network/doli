//! INC-I-289 door (a): ONE peer claims best height H+1 for a block nobody can
//! deliver. The coordinator's gap is network_tip_height - local = 1, two peers
//! that agree with our tip return empty headers, the apply is recent, so Rule 1
//! fires ShallowRollback { depth: 1 } and rolls back canonical D on a claim no
//! block ever backed (mainnet h=835,607).
//! Contract at the gate (devnet 0): a height claim with no block behind it does
//! not roll back our tip. Below the gate: no walk-down either, the claim carries no
//! header proof (INC-I-294 M-2).
//!
// OUTPUT CONTRACT: fn Node::run_periodic_tasks() (evidence -> classify_and_dispatch
//   -> Rule 1 -> ShallowRollback executor rollback_one_block(CoordinatorApproved))
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: rollback counters (shallow_rollback_count, cumulative_rollback_depth)
//   O3: rollback memory (removed_by_rollback(D))
//   O4: coordinator gap input before the tick (network_tip_height - local = 1)
//   O5: return value Ok(()) (tick helper expect)
// PATHS: P1 one peer status at local+1, no block for it, 2 EmptyHeaders, recent apply
// INPUT PARTITIONS: A1 at/above the AH (devnet 0) -> no rollback;
//   A0 below the AH (child re-exec) -> no rollback: no header proof (INC-I-294 M-2);
//   A0+P below the AH with a fresh header proof for D -> exactly one depth-1 rollback
// MATRIX: A1 O1-O5 · A0 O1-O5 · A0+P O1,O2,O4,O5 (O3 N/A: rollback memory unchanged)
//
//! covers: peers.rs (add_peer network tip) recovery.rs (Rule 1) periodic.rs

use network::PeerId;

use super::inc_i_289_common::*;

/// Tip D at F+1; one peer claims F+2 (random hash, later slot); two peers on D
/// return empty headers. Returns (rig, D, gap before the tick).
async fn phantom_claim() -> (Rig, doli_core::Block, u64) {
    let r = rig(&[210]).await;
    let d = r.ours[0].clone();
    peer_at(&r, F + 2, crypto::hash::hash(b"undeliverable H+1"), 212).await;
    for _ in 0..2 {
        let agree = peer_at(&r, F + 1, d.hash(), d.header.slot).await;
        empty_headers(&r, agree).await;
    }
    let gap = coordinator_gap(&r).await;
    (r, d, gap)
}

// A1 INC-I-289 — Decision: a failure means a single peer's height claim with no block behind it still makes the coordinator roll back the canonical tip (door a of the h=835,607 fork).
#[tokio::test]
async fn a1_inc_i_289_single_peer_claim_without_block_does_not_roll_back_tip() {
    let (mut r, d, gap) = phantom_claim().await;
    assert_eq!(
        gap, 1,
        "precondition: the coordinator sees gap=1 from one peer"
    );
    let before = rollbacks(&r);

    tick(&mut r).await;

    assert_on(
        &r,
        &d,
        F + 1,
        "A1: canonical D rolled back on a height claim no block backed",
    )
    .await;
    assert_eq!(
        rollbacks(&r).0,
        before.0,
        "A1: shallow rollback count moved (Rule 1 fired on the phantom gap)"
    );
    assert!(!removed(&r, &d.hash()).await, "A1: D is in rollback memory");
}

// A0 INC-I-289 + INC-I-294 M-2 — Decision: a failure means a node below the gate still rolls back its tip on one peer's height claim and empty headers, with no header proof that a peer's chain leaves it.
#[tokio::test]
async fn a0_inc_i_289_below_ah_single_peer_claim_needs_header_proof() {
    if !in_below_ah_child(
        "inc_i_289_a_phantom_gap::a0_inc_i_289_below_ah_single_peer_claim_needs_header_proof",
    ) {
        return;
    }
    let (mut r, d, gap) = phantom_claim().await;
    assert_eq!(
        gap, 1,
        "precondition: the coordinator sees gap=1 from one peer"
    );
    let before = rollbacks(&r);

    tick(&mut r).await;

    assert_eq!(
        rollbacks(&r),
        before,
        "A0 below AH: Rule 1 rolled back with no header proof"
    );
    assert_on(&r, &d, F + 1, "A0 below AH: canonical D rolled back").await;
    assert!(
        !removed(&r, &d.hash()).await,
        "A0 below AH: D is in rollback memory"
    );

    // Arm B: a fresh header proof for D (tip F+1) keeps the old below-AH rule.
    let (mut r, _d, gap) = phantom_claim().await;
    assert_eq!(gap, 1, "precondition B: the coordinator sees gap=1");
    (r.node.sync_manager.write().await).report_header_divergence(PeerId::random(), F + 1);
    let before = rollbacks(&r);

    tick(&mut r).await;

    assert_eq!(
        rollbacks(&r),
        (before.0 + 1, before.1 + 1),
        "A0 below AH + header proof: the old Rule 1 takes exactly one depth-1 rollback"
    );
    assert_eq!(
        tip_hash(&r).await,
        r.f_hash,
        "A0 below AH + header proof: tip walked to F"
    );
}
