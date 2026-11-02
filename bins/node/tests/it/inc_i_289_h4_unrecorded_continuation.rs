//! INC-I-289 H-4 (AUDIT-P2-631-002): the INC-I-281 hold (`hold_removed_block`)
//! weighs our removed block D plus its heaviest CACHED continuation against each
//! cached rival branch. D's children E1, E2 that arrived by gossip went through
//! the generic Orphan arm: cached, but never recorded in the reorg handler, so the
//! continuation weighs 0. A recorded rival C, C+1 (2) then holds D although D, E1,
//! E2 (3) is the heavier branch from the shared parent P.
//! Control: a recorded rival (2) against D alone (1) still holds D.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(D, peer) -> Result<()>, D = tip A3
//   removed by a coordinator depth-1 rollback (tip P = A2), re-delivered (ExtendsTip
//   -> hold_removed_block -> apply -> drain_cached_children)
//   O1: chain_state best_hash / best_height (on D's branch or still P)
//   O2: block_store canonical entry at F+3 (= D, or none when held)
//   O3: state_db persisted tip and sync manager tip (inside assert_on)
//   O4: return value Ok(()) (gossip helper expect)
// PATHS: P1 ExtendsTip re-delivery of D through hold_removed_block
// INPUT PARTITIONS: H4 at/above AH, rival C,C+1 recorded (2) vs D + UNRECORDED
//   gossip children E1,E2 (3) -> D re-applies;
//   H4c at/above AH, rival recorded (2) vs D alone (1) -> D held
// MATRIX: H4 O1-O4 · H4c O1-O4
//
//! covers: fork_cache.rs (hold_removed_block) block_handling.rs (Orphan arm)

use crypto::Hash;
use doli_core::Block;

use super::inc_i_289_common::*;

/// Tip A3 = D on P = A2; the coordinator rolls D back; the rival C (F+3, hash
/// above D), C+1 (F+4) on P is cached and recorded. Returns (rig, D, P).
async fn roll_back_d_with_rival() -> (Rig, Block, Block) {
    let mut r = rig(&[210, 211, 212]).await;
    let (p, d) = (r.ours[1].clone(), r.ours[2].clone());
    rollback_depth_one(&mut r).await;
    assert_on(&r, &p, F + 2, "precondition: tip P = A2 after the rollback").await;
    assert!(removed(&r, &d.hash()).await, "precondition: D is removed");
    let c0 = higher_sibling(&r, &d, p.hash(), F + 3, d.header.slot + 2);
    let c1 = chain_on(&r, c0.hash(), F + 4, &[c0.header.slot + 1]).remove(0);
    cache_at_production_weight(&r, &c0, F + 3).await;
    cache_at_production_weight(&r, &c1, F + 4).await;
    (r, d, p)
}

// H-4 AUDIT-P2-631-002 INC-I-289 — Decision: a failure means our removed block's gossip-delivered continuation still weighs 0 in the hold, so a lighter recorded rival stalls the heavier removed branch.
#[tokio::test]
async fn h4_inc_i_289_unrecorded_own_continuation_outweighs_recorded_rival() {
    let (mut r, d, _p) = roll_back_d_with_rival().await;
    let s = d.header.slot;
    let e = chain_on(&r, d.hash(), F + 4, &[s + 1, s + 2]);
    gossip(&mut r, &e[0]).await;
    gossip(&mut r, &e[1]).await;
    for b in &e {
        let w = weighed(&r, b).await;
        assert!(w.1 && !w.0, "precondition: E cached and unrecorded — {w:?}");
    }

    gossip(&mut r, &d).await;

    let t = tip_hash(&r).await;
    assert_eq!(
        r.node.block_store.get_hash_by_height(F + 3).unwrap(),
        Some(d.hash()),
        "H-4: D held — D,E1,E2 (3) out-weighs rival C,C+1 (2) but E1,E2 are unrecorded"
    );
    let branch: Vec<Hash> = std::iter::once(d.hash())
        .chain(e.iter().map(Block::hash))
        .collect();
    let at = branch.iter().position(|h| *h == t);
    let at = at.unwrap_or_else(|| panic!("H-4: tip {t:.8} is not on D's branch"));
    let top = if at == 0 {
        d.clone()
    } else {
        e[at - 1].clone()
    };
    assert_on(&r, &top, F + 3 + at as u64, "H-4: tip on D's branch").await;
}

// H-4c AUDIT-P2-631-002 INC-I-289 — Decision: a failure means the fix stopped the INC-I-281 hold, so a heavier recorded rival no longer holds a bare removed block.
#[tokio::test]
async fn h4_control_inc_i_289_recorded_rival_heavier_than_bare_removed_block_holds() {
    let (mut r, d, p) = roll_back_d_with_rival().await;

    gossip(&mut r, &d).await;

    assert_on(&r, &p, F + 2, "H-4c: D re-applied against a heavier rival").await;
    assert_eq!(
        r.node.block_store.get_hash_by_height(F + 3).unwrap(),
        None,
        "H-4c: D canonical at F+3"
    );
}
