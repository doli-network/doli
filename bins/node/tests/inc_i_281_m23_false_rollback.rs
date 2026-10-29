// INC-I-281 M2.3 — P2 (R-3b regression from 7c9b8288, Must): the one-slot REAPPLY hold
// keeps a node from re-adopting its own block T after a FALSE fork-remedy rollback, so a
// majority producer builds a replacement and peers that adopted T wedge (lab reproduction).
// Rule under test: a removed hash is held only while a cached competing sibling (same
// parent, different hash) would win fork choice against it; otherwise it is re-adopted at
// once (INC-I-147 D4).
//
// OUTPUT CONTRACT: non-fork-choice intake of a block removed by rollback_one_block
//   (CoordinatorApproved), re-delivered at once (inside the old one-slot window).
// Functions: Node::handle_new_block (gossip, ExtendsTip after hold_removed_block),
//   Node::on_sync_response (orphan-chase Block reply), Node::drain_cached_children
//   (behind an ExtendsTip apply); the hold reads the fork cache and the reorg handler.
// OUTPUTS:
//   O1 return value (Ok on every path)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at F+1 / F+2
//   O4 state_db persisted chain-state tip
//   O5 sync_manager local tip height
//   O6 canonical status of the competing sibling S
// PATHS: gossip/chase | cached drain
// INPUT PARTITIONS: cached competing sibling S of the removed block T =
//   {none, lighter (chain weight < T's), heavier (> T's)}; equal weight is not built
//   (the tie verdict depends on hash order). The rollback clears the reorg handler, so
//   S is recorded as fork choice records a retained sibling (0 or HEAVY over its parent)
//   and T weighs its producer's weight over the same parent.
// MATRIX (O2-O5 via assert_on / at_fork_point, O6 where S exists):
//   gossip/chase x none    -> D1, then D2 re-adopted at once
//   drain        x none    -> D1 re-adopted, cached D2 drained at once
//   gossip       x lighter -> D1 re-adopted at once, S (C1) not adopted
//   drain        x lighter -> D2 drained at once, S (D2b) not adopted
//   gossip       x heavier -> D1 held, S (C1) becomes the tip
//   drain        x heavier -> covered by inc_i_281_m2_rollback_memory test 8
// The rig: the node's own D1, D2 on F, both removed by two fork-remedy rollbacks.
// covers: fork_cache rollback_memory block_handling fork_choice
// covers: bins/node/src/node/fork_cache.rs
// covers: crates/network/src/sync/reorg/fork_choice.rs
// covers: bins/node/src/node/block_handling.rs
// covers: crates/network/src/sync/manager/rollback_memory.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_m2_support;

use doli_core::Block;
use inc_i_236_m2_reorg_support::seed_cache;
use inc_i_281_m2_support::*;
use network::protocols::SyncResponse;
use network::PeerId;

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE falserollback={cell} wrong={}", wrong as u8);
}

fn on_chain(r: &Rig, b: &Block) -> bool {
    r.node
        .block_store
        .get_height_by_hash(&b.hash())
        .expect("store")
        .is_some()
}

// P2 (Must) — Decision: a failure means a falsely rolled-back block with no competing sibling is still held by a timer, so the majority producer replaces it and peers that adopted it wedge.
#[tokio::test]
async fn no_sibling_readopts_at_once() {
    let mut r = rig().await;
    let t0 = roll_back_to_f(&mut r).await;
    let (d1, d2) = (r.dead[0].clone(), r.dead[1].clone());
    gossip(&mut r, &d1).await;
    let after_gossip = obs(&r).await;
    r.node
        .on_sync_response(PeerId::random(), SyncResponse::Block(Some(d2.clone())))
        .await
        .expect("chase reply: Ok");
    let after_chase = obs(&r).await;
    let in_window = t0.elapsed();
    probe(
        "no_sibling_readopts_at_once",
        after_gossip.best_hash != d1.hash() || after_chase.best_hash != d2.hash(),
    );
    assert_within_window(&r, t0);
    assert_eq!(
        after_gossip.best_hash,
        d1.hash(),
        "gossip: the removed D1 (no competing sibling) was held {in_window:?} after the rollback"
    );
    assert_on(&r, &d2, F + 2, "chase reply: removed D2 re-adopted at once").await;
    assert_eq!(after_chase.at_f1, Some(d1.hash()), "D1 stays canonical");
}

// P2 (Must) — Decision: a failure means the cached-child drain still holds a falsely removed child with no competing sibling, so the node's own branch does not come back in one intake.
#[tokio::test]
async fn no_sibling_readopts_at_once_cached_drain() {
    let mut r = rig().await;
    let t0 = roll_back_to_f(&mut r).await;
    let (d1, d2) = (r.dead[0].clone(), r.dead[1].clone());
    seed_cache(&r.node, std::slice::from_ref(&d2)).await;
    gossip(&mut r, &d1).await;
    let after = obs(&r).await;
    probe(
        "no_sibling_readopts_at_once_cached_drain",
        after.best_hash != d2.hash(),
    );
    assert_within_window(&r, t0);
    assert_on(
        &r,
        &d2,
        F + 2,
        "D1 re-adopted and the cached D2 drained at once",
    )
    .await;
    assert_eq!(after.at_f1, Some(d1.hash()), "D1 canonical under D2");
}

// P2 (Must) — Decision: a failure means a cached sibling that cannot out-weight the removed block still holds it, i.e. the hold is not decided by fork choice.
#[tokio::test]
async fn lighter_sibling_readopts_at_once() {
    let mut r = rig().await;
    let t0 = roll_back_to_f(&mut r).await;
    let (d1, d2, c1) = (r.dead[0].clone(), r.dead[1].clone(), r.canon[0].clone());
    cache_sibling(&r, &c1, F + 1, 0).await;
    let (w_s, w_t) = sibling_vs_removed(&r, &c1, &d1, F + 1).await;
    gossip(&mut r, &d1).await;
    let after = obs(&r).await;
    probe(
        "lighter_sibling_readopts_at_once",
        after.best_hash != d1.hash(),
    );
    assert_within_window(&r, t0);
    assert!(
        w_s < w_t,
        "precondition: C1 adds {w_s} over F, D1's producer {w_t}"
    );
    assert_on(
        &r,
        &d1,
        F + 1,
        "removed D1 re-adopted at once; its cached sibling C1 is lighter",
    )
    .await;
    assert!(!on_chain(&r, &c1), "the lighter sibling C1 was adopted");
    gossip(&mut r, &d2).await;
    assert_on(&r, &d2, F + 2, "removed D2 re-adopted at once").await;
}

// P2 (Must) — Decision: a failure means the drain holds a removed child although its only cached sibling is lighter.
#[tokio::test]
async fn lighter_sibling_readopts_at_once_cached_drain() {
    let mut r = rig().await;
    let t0 = roll_back_to_f(&mut r).await;
    let (d1, d2) = (r.dead[0].clone(), r.dead[1].clone());
    // Higher slot than D2, so the drain offers D2 first.
    let d2b = child_of(&r, &d1, F + 2, 208);
    seed_cache(&r.node, std::slice::from_ref(&d2)).await;
    cache_sibling(&r, &d2b, F + 2, 0).await;
    let (w_s, w_t) = sibling_vs_removed(&r, &d2b, &d2, F + 2).await;
    gossip(&mut r, &d1).await;
    let after = obs(&r).await;
    probe(
        "lighter_sibling_readopts_at_once_cached_drain",
        after.best_hash != d2.hash(),
    );
    assert_within_window(&r, t0);
    assert!(
        w_s < w_t,
        "precondition: D2b adds {w_s} over D1, D2's producer {w_t}"
    );
    assert_on(
        &r,
        &d2,
        F + 2,
        "the drain re-adopted D2 at once; its cached sibling D2b is lighter",
    )
    .await;
    assert!(!on_chain(&r, &d2b), "the lighter sibling D2b was adopted");
}

// P2 (Must) — Decision: a failure means the fix dropped the R-3b protection: a removed block is re-adopted outside fork choice while a heavier competing sibling is cached.
#[tokio::test]
async fn heavier_sibling_holds_and_adopts_sibling() {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    let (d1, c1) = (r.dead[0].clone(), r.canon[0].clone());
    cache_sibling(&r, &c1, F + 1, HEAVY).await;
    let (w_s, w_t) = sibling_vs_removed(&r, &c1, &d1, F + 1).await;
    gossip(&mut r, &d1).await;
    let after = obs(&r).await;
    probe(
        "heavier_sibling_holds_and_adopts_sibling",
        !at_fork_point(&r, &after),
    );
    assert!(
        w_s > w_t,
        "precondition: C1 adds {w_s} over F, D1's producer {w_t}"
    );
    assert!(
        at_fork_point(&r, &after),
        "removed D1 re-adopted outside fork choice while a heavier sibling is cached: {after:?}"
    );
    gossip(&mut r, &c1).await;
    assert_on(&r, &c1, F + 1, "the heavier sibling C1 becomes the tip").await;
    assert!(!on_chain(&r, &d1), "D1 stays off the chain");
}
