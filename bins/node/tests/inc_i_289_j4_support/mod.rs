//! QA-639-003 gate-edge fixtures. A devnet test node with four producers of bond 1
//! on the production state_db UTXO backend (so `Node::new` can reopen its data
//! dir), base h=1..=F by keys[0], then our branch applied on F. Every producer
//! weighs 1, so the per-block unit is a block count and the bond unit is the
//! count of DISTINCT producers. Copied from `it/inc_i_289_common.rs`,
//! `it/inc_i_289_n3_ranking_unit.rs` and `it/inc_i_289_k2_finality.rs`.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

pub mod scenarios;

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
/// Producers: keys[0] is the node's own key, keys[1..=3] build every branch.
pub const N: usize = 4;

/// Which side of the INC-I-288 gate the mechanism must take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// At/above the gate: bond of distinct supporters; finality persisted + restored.
    On,
    /// Below the gate: one weight per block; finality not persisted or not restored.
    Off,
}

pub struct Rig {
    pub node: Node,
    pub params: ConsensusParams,
    pub f_hash: Hash,
    /// `ours[i]` was applied at height `F + 1 + i`.
    pub ours: Vec<Block>,
    pub keys: Vec<KeyPair>,
    pub tmp: TempDir,
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

/// First slot at or after `from` whose leader is `k`.
fn slot_led_by(keys: &[KeyPair], k: &KeyPair, from: u32) -> u32 {
    (from..from + 30)
        .find(|s| leader(keys, *s).public_key() == k.public_key())
        .expect("fixture: every producer leads one slot in four")
}

/// A chain on `prev` from `height`: block i by `keys[by[i]]`, each at that
/// producer's next led slot after the previous block's slot (the first after `after`).
pub fn chain_by(r: &Rig, prev: Hash, height: u64, by: &[usize], after: u32) -> Vec<Block> {
    let (mut prev, mut slot) = (prev, after);
    by.iter()
        .enumerate()
        .map(|(i, k)| {
            slot = slot_led_by(&r.keys, &r.keys[*k], slot + 1);
            let b = block(height + i as u64, slot, prev, &r.keys[*k], &r.params);
            prev = b.hash();
            b
        })
        .collect()
}

/// Base h=1..=F, then our branch on F: block i by `keys[by[i]]`, applied.
pub async fn rig(by: &[usize]) -> Rig {
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..N).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    {
        let mut utxo = node.utxo_set.write().await;
        *utxo = storage::UtxoSet::from_state_db(node.state_db.clone());
    }
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let b = block(h, h as u32, prev, &keys[0], &params);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    let mut r = Rig {
        node,
        params,
        f_hash: prev,
        ours: Vec::new(),
        keys,
        tmp,
    };
    let ours = chain_by(&r, prev, F + 1, by, 209);
    for b in &ours {
        apply(&mut r, b).await;
    }
    r.ours = ours;
    if let Ok(v) = std::env::var(AH_ENV) {
        let want: u64 = v.parse().expect("fixture: numeric gate");
        assert_eq!(
            node_gate(&r),
            want,
            "fixture: the node runs with the child's gate"
        );
    }
    for h in 1..=(F + 10) {
        let ps = r.node.producer_set.read().await;
        for k in &r.keys {
            let w = (ps.get_by_pubkey(k.public_key())).map(|p| p.effective_weight(h));
            assert_eq!(w, Some(1), "fixture: every producer weighs 1 at h={h}");
        }
    }
    r
}

/// Raw apply (no intake path: no door, no hold, no drain).
pub async fn apply(r: &mut Rig, b: &Block) {
    r.node
        .apply_block(b.clone(), ValidationMode::Light)
        .await
        .unwrap_or_else(|e| panic!("fixture: block at slot {} must apply: {e}", b.header.slot));
}

/// Drop the node and reopen the same data dir through the restoring constructor.
pub async fn reopen(r: Rig) -> Rig {
    let Rig {
        node,
        params,
        f_hash,
        ours,
        keys,
        tmp,
    } = r;
    let config = node.config.clone();
    drop(node);
    let node = Node::new(config, Some(keys[0].clone()), None, None, None, None)
        .await
        .expect("Node::new must reopen the data dir");
    Rig {
        node,
        params,
        f_hash,
        ours,
        keys,
        tmp,
    }
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

pub async fn gossip(r: &mut Rig, b: &Block) {
    r.node
        .handle_new_block(b.clone(), PeerId::random())
        .await
        .expect("handle_new_block returns Ok");
}

pub async fn tick(r: &mut Rig) {
    r.node
        .run_periodic_tasks()
        .await
        .expect("run_periodic_tasks returns Ok");
}

/// (best_hash, best_height, best_slot).
pub async fn tip(r: &Rig) -> (Hash, u64, u32) {
    let cs = r.node.chain_state.read().await;
    (cs.best_hash, cs.best_height, cs.best_slot)
}

/// Ticks until the tip is `want` or `max` ticks ran.
pub async fn settle(r: &mut Rig, want: &Hash, max: usize) {
    for i in 0..=max {
        if tip(r).await.0 == *want || i == max {
            return;
        }
        tick(r).await;
    }
}

pub async fn peer_at(r: &Rig, height: u64, tip: Hash, slot: u32) {
    (r.node.sync_manager.write().await).add_peer(PeerId::random(), height, tip, slot);
}

pub async fn removed(r: &Rig, h: &Hash) -> bool {
    r.node.sync_manager.read().await.removed_by_rollback(h)
}

pub async fn cached(r: &Rig, h: &Hash) -> bool {
    r.node.fork_block_cache.read().await.contains_key(h)
}

pub async fn recorded(r: &Rig, h: &Hash) -> bool {
    (r.node.sync_manager.read().await)
        .reorg_handler()
        .get_block_weight(h)
        .is_some()
}

pub fn stored(r: &Rig, b: &Block) -> bool {
    r.node.block_store.has_block(&b.hash()).unwrap()
}

pub fn canonical(r: &Rig, height: u64) -> Option<Hash> {
    r.node.block_store.get_hash_by_height(height).unwrap()
}

/// Cached with `weight` recorded in the reorg handler (as fork choice holds a block).
pub async fn cache_weighed(r: &Rig, b: &Block, height: u64, weight: u64) {
    (r.node.fork_block_cache.write().await).insert(b.hash(), b.clone());
    r.node.sync_manager.write().await.record_fork_block_weight(
        b.hash(),
        b.header.prev_hash,
        weight,
        height,
    );
}

/// Tip hash, height, canonical entry, persisted tip and sync tip all name `b`.
pub async fn assert_on(r: &Rig, b: &Block, height: u64, label: &str) {
    let (best_hash, best_height, _) = tip(r).await;
    assert_eq!(best_hash, b.hash(), "{label}: tip hash (h={best_height})");
    assert_eq!(best_height, height, "{label}: tip height");
    assert_eq!(canonical(r, height), Some(b.hash()), "{label}: canonical");
    let persisted = r.node.state_db.get_chain_state().map(|c| c.best_hash);
    assert_eq!(persisted, Some(b.hash()), "{label}: persisted tip");
    let sync_h = r.node.sync_manager.read().await.local_tip().0;
    assert_eq!(sync_h, height, "{label}: sync manager tip");
}

/// The local tip the gated site compares is `height` (applied tip = sync tip).
pub async fn assert_decision_tip(r: &Rig, height: u64, label: &str) {
    assert_eq!(tip(r).await.1, height, "{label}: decision tip height");
    let sync_h = r.node.sync_manager.read().await.local_tip().0;
    assert_eq!(sync_h, height, "{label}: sync manager tip at the decision");
}

/// (per-block unit, bond unit of the DISTINCT producers) of `blocks` at `height`.
pub async fn units(r: &Rig, blocks: &[Block], height: u64) -> (u64, u64) {
    let audit = NetworkParams::load(Network::Devnet).security_audit_activation_height;
    let ps = r.node.producer_set.read().await;
    let mut seen = Vec::new();
    let (mut per_block, mut bond) = (0u64, 0u64);
    for b in blocks {
        let pi = (ps.get_by_pubkey(&b.header.producer)).expect("fixture: producer registered");
        per_block += pi.effective_weight(height);
        if !seen.contains(&b.header.producer) {
            seen.push(b.header.producer);
            bond += pi.selection_weight_at(height, audit);
        }
    }
    (per_block, bond)
}

/// `a` has more blocks and `b` more distinct bond: the two rules disagree.
pub async fn assert_rules_disagree(r: &Rig, a: &[Block], b: &[Block], h: u64, label: &str) {
    let (a_blocks, a_bond) = units(r, a, h).await;
    let (b_blocks, b_bond) = units(r, b, h).await;
    assert!(
        a_blocks > b_blocks && b_bond > a_bond,
        "{label} precondition: blocks {a_blocks} > {b_blocks} and bond {b_bond} > {a_bond}"
    );
}

pub const AH_ENV: &str = "DOLI_INC_I_288_FORK_CHOICE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_289_J4_GATE_CHILD";

/// Parent: re-exec test `name` alone with the INC-I-288 gate at `gate`; returns
/// true in the child only (which checks that the node's params carry `gate`).
pub fn in_gate_child(name: &str, gate: u64) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        let got = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
        assert_eq!(got, gate, "child must pick up the gate override");
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(AH_ENV, gate.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let panic_line = (stderr.lines())
        .skip_while(|l| !l.contains("panicked at"))
        .take(4)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "gate={gate} child failed or did not run:\n{panic_line}\n--- stdout tail ---\n{}",
        stdout.lines().rev().take(6).collect::<Vec<_>>().join("\n")
    );
    false
}

/// The gate the node itself read (config path, not only the env).
pub fn node_gate(r: &Rig) -> u64 {
    r.node
        .config
        .network
        .params()
        .inc_i_288_fork_choice_activation_height
}
