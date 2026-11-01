//! INC-I-293 S-1 (run 649) d2: the gossip ReorgCandidate caller of the check door
//! with a candidate that is already on our chain, in the run-648 tie shape (R2..Rn
//! by one producer, hash(R2) < hash(Rn)). Today this caller is fenced before the
//! door: `process_block` -> `intake_body` returns `None` for a block with a
//! canonical height entry (`body_commitments.rs:88-90`), and the event loop drops
//! it upstream (`network_events.rs:107-115`). This cell pins that the gossip path
//! never rolls back for it, with or without the fence once the door rule lands.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(Block, PeerId) -> Result<()>
//   O1: chain_state best_hash / best_height
//   O2: block_store canonical entry at every height F+1..=tip
//   O3: return value Ok(())
// PATHS: P1 the gossiped block is canonical (a prefix of our chain)
// INPUT PARTITIONS: X1 R1 (keys[0]) then R2..Rn (keys[1]) applied, hash(R2) < hash(Rn),
//   above the INC-I-288 AH (devnet 0)
// MATRIX: d2 P1xX1 O1-O3 (pin: passes today, held by `intake_body`)

use super::inc_i_289_common::*;
use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use doli_core::Block;

/// Fixed seeds: every hash is a function of the keys and the slots.
fn keys() -> Vec<KeyPair> {
    (11..=13u8).map(|i| KeyPair::from_seed([i; 32])).collect()
}

/// R1 at F+1 (slot 9), then R2..Rn by keys[1] (not the node's own key) applied,
/// n the first length >= 3 whose tip hashes above R2. Returns the rig and [R1..Rn].
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

async fn assert_chain(r: &Rig, chain: &[Block], label: &str) {
    assert_on(r, chain.last().unwrap(), F + chain.len() as u64, label).await;
    for (i, b) in chain.iter().enumerate() {
        let h = F + 1 + i as u64;
        assert_eq!(
            r.node.block_store.get_hash_by_height(h).unwrap(),
            Some(b.hash()),
            "{label}: canonical entry at h={h}"
        );
    }
}

// d2 — Decision: a failure means a gossiped block already on our chain reaches the
// ReorgCandidate door and the tie rule rolls back every canonical block above it.
#[tokio::test]
async fn d2_gossip_of_a_canonical_prefix_block_keeps_the_chain_above_gate() {
    let (mut r, ours) = x1().await;
    assert_chain(&r, &ours, "d2 precondition").await;
    assert!(
        ours[1].hash() < ours.last().unwrap().hash(),
        "d2 precondition: hash(R2) < hash(tip), the lower-hash rule picks R2 on a tie"
    );
    for (i, b) in ours.iter().take(2).enumerate() {
        gossip(&mut r, b).await;
        assert_chain(&r, &ours, &format!("d2 after gossip of R{}", i + 1)).await;
    }
}
