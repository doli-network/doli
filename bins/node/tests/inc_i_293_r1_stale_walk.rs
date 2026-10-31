//! INC-I-293 R1 (run 648): the INC-I-235 lock cell failed about 1 run in 5 at
//! 776e37a0 (`height after reorg=57 end=18 target=61`). Cause: the F2 orphan arm
//! starts a by-hash walk for [A, A1]; another path adopts the same branch first;
//! the walk then completes with a tip that is ALREADY canonical. Fork choice
//! weighs our side [A1..tip] against the candidate [A1]: when the blocks above A1
//! add no new supporter the weights tie, the lower-hash rule picks A1, and the node
//! rolls back every canonical block above A1 to "apply" A1 again. In the lock
//! fixture that tie needs the node's own key to lead A1's slot and
//! hash(A1) < hash(tip): random keys -> about 1 run in 6. Fixed seeds here.
//!
// OUTPUT CONTRACT: fn Node::handle_completed_fork_recovery(CompletedRecovery)
//   O1: chain_state best_hash / best_height
//   O2: block_store canonical entry at every height F+1..=tip
//   O3: persisted chain state tip (state_db)
//   O4: sync manager local tip
//   O5: return value Ok(())
// PATHS: P1 walked tip already canonical (stale walk); P2 walked tip off our chain
//   and heavier (a real fork: must still reorg)
// INPUT PARTITIONS: X1 our chain R1..Rn with R2..Rn by ONE producer (tie on
//   support) and hash(R2) < hash(Rn) (the lower-hash rule picks R2); X2 our tip D
//   (1 producer) vs walked R1,R2,R3 (3 producers); at/above the INC-I-288 AH
//   (devnet 0) and below it (env override)
// MATRIX: A1 P1xX1 above AH O1-O5 (red at 776e37a0) · C1 P2xX2 above AH O1-O5 ·
//   B1 P1xX1 below AH O1-O5
//! covers: fork_walk.rs (handle_completed_fork_recovery), fork_recovery.rs
//! (evaluate_recovered_fork), reorg/doors.rs (check_reorg_supported tie)

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use doli_core::Block;
use inc_i_289_common::*;
use network::sync::CompletedRecovery;

/// Fixed seeds: every hash is a function of the keys and the slots.
fn keys() -> Vec<KeyPair> {
    (11..=13u8).map(|i| KeyPair::from_seed([i; 32])).collect()
}

/// X1: R1 at F+1 (slot 9), then R2..Rn at slots s, s+3, s+6, ... (one leader,
/// keys[1], not the node's own key), all applied. n is the first length >= 3
/// whose tip hashes above R2. Returns the rig and our chain [R1..Rn].
async fn x1() -> (Rig, Vec<Block>) {
    let mut r = rig_with_keys(keys(), &[9]).await;
    let s = (10u32..)
        .find(|s| leader(&r.keys, *s).public_key() == r.keys[1].public_key())
        .unwrap();
    let r1 = r.ours[0].clone();
    let chain = (3usize..40)
        .map(|n| {
            let slots: Vec<u32> = (0..n as u32).map(|i| s + 3 * i).collect();
            chain_on(&r, r1.hash(), F + 2, &slots)
        })
        .find(|c| c[0].hash() < c.last().unwrap().hash())
        .expect("fixture: a tip hashing above R2 within 40 blocks");
    for b in &chain {
        assert_eq!(
            b.header.producer,
            *r.keys[1].public_key(),
            "fixture: one producer"
        );
        r.node
            .apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: canonical block");
    }
    let mut ours = vec![r1];
    ours.extend(chain);
    r.ours = ours.clone();
    (r, ours)
}

/// The stale walk: [R1, R2] on F, both already canonical.
fn stale_walk(r: &Rig, ours: &[Block]) -> CompletedRecovery {
    CompletedRecovery {
        blocks: vec![ours[0].clone(), ours[1].clone()],
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    }
}

/// O1-O4: tip, height, persisted tip, sync tip on the top; every canonical entry kept.
async fn assert_chain(r: &Rig, chain: &[Block], label: &str) {
    let top_h = F + chain.len() as u64;
    assert_on(r, chain.last().unwrap(), top_h, label).await;
    for (i, b) in chain.iter().enumerate() {
        let h = F + 1 + i as u64;
        assert_eq!(
            r.node.block_store.get_hash_by_height(h).unwrap(),
            Some(b.hash()),
            "{label}: canonical entry at h={h}"
        );
    }
}

async fn stale_walk_keeps_the_chain(label: &str) {
    let (mut r, ours) = x1().await;
    let top_h = F + ours.len() as u64;
    assert_chain(&r, &ours, &format!("{label} precondition")).await;
    assert!(
        ours[1].hash() < ours.last().unwrap().hash(),
        "{label} precondition: hash(R2) < hash(tip), the lower-hash rule picks R2 on a tie"
    );
    let res = (r.node)
        .handle_completed_fork_recovery(stale_walk(&r, &ours))
        .await;
    let h = r.node.chain_state.read().await.best_height;
    assert!(res.is_ok(), "{label}: returns Ok: {res:?}");
    assert_eq!(
        h, top_h,
        "{label}: a walk whose tip is already canonical rolled the chain back from h={top_h} to h={h}"
    );
    assert_chain(&r, &ours, label).await;
}

// A1 — Decision: a failure means a completed walk whose tip is already on our
// canonical chain is weighed as a fork: on a support tie the lower-hash rule
// rolls back every canonical block above it (the INC-I-235 lock flake).
#[tokio::test]
async fn a1_stale_walk_of_a_canonical_branch_keeps_the_chain_above_gate() {
    stale_walk_keeps_the_chain("A1 above gate").await;
}

// B1 — Decision: a failure means the below-gate door changed for a stale walk
// (the guard leaked below the INC-I-288 gate).
#[tokio::test]
async fn b1_stale_walk_of_a_canonical_branch_below_gate_is_unchanged() {
    if !in_below_ah_child("b1_stale_walk_of_a_canonical_branch_below_gate_is_unchanged") {
        return;
    }
    stale_walk_keeps_the_chain("B1 below gate").await;
}

// C1 — Decision: a failure means the guard also refuses a REAL fork: a walked
// branch off our chain that out-supports our tip must still reorg (above gate).
#[tokio::test]
async fn c1_walk_of_a_heavier_branch_off_our_chain_still_reorgs_above_gate() {
    let mut r = rig_with_keys(keys(), &[10]).await;
    let walked = chain_on(&r, r.f_hash, F + 1, &[20, 21, 22]);
    let done = CompletedRecovery {
        blocks: walked.clone(),
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    };
    let res = r.node.handle_completed_fork_recovery(done).await;
    assert!(res.is_ok(), "C1: returns Ok: {res:?}");
    assert_chain(&r, &walked, "C1: the heavier walked branch is adopted").await;
}
