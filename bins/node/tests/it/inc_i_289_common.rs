//! INC-I-289 fixtures (self-contained: the same file compiles at 6cd0aba6, where
//! the INC-I-288 fixtures and the INC-I-288 gate field do not exist). A devnet
//! test node on base h=1..=F, then the node's own blocks applied on F. Every
//! producer weighs 1 at these heights, so a branch weight is its block count.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network, NetworkParams, Transaction};
use doli_node::node::{Node, RollbackAuthority, RollbackOutcome};
use network::PeerId;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// Fork point: the last base block.
pub const F: u64 = 5;

pub struct Rig {
    pub node: Node,
    pub f_hash: Hash,
    /// Applied: `ours[i]` is at height `F + 1 + i`.
    pub ours: Vec<Block>,
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

/// A chain on `prev` from `height`, one block per slot, each by its slot leader.
pub fn chain_on(r: &Rig, prev: Hash, height: u64, slots: &[u32]) -> Vec<Block> {
    let mut prev = prev;
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let b = block(
                height + i as u64,
                *s,
                prev,
                leader(&r.keys, *s),
                &r.node.params,
            );
            prev = b.hash();
            b
        })
        .collect()
}

/// Base h=1..=F, then `our_slots` applied on F (`Node::new_for_test` sync config).
pub async fn rig(our_slots: &[u32]) -> Rig {
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
    let mut r = Rig {
        node,
        f_hash: prev,
        ours: Vec::new(),
        keys,
        _tmp: tmp,
    };
    let ours = chain_on(&r, prev, F + 1, our_slots);
    for b in &ours {
        r.node
            .apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: own block");
    }
    r.ours = ours;
    let top = r
        .ours
        .last()
        .expect("fixture: own branch is not empty")
        .clone();
    assert_on(&r, &top, F + r.ours.len() as u64, "fixture: tip = own top").await;
    for h in 1..=(F + 6) {
        let ps = r.node.producer_set.read().await;
        for k in &r.keys {
            let w = ps
                .get_by_pubkey(k.public_key())
                .map(|p| p.effective_weight(h));
            assert_eq!(w, Some(1), "fixture: every producer weighs 1 at h={h}");
        }
    }
    r
}

/// The coordinator's ShallowRollback { depth: 1 } executor call.
pub async fn rollback_depth_one(r: &mut Rig) {
    let out = r
        .node
        .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("rollback_one_block");
    assert_eq!(
        out,
        RollbackOutcome::RolledBack,
        "precondition: rolled back"
    );
}

/// A gossip block (the production intake path).
pub async fn gossip(r: &mut Rig, b: &Block) {
    r.node
        .handle_new_block(b.clone(), PeerId::random())
        .await
        .expect("handle_new_block returns Ok");
}

/// A peer whose status names (`height`, `tip`, `slot`).
pub async fn peer_at(r: &Rig, height: u64, tip: Hash, slot: u32) -> PeerId {
    let peer = PeerId::random();
    (r.node.sync_manager.write().await).add_peer(peer, height, tip, slot);
    peer
}

/// The node's periodic tick: evidence report, coordinator classify, executor.
pub async fn tick(r: &mut Rig) {
    r.node
        .run_periodic_tasks()
        .await
        .expect("run_periodic_tasks returns Ok");
}

/// One EmptyHeaders report from `peer`, at the gap the node sees now.
pub async fn empty_headers(r: &Rig, peer: PeerId) {
    let mut sync = r.node.sync_manager.write().await;
    let gap = sync.network_tip_height().saturating_sub(sync.local_tip().0);
    sync.report_empty_headers(peer, gap);
}

/// The coordinator's gap input: network_tip_height - local height.
pub async fn coordinator_gap(r: &Rig) -> u64 {
    let sync = r.node.sync_manager.read().await;
    sync.network_tip_height().saturating_sub(sync.local_tip().0)
}

pub async fn removed(r: &Rig, h: &Hash) -> bool {
    r.node.sync_manager.read().await.removed_by_rollback(h)
}

pub async fn cached(r: &Rig, h: &Hash) -> bool {
    r.node.fork_block_cache.read().await.contains_key(h)
}

/// (shallow_rollback_count, cumulative_rollback_depth).
pub fn rollbacks(r: &Rig) -> (u32, u32) {
    (
        r.node.shallow_rollback_count,
        r.node.cumulative_rollback_depth,
    )
}

pub async fn tip_hash(r: &Rig) -> Hash {
    r.node.chain_state.read().await.best_hash
}

/// Tip hash, height, canonical entry, persisted tip and sync tip all name `b`.
pub async fn assert_on(r: &Rig, b: &Block, height: u64, label: &str) {
    let (best_hash, best_height) = {
        let cs = r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    assert_eq!(best_hash, b.hash(), "{label}: tip hash (h={best_height})");
    assert_eq!(best_height, height, "{label}: tip height");
    assert_eq!(
        r.node.block_store.get_hash_by_height(height).unwrap(),
        Some(b.hash()),
        "{label}: canonical entry at h={height}"
    );
    let persisted = r.node.state_db.get_chain_state().map(|c| c.best_hash);
    assert_eq!(persisted, Some(b.hash()), "{label}: persisted tip");
    let sync_h = r.node.sync_manager.read().await.local_tip().0;
    assert_eq!(sync_h, height, "{label}: sync manager tip");
}

/// Ticks until the tip is `want` or `max` ticks ran; returns the ticks used.
pub async fn tick_until(r: &mut Rig, want: &Hash, max: usize) -> usize {
    for i in 0..=max {
        if tip_hash(r).await == *want {
            return i;
        }
        if i < max {
            tick(r).await;
        }
    }
    max
}

/// (weight recorded in the reorg handler, body cached, in the retained set).
pub async fn weighed(r: &Rig, b: &Block) -> (bool, bool, bool) {
    let h = b.hash();
    let recorded = (r.node.sync_manager.read().await)
        .reorg_handler()
        .get_block_weight(&h)
        .is_some();
    let cached = r.node.fork_block_cache.read().await.contains_key(&h);
    (recorded, cached, r.node.wedge_retained_tips.contains(&h))
}

/// The block store holds `b` (it was applied at least once).
pub fn stored(r: &Rig, b: &Block) -> bool {
    r.node.block_store.has_block(&b.hash()).unwrap()
}

/// A block as fork choice holds it after a walk or a door: body cached, weight
/// recorded at the production weight (1 here).
pub async fn cache_at_production_weight(r: &Rig, b: &Block, height: u64) {
    (r.node.fork_block_cache.write().await).insert(b.hash(), b.clone());
    r.node.sync_manager.write().await.record_fork_block_weight(
        b.hash(),
        b.header.prev_hash,
        1,
        height,
    );
}

/// The first slot after `after` whose leader's block at `height` on `parent`
/// hashes above `d`.
pub fn higher_sibling(r: &Rig, d: &Block, parent: Hash, height: u64, after: u32) -> Block {
    let p = r.node.params.clone();
    (after + 1..after + 200)
        .map(|s| block(height, s, parent, leader(&r.keys, s), &p))
        .find(|b| b.hash().as_bytes() > d.hash().as_bytes())
        .expect("fixture: a sibling hashing above D within 200 slots")
}

pub const AH_ENV: &str = "DOLI_INC_I_288_FORK_CHOICE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_289_BELOW_AH_CHILD";
pub const BELOW_AH: u64 = 1_000_000_000;
const AH_FIELD: &str = "inc_i_288_fork_choice_activation_height";

/// Parent: re-exec `name` alone with the INC-I-288 AH far above every test
/// height; returns true in the child. Where the gate field does not exist
/// (6cd0aba6) the child runs the only rule there is.
pub fn in_below_ah_child(name: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        let dbg = format!("{:?}", NetworkParams::load(Network::Devnet));
        assert!(
            !dbg.contains(AH_FIELD) || dbg.contains(&format!("{AH_FIELD}: {BELOW_AH}")),
            "child must pick up the AH env override"
        );
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(AH_ENV, BELOW_AH.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "below-AH child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}
