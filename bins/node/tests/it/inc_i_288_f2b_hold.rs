//! INC-I-288 F-2b REL2-288-P1-001: at/above the AH the INC-I-281 hold
//! (`hold_removed_block`) weighs ONE cached sibling block against our ONE removed
//! block. Under production weights (every producer weighs 1 in its first year,
//! `ProducerInfo::effective_weight`) `1 > 1` is false, so a cached 2-block sibling
//! branch that out-weighs the removed block from the common ancestor never holds
//! it, and a fork-remedy rollback is undone by a re-delivery (INV-FORK-003).
//! Live rig (M5 live). No synthetic weight: every recorded weight is read from the
//! producer set.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(D, peer) -> Result<()>, D = our tip A3
//   removed by a coordinator depth-1 rollback (tip P = A2), re-delivered (ExtendsTip);
//   the fork cache holds siblings on P, recorded at production weight
//   O1: chain_state best_hash / best_height
//   O2: block_store canonical entry at F+3 (D's height)
//   O3: state_db persisted tip
//   O4: sync_manager local tip (height, hash)
//   O5: return value — Ok(()) (asserted by the gossip helper)
// PATHS: P1 ExtendsTip re-delivery of D through hold_removed_block
// INPUT PARTITIONS:
//   P1a (H1): at/above the AH, cached branch C, C+1 on P (weight 1 each), hash(C) > hash(D)
//             -> D held (not re-applied): the branch weighs 2 from P, D weighs 1
//   P1b (H2): at/above the AH, one cached sibling S on P, tied, hash(S) < hash(D)
//             -> D re-applies at once (AUDIT-P2-629-004, as f2_004)
//   BELOW-AH (H3a): P1a shape -> old rule: C alone, tied, higher hash = TieKeep -> D re-applies
//   BELOW-AH (H3b): P1b shape -> old rule: tied lower hash = TieSwitch -> D held
// MATRIX: P1a O1-O5 · P1b O1-O5 · H3a O1-O5 · H3b O1-O5
//
//! covers: fork_cache.rs fork_recovery.rs wedge_escape.rs doors.rs fork_choice.rs mod.rs block_handling.rs rollback_memory.rs

use crypto::Hash;
use doli_core::Block;

use super::inc_i_288_f2_common::*;

/// `b`'s producer weight at `height`, as `ProducerInfo::effective_weight` gives it.
async fn production_weight(r: &Rig, b: &Block, height: u64) -> u64 {
    r.node
        .producer_set
        .read()
        .await
        .get_by_pubkey(&b.header.producer)
        .map(|p| p.effective_weight(height))
        .expect("fixture: the block's producer is registered")
}

/// A competing block as fork choice holds it after a walk (INC-I-281
/// `cache_sibling`): body cached, weight recorded — at the PRODUCTION weight.
async fn cache_at_production_weight(r: &Rig, b: &Block, height: u64) -> u64 {
    let w = production_weight(r, b, height).await;
    r.node
        .fork_block_cache
        .write()
        .await
        .insert(b.hash(), b.clone());
    r.node.sync_manager.write().await.record_fork_block_weight(
        b.hash(),
        b.header.prev_hash,
        w,
        height,
    );
    w
}

/// The first slot after `after` whose leader's block at `height` on `parent`
/// hashes below `d` (`lower`) or above it (`!lower`).
fn sibling(r: &Rig, d: &Block, parent: Hash, height: u64, after: u32, lower: bool) -> Block {
    let p = r.node.params.clone();
    (after + 1..after + 200)
        .map(|s| block(height, s, parent, leader(&r.keys, s), &p))
        .find(|b| (b.hash().as_bytes() < d.hash().as_bytes()) == lower)
        .expect("fixture: a sibling with the wanted hash order within 200 slots")
}

/// Tip A3 = D on P = A2; the coordinator rolls D back. Returns (rig, D, P).
async fn roll_back_d() -> (Rig, Block, Block) {
    let mut r = live_rig(&[210, 211, 212], &[]).await;
    let (p, d) = (r.ours[1].clone(), r.ours[2].clone());
    rollback_depth_one(&mut r).await;
    assert_on(
        &r,
        &p,
        F + 2,
        "precondition: the depth-1 rollback left P = A2 as tip",
    )
    .await;
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

/// The INC-I-281 shape: C (sibling of D on P, hash above D) and C+1 on C, both
/// cached and recorded at production weight. Returns (C, C+1).
async fn cache_heavier_branch(r: &Rig, d: &Block, p: &Block) -> (Block, Block) {
    let c = sibling(r, d, p.hash(), F + 3, d.header.slot, false);
    let s1 = c.header.slot + 1;
    let c1 = block(F + 4, s1, c.hash(), leader(&r.keys, s1), &r.node.params);
    let wc = cache_at_production_weight(r, &c, F + 3).await;
    let wc1 = cache_at_production_weight(r, &c1, F + 4).await;
    let wd = production_weight(r, d, F + 3).await;
    assert_eq!(
        (wc, wc1, wd),
        (1, 1, 1),
        "precondition: production weight 1 per block (first year)"
    );
    assert!(
        c.hash().as_bytes() > d.hash().as_bytes(),
        "precondition: C hashes above D, so no tie-break can hold D"
    );
    (c, c1)
}

/// One tied sibling S of D on P with a LOWER hash, cached and recorded at
/// production weight.
async fn cache_tied_lower_sibling(r: &Rig, d: &Block, p: &Block) -> Block {
    let s = sibling(r, d, p.hash(), F + 3, d.header.slot, true);
    let ws = cache_at_production_weight(r, &s, F + 3).await;
    assert_eq!(
        ws,
        production_weight(r, d, F + 3).await,
        "precondition: S ties D at production weight"
    );
    s
}

/// D was NOT re-applied: no chain-tip output names D.
async fn assert_not_reapplied(r: &Rig, d: &Block, label: &str) {
    let after = tip(r).await;
    assert_ne!(
        after.best_hash,
        d.hash(),
        "{label}: D was re-applied (tip) — {after:?}"
    );
    assert_ne!(
        r.node.block_store.get_hash_by_height(F + 3).unwrap(),
        Some(d.hash()),
        "{label}: D is canonical again at F+3"
    );
    assert_ne!(
        after.persisted,
        Some(d.hash()),
        "{label}: D is the persisted tip again"
    );
    assert_ne!(
        r.node.sync_manager.read().await.local_tip().1,
        d.hash(),
        "{label}: the sync manager tip is D again"
    );
}

// REL2-288-P1-001 INC-I-288 — Decision: a failure means the INC-I-281 hold is inert under production weights, so a block a fork-remedy rollback removed is re-applied at once while a heavier cached sibling branch exists (INV-FORK-003).
#[tokio::test]
async fn f2b_h1_inc_i_288_heavier_cached_branch_holds_removed_block() {
    let (mut r, d, p) = roll_back_d().await;
    let (c, c1) = cache_heavier_branch(&r, &d, &p).await;
    assert_eq!(
        chain_weight(&r, &c1.hash()).await,
        chain_weight(&r, &c.hash()).await + 1,
        "precondition: C+1 extends C in the reorg handler"
    );

    gossip(&mut r, &d).await;

    assert_not_reapplied(
        &r,
        &d,
        "F-2b H1: the cached branch C, C+1 (weight 2 from P) out-weighs D (weight 1), \
         yet D was not held",
    )
    .await;
    assert_eq!(
        tip(&r).await.canonical_f1,
        Some(r.ours[0].hash()),
        "F-2b H1: A1 stays canonical"
    );
}

// REL2-288-P1-001 INC-I-288 — Decision: a failure means the H1 fix brought back the hold on a one-block tie, so our own rolled-back tip waits up to 300 s behind a lower-hash sibling the weight door refuses (AUDIT-P2-629-004).
#[tokio::test]
async fn f2b_h2_inc_i_288_one_tied_lower_hash_sibling_does_not_hold() {
    let (mut r, d, p) = roll_back_d().await;
    cache_tied_lower_sibling(&r, &d, &p).await;

    gossip(&mut r, &d).await;

    assert_on(
        &r,
        &d,
        F + 3,
        "F-2b H2: D re-applied at once (a one-block tie does not hold)",
    )
    .await;
    assert_eq!(
        tip(&r).await.canonical_f1,
        Some(r.ours[0].hash()),
        "F-2b H2: A1 stays canonical"
    );
}

// REL2-288-P1-001 INC-I-288 — Decision: a failure means the branch-weight hold leaked below its activation height, so pre-AH re-apply of a rolled-back block beside a cached 2-block branch changed without a gate.
#[tokio::test]
async fn f2b_h3a_inc_i_288_below_ah_heavier_branch_old_rule_reapplies() {
    if !in_below_ah_child(
        "inc_i_288_f2b_hold::f2b_h3a_inc_i_288_below_ah_heavier_branch_old_rule_reapplies",
    ) {
        return;
    }
    let (mut r, d, p) = roll_back_d().await;
    cache_heavier_branch(&r, &d, &p).await;

    gossip(&mut r, &d).await;

    assert_on(
        &r,
        &d,
        F + 3,
        "F-2b H3a below AH: the old rule weighs C alone (tied, higher hash) and re-applies D",
    )
    .await;
}

// REL2-288-P1-001 INC-I-288 — Decision: a failure means the at/above-AH tie rule leaked below its activation height, so the pre-AH TieSwitch hold of a rolled-back block was dropped without a gate.
#[tokio::test]
async fn f2b_h3b_inc_i_288_below_ah_tied_lower_hash_old_rule_holds() {
    if !in_below_ah_child(
        "inc_i_288_f2b_hold::f2b_h3b_inc_i_288_below_ah_tied_lower_hash_old_rule_holds",
    ) {
        return;
    }
    let (mut r, d, p) = roll_back_d().await;
    cache_tied_lower_sibling(&r, &d, &p).await;

    gossip(&mut r, &d).await;

    assert_not_reapplied(&r, &d, "F-2b H3b below AH: the old TieSwitch arm holds D").await;
    assert_on(&r, &p, F + 2, "F-2b H3b below AH: P stays the tip").await;
}
