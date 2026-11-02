//! INC-I-289 O-1 (AUDIT-P2-644-001/-002): the bond rank of the cached branches. A
//! block no bonded producer signs must not take the weight from an honest cached
//! branch: neither an orphan whose parent is `Hash::ZERO` (its epoch has no table)
//! nor a child with its parent's slot (it can never apply).
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(D, peer) (ExtendsTip -> hold_removed_block
//   -> branch_rank -> ranked_branches)
//   O1: canonical entry at F+3 (None = D held, Some(D) = D re-applied)
//   O2: chain_state best_hash (= P while D is held)
//   O3: doli_fork_choice_table_missing_total (moves only on a refused reorg)
//   PATHS: P1 re-delivery of a rollback-removed D with a cached bond-heavier rival
//   INPUT PARTITIONS: rival R1..R3 (2 producers) vs D (1) | + ZERO-parent orphan by a
//     no-bond key (epoch 0 table pruned) | + equal-slot child of R3 by a no-bond
//     key | neither (control)
//   MATRIX: control x O1 O2 · orphan x O1 O2 -> o1a · orphan x O3 -> o1b ·
//     equal-slot x O1 O2 -> o1c
//! covers: bins/node/src/node/epoch_tables.rs branch_rank.rs fork_cache.rs

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::{Hash, KeyPair};
use doli_core::Block;
use doli_node::metrics::FORK_CHOICE_TABLE_MISSING;
use inc_i_289_common::*;

/// The metric is process-wide: one test at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

enum Junk {
    None,
    ZeroParentOrphan,
    EqualSlotLeaf,
}

/// Tip P = ours[1] (F+2) after a depth-1 rollback removed D = ours[2]. Cached: the
/// rival R1,R2,R3 on P by the two producers that are not D's (bond 2 > D's 1), and
/// `junk` by a key with no bond. Epoch 0's table is pruned, as on a node past the window.
async fn hold_rig(junk: Junk) -> (Rig, Block, Hash) {
    let mut r = rig(&[210, 211, 212]).await;
    let (p, d) = (r.ours[1].clone(), r.ours[2].clone());
    rollback_depth_one(&mut r).await;
    assert_on(&r, &p, F + 2, "precondition: tip P after the rollback").await;
    assert!(removed(&r, &d.hash()).await, "precondition: D is removed");
    let bpe = r.node.config.network.blocks_per_reward_epoch();
    assert!(F + 2 >= bpe, "fixture: the tip is past epoch 0 (bpe {bpe})");
    r.node.state_db.delete_epoch_table(0);
    let ks: Vec<KeyPair> = (r.keys.iter())
        .filter(|k| *k.public_key() != d.header.producer)
        .cloned()
        .collect();
    let params = r.node.params.clone();
    let r1 = block(F + 3, 213, p.hash(), &ks[0], &params);
    let r2 = block(F + 4, 214, r1.hash(), &ks[1], &params);
    let r3 = block(F + 5, 215, r2.hash(), &ks[0], &params);
    for (i, b) in [&r1, &r2, &r3].into_iter().enumerate() {
        cache_at_production_weight(&r, b, F + 3 + i as u64).await;
    }
    let nobody = KeyPair::generate();
    let junk = match junk {
        Junk::None => None,
        Junk::ZeroParentOrphan => Some(block(1, 1_000, Hash::ZERO, &nobody, &params)),
        Junk::EqualSlotLeaf => Some(block(F + 6, 215, r3.hash(), &nobody, &params)),
    };
    if let Some(j) = junk {
        r.node.fork_block_cache.write().await.insert(j.hash(), j);
    }
    (r, d, p.hash())
}

/// D was not re-applied and the tip stays P.
async fn held(r: &Rig, d: &Block, p: &Hash) -> bool {
    let at = r.node.block_store.get_hash_by_height(F + 3).unwrap();
    assert_ne!(at, Some(d.hash()), "D re-applied: the hold did not fire");
    tip_hash(r).await == *p && at.is_none()
}

// O-1 control — Decision: a failure means the fixture is vacuous (the rival never holds D).
#[tokio::test]
async fn o1_control_the_bond_heavier_rival_holds_the_removed_block() {
    let _s = SERIAL.lock().await;
    let (mut r, d, p) = hold_rig(Junk::None).await;
    gossip(&mut r, &d).await;
    assert!(held(&r, &d, &p).await, "control: D held, tip stays P");
}

// O-1a INC-I-289 — Decision: a failure means one stake-free ZERO-parent orphan zeroes every cached branch's rank, so the INC-I-281 hold stops protecting the heavier rival.
#[tokio::test]
async fn o1a_zero_parent_orphan_does_not_zero_the_honest_branch() {
    let _s = SERIAL.lock().await;
    let (mut r, d, p) = hold_rig(Junk::ZeroParentOrphan).await;
    gossip(&mut r, &d).await;
    assert!(
        held(&r, &d, &p).await,
        "O-1a: D held, tip stays P (the honest rival keeps its bond rank)"
    );
}

// O-1b INC-I-289 — Decision: a failure means the ranking counts a refused reorg that never happened, so the operator metric is noise.
#[tokio::test]
async fn o1b_ranking_does_not_count_a_refusal() {
    let _s = SERIAL.lock().await;
    let (mut r, d, _) = hold_rig(Junk::ZeroParentOrphan).await;
    let m0 = FORK_CHOICE_TABLE_MISSING.get();
    gossip(&mut r, &d).await;
    assert_eq!(
        FORK_CHOICE_TABLE_MISSING.get() - m0,
        0,
        "O-1b: the ranking refused nothing, the refusal metric must not move"
    );
}

// O-1c INC-I-289 — Decision: a failure means a no-bond key that signs one equal-slot child under an honest cached tip returns the honest branch to rank 0.
#[tokio::test]
async fn o1c_equal_slot_any_key_leaf_does_not_zero_the_honest_branch() {
    let _s = SERIAL.lock().await;
    let (mut r, d, p) = hold_rig(Junk::EqualSlotLeaf).await;
    gossip(&mut r, &d).await;
    assert!(
        held(&r, &d, &p).await,
        "O-1c: D held, tip stays P (R1..R3 keep their bond under the stuck leaf)"
    );
}
