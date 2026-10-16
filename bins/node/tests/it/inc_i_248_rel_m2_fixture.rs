//! INC-I-248 REL-M2 — shared fixture: block-store layouts for the start-time provenance check.
//!
//! OUTPUT CONTRACT: N/A — fixture module. Its asserts are fixture preconditions only.
//! INPUT PARTITIONS: N/A — fixture module.
// covers: bins/node/src/node/maintainer_sanitize.rs, bins/node/src/run.rs (start-time sanitize)

#![allow(dead_code)]

use std::ops::RangeInclusive;
use std::sync::Arc;

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m1_fixture as fx;
use crypto::Hash;
use doli_core::maintainer::{quorum_eligible, LocalRootView};
use doli_core::{Block, BlockHeader, MaintainerSet, Transaction};
use doli_node::node::maintainer_quorum::ProbeReason;
use doli_node::node::Node;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

pub const S: u64 = h::SNAP_HEIGHT;
pub const TIP: u64 = S + 5;

/// Coinbase-only block at `height`; the hash is distinct per height.
pub fn stub_block(height: u64, prev_hash: Hash) -> Block {
    let coinbase = Transaction::new_coinbase(
        1,
        doli_core::consensus::reward_pool_pubkey_hash(),
        height,
        height as u32,
    );
    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase)),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: height * 10,
        slot: height as u32,
        producer: h::pk(1),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![coinbase])
}

/// `stub_block` with `tx` appended after the coinbase.
pub fn stub_block_with(height: u64, prev_hash: Hash, tx: Transaction) -> Block {
    let stub = stub_block(height, prev_hash);
    let mut txs = stub.transactions.clone();
    txs.push(tx);
    let mut header = stub.header.clone();
    header.merkle_root = doli_core::block::compute_merkle_root(&txs);
    Block::new(header, txs)
}

/// Stored block at `height` holds an AddMaintainer or RemoveMaintainer tx.
pub fn block_has_maintainer_change(node: &Node, height: u64) -> bool {
    node.block_store
        .get_block_by_height(height)
        .expect("fixture: block read")
        .expect("fixture: block stored")
        .transactions
        .iter()
        .any(|tx| tx.is_maintainer_change())
}

/// Store a canonical block at every height in `ranges` (in order).
pub fn put_heights(node: &Node, ranges: &[RangeInclusive<u64>]) {
    put_heights_with(node, ranges, stub_block);
}

/// `put_heights` with the block at each height built by `block_at(height, prev_hash)`.
pub fn put_heights_with(
    node: &Node,
    ranges: &[RangeInclusive<u64>],
    block_at: impl Fn(u64, Hash) -> Block,
) {
    let mut prev = Hash::ZERO;
    for range in ranges {
        for height in range.clone() {
            let block = block_at(height, prev);
            prev = block.hash();
            node.block_store
                .put_block_canonical(&block, height)
                .expect("fixture: put_block_canonical");
        }
    }
}

pub fn has_block(node: &Node, height: u64) -> bool {
    node.block_store
        .get_hash_by_height(height)
        .expect("fixture: height index read")
        .is_some()
}

/// Chain tip = `tip`, best hash = the stored block at `tip`.
pub async fn set_tip_to_store(node: &Node, tip: u64) {
    let hash = node
        .block_store
        .get_hash_by_height(tip)
        .expect("fixture: height index read")
        .expect("fixture: a block must be stored at the tip");
    let mut cs = node.chain_state.write().await;
    cs.best_height = tip;
    cs.best_hash = hash;
}

/// Replace the node's maintainer state with `st`, saved to disk and loaded back.
pub fn install_state(node: &mut Node, st: &MaintainerState) {
    st.save(&node.config.data_dir)
        .expect("fixture: maintainer state must persist");
    let loaded = MaintainerState::load(&node.config.data_dir).expect("fixture: state must load");
    node.set_maintainer_state(Arc::new(RwLock::new(loaded)));
}

/// The five producer keys as a well-formed set stamped at `at`.
pub fn producer_set_at(at: u64) -> MaintainerSet {
    fx::well_formed(
        h::producer_keys().iter().map(|k| *k.public_key()).collect(),
        at,
    )
}

pub fn assert_retired_check_off(node: &Node) {
    assert_eq!(
        node.params.retired_genesis_maintainer_keys_height,
        u64::MAX,
        "fixture: the retired-key check must not fire in these tests"
    );
}

/// Testnet node with an empty store and `st` installed.
pub async fn node_with(st: &MaintainerState) -> (Node, TempDir) {
    let (mut node, temp) = h::testnet_node(&h::producer_keys()).await;
    assert_retired_check_off(&node);
    install_state(&mut node, st);
    (node, temp)
}

/// Pre-fix snap data dir: producer seed at h=1 (real seed path, read back from disk),
/// store = block 1 + blocks S+1..=TIP, chain tip TIP.
pub async fn stale_snap_node() -> (Node, TempDir) {
    let (mut node, temp) = h::testnet_node(&h::producer_keys()).await;
    assert_retired_check_off(&node);
    node.maybe_bootstrap_maintainer_set(1).await;
    let disk = h::disk_state_of(&node);
    assert!(
        h::is_producer_seed(&disk.set),
        "fixture: producer seed on disk"
    );
    assert_eq!(disk.last_derived_height, 1, "fixture: seed stamped at 1");
    install_state(&mut node, &disk);
    put_heights(&node, &[1..=1, S + 1..=TIP]);
    set_tip_to_store(&node, TIP).await;
    assert!(
        has_block(&node, 1)
            && !has_block(&node, 2)
            && !has_block(&node, S)
            && has_block(&node, S + 1)
            && has_block(&node, TIP)
            && !has_block(&node, TIP + 1),
        "fixture: store = block 1 + S+1..=TIP"
    );
    (node, temp)
}

/// REL-M1 eligibility of the node's current root.
pub async fn eligible(node: &Node) -> bool {
    let st = h::state_of(node).await;
    quorum_eligible(&LocalRootView {
        digest: (!st.set.members.is_empty()).then(|| h::node_digest(node, &st.set)),
        last_derived_height: st.last_derived_height,
    })
}

/// The start-time check keeps the root: false, memory + disk + file bytes unchanged,
/// and a non-empty root stays ineligible (the startup probe does not arm).
pub async fn assert_start_keeps(node: &Node, ctx: &str) {
    let before = h::state_of(node).await;
    let bytes = fx::file_bytes(node);
    assert!(
        bytes.is_some(),
        "{ctx}: fixture: maintainer_state.bin present"
    );
    assert!(
        !node.sanitize_retired_maintainer_set().await,
        "{ctx}: the start-time check reset the root"
    );
    fx::assert_state(node, &before.set, before.last_derived_height, ctx).await;
    assert_eq!(
        fx::file_bytes(node),
        bytes,
        "{ctx}: maintainer_state.bin rewritten"
    );
    if !before.set.members.is_empty() {
        assert!(!eligible(node).await, "{ctx}: root became eligible");
        node.arm_maintainer_quorum(ProbeReason::Startup).await;
        assert_eq!(
            node.maintainer_probe_phase(),
            "idle",
            "{ctx}: the startup probe armed"
        );
    }
}
