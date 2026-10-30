//! INC-I-288 C-1 fixtures: a devnet test node on base h=1..=F, then the node's own
//! branch "ours" (applied) and a competing branch "theirs" on the same parent F
//! (built, never applied). Every producer weighs 1 at these heights, so a branch
//! weight is its block count.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::{Node, RollbackAuthority, RollbackOutcome};
use network::PeerId;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// Fork point: the last block both branches share.
pub const F: u64 = 5;

pub struct Rig {
    pub node: Node,
    pub f_hash: Hash,
    /// Applied: `ours[i]` is at height `F + 1 + i`.
    pub ours: Vec<Block>,
    /// Built on F, not applied: `theirs[i]` is at height `F + 1 + i`.
    pub theirs: Vec<Block>,
    /// The 3 producers; `keys[0]` is the node's own producer key.
    pub keys: Vec<KeyPair>,
    _tmp: TempDir,
}

pub fn leader(keys: &[KeyPair], slot: u32) -> &KeyPair {
    let mut scheduled: Vec<&KeyPair> = keys.iter().collect();
    scheduled.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    scheduled[(slot as usize) % scheduled.len()]
}

pub fn block(height: u64, slot: u32, prev: Hash, by: &KeyPair, p: &ConsensusParams) -> Block {
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(p.block_reward(height), pool, height, 0);
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase)),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: p.genesis_time + slot as u64 * p.slot_duration,
        slot,
        producer: *by.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    crate::block_sig::sign_header(&mut header, by);
    Block::new(header, vec![coinbase])
}

fn branch(p: &ConsensusParams, keys: &[KeyPair], prev: Hash, slots: &[u32]) -> Vec<Block> {
    let mut out = Vec::with_capacity(slots.len());
    let mut prev = prev;
    for (i, slot) in slots.iter().enumerate() {
        let b = block(F + 1 + i as u64, *slot, prev, leader(keys, *slot), p);
        prev = b.hash();
        out.push(b);
    }
    out
}

/// Base h=1..=F, then `our_slots` applied on F; `their_slots` built on F.
pub async fn rig(our_slots: &[u32], their_slots: &[u32]) -> Rig {
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    let p = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let b = block(h, h as u32, prev, &keys[0], &p);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    let ours = branch(&p, &keys, prev, our_slots);
    let theirs = branch(&p, &keys, prev, their_slots);
    for b in &ours {
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: own block");
    }
    let r = Rig {
        node,
        f_hash: prev,
        ours,
        theirs,
        keys,
        _tmp: tmp,
    };
    let top = r.ours.last().expect("fixture: own branch is not empty");
    assert_on(
        &r,
        top,
        F + r.ours.len() as u64,
        "fixture: tip = own branch top",
    )
    .await;
    for h in 1..=(F + 1 + our_slots.len().max(their_slots.len()) as u64) {
        assert_eq!(
            unit_weight(&r, h).await,
            1,
            "fixture: every producer weighs 1 at h={h}"
        );
    }
    r
}

/// Every producer's effective weight at `h`, when they all agree (else 0).
async fn unit_weight(r: &Rig, h: u64) -> u64 {
    let ps = r.node.producer_set.read().await;
    let ws: Vec<u64> = r
        .ours
        .iter()
        .chain(r.theirs.iter())
        .filter_map(|b| ps.get_by_pubkey(&b.header.producer))
        .map(|p| p.effective_weight(h))
        .collect();
    match ws.first() {
        Some(w) if ws.iter().all(|x| x == w) => *w,
        _ => 0,
    }
}

/// The coordinator's ShallowRollback { depth: 1 } (ends in reset_sync_for_rollback).
pub async fn rollback_depth_one(r: &mut Rig) {
    let out = r
        .node
        .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("rollback_one_block");
    assert_eq!(
        out,
        RollbackOutcome::RolledBack,
        "precondition: one block rolled back"
    );
}

pub async fn gossip(r: &mut Rig, b: &Block) {
    r.node
        .handle_new_block(b.clone(), PeerId::random())
        .await
        .expect("handle_new_block returns Ok");
}

/// Every chain-tip output a reorg moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tip {
    pub best_hash: Hash,
    pub best_height: u64,
    pub canonical_f1: Option<Hash>,
    pub persisted: Option<Hash>,
    pub sync_height: u64,
}

pub async fn tip(r: &Rig) -> Tip {
    let (best_hash, best_height) = {
        let cs = r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    Tip {
        best_hash,
        best_height,
        canonical_f1: r.node.block_store.get_hash_by_height(F + 1).unwrap(),
        persisted: r.node.state_db.get_chain_state().map(|c| c.best_hash),
        sync_height: r.node.sync_manager.read().await.local_tip().0,
    }
}

/// Tip, canonical index, persisted tip and sync tip all name `b` at `height`.
pub async fn assert_on(r: &Rig, b: &Block, height: u64, label: &str) {
    let t = tip(r).await;
    assert_eq!(t.best_hash, b.hash(), "{label}: tip hash ({t:?})");
    assert_eq!(t.best_height, height, "{label}: tip height");
    assert_eq!(
        r.node.block_store.get_hash_by_height(height).unwrap(),
        Some(b.hash()),
        "{label}: canonical entry at h={height}"
    );
    assert_eq!(t.persisted, Some(b.hash()), "{label}: persisted tip");
    assert_eq!(t.sync_height, height, "{label}: sync manager tip");
}

pub async fn chain_weight(r: &Rig, h: &Hash) -> u64 {
    r.node
        .sync_manager
        .read()
        .await
        .reorg_handler()
        .chain_weight(h)
}

pub async fn current_weight(r: &Rig) -> u64 {
    r.node
        .sync_manager
        .read()
        .await
        .reorg_handler()
        .current_weight()
}

pub async fn parent_link(r: &Rig, h: &Hash) -> Option<Hash> {
    r.node
        .sync_manager
        .read()
        .await
        .reorg_handler()
        .get_parent(h)
}

/// The weight door ran on `b`: its weight is recorded, its body cached, and it
/// is in the eligibility-validated retained set.
pub async fn weighed(r: &Rig, b: &Block) -> (bool, bool, bool) {
    let h = b.hash();
    let recorded = r
        .node
        .sync_manager
        .read()
        .await
        .reorg_handler()
        .get_block_weight(&h)
        .is_some();
    let cached = r.node.fork_block_cache.read().await.contains_key(&h);
    (recorded, cached, r.node.wedge_retained_tips.contains(&h))
}

pub async fn stuck_fork_raised(r: &Rig) -> bool {
    r.node.sync_manager.write().await.take_stuck_fork_signal()
}
