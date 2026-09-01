//! INC-I-204 M4.1 — shared fixture for the `forceReorgTo` operator escape.
//!
//! OUTPUT CONTRACT: N/A — fixture file. It declares no `#[test]`; the enumerations
//! live with the functions under test in the sibling `inc_i_204_m41_*` modules.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! `wedged_node()` rebuilds the RECORDED incident cell, and it does so by reusing
//! the construction that `tied_fork_finality.rs` already proves wedges
//! (`test_inc_i190_depth0_finality_lock_would_wedge_tied_fork`, IP5): a finalized
//! sibling A at `tip == finality == 6`, a strictly-longer competing branch
//! B/D1/D2 the node HOLDS and refuses, gap 2 (`0 < gap < 50`), and no rung left.
//! Every M4.1 rescue assertion is therefore a strict delta over a wedge this repo
//! already demonstrates — a pass cannot be a fixture artifact.

#![allow(dead_code)] // each consumer uses a subset

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Attestation, Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

pub const WEDGE_TIP_HEIGHT: u64 = 6;

pub async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

pub fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

pub fn build_chain(
    start_height: u64,
    start_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    count: usize,
    params: &ConsensusParams,
) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);
    let mut prev = prev_hash;
    for i in 0..count {
        let block = build_block(
            start_height + i as u64,
            start_slot + i as u32,
            prev,
            producer,
            params,
        );
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

pub async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at slot={}: {}", block.header.slot, e));
    }
}

/// The deterministically-scheduled slot leader: producers sorted by pubkey bytes,
/// indexed `slot % len`. A fork block by anyone else is dropped by the eligibility
/// gate (`fork_recovery.rs:573-575`), so every ADMITTED fixture block uses this.
pub fn leader(producers: &[KeyPair], slot: u32) -> &KeyPair {
    let mut scheduled: Vec<&KeyPair> = producers.iter().collect();
    scheduled.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    scheduled[(slot as usize) % scheduled.len()]
}

/// A genuine ProducerSet member that is NOT the scheduled leader for `slot`.
/// Bonded (so corroboration by weight can still pass) but unscheduled (so the
/// eligibility gate must refuse it).
pub fn unscheduled(producers: &[KeyPair], slot: u32) -> &KeyPair {
    let mut scheduled: Vec<&KeyPair> = producers.iter().collect();
    scheduled.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    let n = scheduled.len();
    scheduled[((slot as usize) + 1) % n]
}

/// Real finality ingress: every genuine ProducerSet member attests `block`.
pub async fn attest_all_producers_for(
    node: &mut Node,
    producers: &[KeyPair],
    block: &Block,
    height: u64,
) {
    let peer = network::PeerId::random();
    for p in producers {
        let att = Attestation::new(
            block.hash(),
            block.header.slot,
            height,
            1, // self-declared weight is IGNORED — authority comes from the ProducerSet
            p.private_key(),
            *p.public_key(),
        );
        assert!(att.verify().is_ok(), "fixture attestation must self-verify");
        node.on_new_attestation(att.to_bytes(), peer).await;
    }
}

/// The recorded wedge, ready for an escape attempt.
pub struct Wedge {
    pub node: Node,
    pub producers: Vec<KeyPair>,
    pub params: ConsensusParams,
    /// h=5, the common ancestor of both branches — one below the finality marker.
    pub base_tip: Hash,
    /// The finalized canonical sibling at h=6. The LOSING branch after a rescue.
    pub block_a: Block,
    /// The competing branch the node holds and refuses: h=6, h=7, h=8.
    pub branch: Vec<Block>,
    pub tmp: TempDir,
}

/// Build the wedge WITHOUT feeding the competing branch. `feed_branch` decides how
/// much of it the node receives, so the "branch still arriving" partitions are
/// reachable from the same fixture.
pub async fn wedged_node() -> Wedge {
    let (mut node, producers, tmp) = make_node(3).await;
    let params = node.params.clone();

    // Common prefix h=1..5.
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base).await;
    let base_tip = base[4].hash();

    // Canonical sibling A at h=6 (lower slot) becomes the local tip.
    let block_a = build_block(6, 100, base_tip, &producers[0], &params);
    apply_chain(&mut node, std::slice::from_ref(&block_a)).await;
    assert_eq!(node.chain_state.read().await.best_hash, block_a.hash());

    // Real 100%-weight attestation, then a depth-2 descendant, finalizes A at h=6.
    attest_all_producers_for(&mut node, &producers, &block_a, 6).await;
    {
        let mut sync = node.sync_manager.write().await;
        sync.track_block_for_finality(crypto::hash::hash(&[0xD2, 0xAA]), 8, 202, 3);
    }
    assert_eq!(
        node.sync_manager.read().await.last_finalized_height(),
        Some(WEDGE_TIP_HEIGHT),
        "fixture: A must be finalized at h=6"
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        WEDGE_TIP_HEIGHT,
        "fixture: tip == finality == 6 — the cell the escape exists for"
    );

    // The strictly-longer competing branch, every block by its scheduled leader.
    let b = build_block(6, 200, base_tip, leader(&producers, 200), &params);
    let d1 = build_block(7, 201, b.hash(), leader(&producers, 201), &params);
    let d2 = build_block(8, 202, d1.hash(), leader(&producers, 202), &params);

    Wedge {
        node,
        producers,
        params,
        base_tip,
        block_a,
        branch: vec![b, d1, d2],
        tmp,
    }
}

/// Deliver the first `n` blocks of the competing branch over the gossip path and
/// assert the node STILL refuses them — the wedge is real before any escape runs.
pub async fn feed_branch(w: &mut Wedge, n: usize) {
    let peer = network::PeerId::random();
    let blocks: Vec<Block> = w.branch.iter().take(n).cloned().collect();
    for block in blocks {
        w.node.handle_new_block(block, peer).await.unwrap();
    }
    assert_eq!(
        w.node.chain_state.read().await.best_hash,
        w.block_a.hash(),
        "fixture: the AUTOMATIC path must still be wedged on A after delivery — \
         if it escaped on its own, no M4.1 assertion below proves anything"
    );
}
