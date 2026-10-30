//! INC-I-288 F-2c (B) SEC-INPUT-R3-001 / SEC-LOGIC-R3-002: at/above the AH the
//! INC-I-281 hold (`hold_removed_block`) weighs a whole cached rival branch on the
//! tip against our removed block D ALONE (`branch_weight(&[D])`). D's own cached
//! descendants E1, E2 are skipped (`cached_branches_on(.., skip = D)`), so the
//! rival C, C+1 (2) holds D although D, E1, E2 (3) is the heavier branch from the
//! shared parent P. Both sides are cached and recorded the same way (production
//! weight, 1 per producer), so the hold compares like with like. Live rig (M5 live).
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(D, peer) -> Result<()>, D = our tip A3
//   removed by a coordinator depth-1 rollback (tip P = A2), re-delivered (ExtendsTip);
//   the fork cache holds the rival C, C+1 on P and D's children E1, E2, all
//   recorded at production weight
//   O1: chain_state best_hash / best_height (the tip is on D's branch)
//   O2: block_store canonical entry at F+3 (= D) and at the tip height
//   O3: state_db persisted tip (= the tip)
//   O4: sync_manager local tip height (= the tip height)
//   O5: return value — Ok(()) (asserted by the gossip helper)
// PATHS: P1 ExtendsTip re-delivery of D through hold_removed_block
// INPUT PARTITIONS:
//   B1: at/above the AH, rival C, C+1 (2) < D, E1, E2 (3) -> D is NOT held, it
//       re-applies at once (its children may follow by drain)
//   (rival heavier than D alone with no D children = f2b_h1, held; one tied
//    lower-hash sibling = f2b_h2, not held — both kept green, not repeated here)
//   B0 BELOW-AH: B1 shape -> old rule (C alone, tied, higher hash = TieKeep) -> D re-applies
// MATRIX: B1 O1-O5 · B0 O1-O5
//
//! covers: fork_cache.rs block_handling.rs fork_choice.rs doors.rs rollback_memory.rs

use crypto::Hash;
use doli_core::Block;

use super::inc_i_288_f2c_common::*;

/// Tip A3 = D on P = A2; the coordinator rolls D back. Returns (rig, D, P).
async fn roll_back_d() -> (Rig, Block, Block) {
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let (p, d) = (r.ours[1].clone(), r.ours[2].clone());
    rollback_depth_one(&mut r).await;
    assert_on(&r, &p, F + 2, "precondition: tip P = A2 after the rollback").await;
    assert!(
        r.node
            .sync_manager
            .read()
            .await
            .removed_by_rollback(&d.hash()),
        "precondition: D is remembered as removed by a fork-remedy rollback"
    );
    (r, d, p)
}

/// The first slot after `after` whose leader's block at `height` on `parent`
/// hashes above `d`.
fn higher_sibling(r: &Rig, d: &Block, parent: Hash, height: u64, after: u32) -> Block {
    let p = r.node.params.clone();
    (after + 1..after + 200)
        .map(|s| block(height, s, parent, leader(&r.keys, s), &p))
        .find(|b| b.hash().as_bytes() > d.hash().as_bytes())
        .expect("fixture: a sibling hashing above D within 200 slots")
}

/// D's children E1 (F+4), E2 (F+5) and the rival C (F+3, hash above D), C+1
/// (F+4) on P, all cached and recorded at production weight. Returns (E, C).
async fn cache_both_branches(r: &Rig, d: &Block, p: &Block) -> (Vec<Block>, Vec<Block>) {
    let s = d.header.slot;
    let e = chain_on(r, d.hash(), F + 4, &[s + 1, s + 2]);
    let c0 = higher_sibling(r, d, p.hash(), F + 3, s + 2);
    let c1 = block(
        F + 4,
        c0.header.slot + 1,
        c0.hash(),
        leader(&r.keys, c0.header.slot + 1),
        &r.node.params,
    );
    let mut ws = Vec::new();
    for (b, h) in [(&e[0], F + 4), (&e[1], F + 5), (&c0, F + 3), (&c1, F + 4)] {
        ws.push(cache_at_production_weight(r, b, h).await);
    }
    ws.push(production_weight(r, d, F + 3).await);
    assert_eq!(
        ws,
        vec![1; 5],
        "precondition: production weight 1 per block (first year)"
    );
    assert_eq!(
        recorded_weight(r, &d.hash()).await,
        Some(1),
        "precondition: D keeps its recorded weight after the rollback (at/above the AH)"
    );
    (e, vec![c0, c1])
}

/// D re-applied at F+3 and the tip is D or one of its children.
async fn assert_d_branch_kept(r: &Rig, d: &Block, e: &[Block], label: &str) {
    let t = tip(r).await;
    assert_eq!(
        r.node.block_store.get_hash_by_height(F + 3).unwrap(),
        Some(d.hash()),
        "{label}: D is not canonical at F+3 (held) — tip {t:?}"
    );
    let branch: Vec<Hash> = std::iter::once(d.hash())
        .chain(e.iter().map(Block::hash))
        .collect();
    let at = branch
        .iter()
        .position(|h| *h == t.best_hash)
        .unwrap_or_else(|| panic!("{label}: tip is not on D's branch — {t:?}"));
    let height = F + 3 + at as u64;
    assert_eq!(t.best_height, height, "{label}: tip height");
    assert_eq!(
        r.node.block_store.get_hash_by_height(height).unwrap(),
        Some(t.best_hash),
        "{label}: canonical entry at the tip height"
    );
    assert_eq!(t.persisted, Some(t.best_hash), "{label}: persisted tip");
    assert_eq!(t.sync_height, height, "{label}: sync manager tip");
    assert_eq!(
        t.canonical_f1,
        Some(r.ours[0].hash()),
        "{label}: A1 stays canonical"
    );
}

// SEC-INPUT-R3-001 SEC-LOGIC-R3-002 INC-I-288 — Decision: a failure means the hold weighs our removed block alone against a whole rival branch, so a lighter cached branch stalls the heavier removed branch for up to 300 s.
#[tokio::test]
async fn f2c_b1_inc_i_288_removed_block_with_heavier_own_branch_is_not_held() {
    let (mut r, d, p) = roll_back_d().await;
    let (e, c) = cache_both_branches(&r, &d, &p).await;
    assert_eq!(
        chain_weight(&r, &c[1].hash()).await,
        chain_weight(&r, &c[0].hash()).await + 1,
        "precondition: C+1 extends C in the reorg handler"
    );
    assert_eq!(
        chain_weight(&r, &e[1].hash()).await,
        chain_weight(&r, &d.hash()).await + 2,
        "precondition: E1, E2 extend D in the reorg handler"
    );

    gossip(&mut r, &d).await;

    assert_d_branch_kept(
        &r,
        &d,
        &e,
        "F-2c B1: D, E1, E2 (3) out-weighs C, C+1 (2) from P, yet D was held",
    )
    .await;
}

// SEC-INPUT-R3-001 INC-I-288 — Decision: a failure means the branch-against-branch hold leaked below its activation height, so pre-AH re-apply of a rolled-back block changed without a gate.
#[tokio::test]
async fn f2c_b0_inc_i_288_below_ah_removed_block_with_children_old_rule_reapplies() {
    if !in_below_ah_child(
        "inc_i_288_f2c_hold_branch::f2c_b0_inc_i_288_below_ah_removed_block_with_children_old_rule_reapplies",
    ) {
        return;
    }
    let (mut r, d, p) = roll_back_d().await;
    let s = d.header.slot;
    let e = chain_on(&r, d.hash(), F + 4, &[s + 1, s + 2]);
    let c0 = higher_sibling(&r, &d, p.hash(), F + 3, s + 2);
    let c1 = block(
        F + 4,
        c0.header.slot + 1,
        c0.hash(),
        leader(&r.keys, c0.header.slot + 1),
        &r.node.params,
    );
    for (b, h) in [(&e[0], F + 4), (&e[1], F + 5), (&c0, F + 3), (&c1, F + 4)] {
        cache_at_production_weight(&r, b, h).await;
    }

    gossip(&mut r, &d).await;

    assert_d_branch_kept(
        &r,
        &d,
        &e,
        "F-2c B0 below AH: the old rule weighs C alone (tied, higher hash) and re-applies D",
    )
    .await;
}
