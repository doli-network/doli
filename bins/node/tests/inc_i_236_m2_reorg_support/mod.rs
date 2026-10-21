//! INC-I-236 M2 reorg-door fixtures: a node at tip A (h=6) and a competing
//! branch B(h6) -> D1(h7) -> D2(h8), each named by its scheduled slot leader.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use network::sync::{CompletedRecovery, ReorgResult};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

pub const BRANCH_SLOTS: [u32; 3] = [200, 201, 202];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sig {
    Valid,
    Forged,
    Unsigned,
}

pub struct Fx {
    pub node: Node,
    pub producers: Vec<KeyPair>,
    pub params: ConsensusParams,
    pub base_tip: Hash,
    pub block_a: Block,
    pub _tmp: TempDir,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Snap {
    pub best_hash: Hash,
    pub best_height: u64,
    pub canonical_h6: Option<Hash>,
    pub persisted_tip: Option<Hash>,
}

pub fn block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    sig: Sig,
    params: &ConsensusParams,
) -> Block {
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(params.block_reward(height), pool, height, 0);
    let mut header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase)),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: params.genesis_time + slot as u64 * params.slot_duration,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    match sig {
        Sig::Valid => crate::block_sig::sign_header(&mut header, producer),
        Sig::Forged => crate::block_sig::sign_header(&mut header, &KeyPair::generate()),
        Sig::Unsigned => {}
    }
    Block::new(header, vec![coinbase])
}

pub fn leader(producers: &[KeyPair], slot: u32) -> &KeyPair {
    let mut scheduled: Vec<&KeyPair> = producers.iter().collect();
    scheduled.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    scheduled[(slot as usize) % scheduled.len()]
}

/// Base h=1..5 plus canonical sibling A (h=6, slot 100) applied; tip = A.
pub async fn fixture() -> Fx {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=5u64 {
        let b = block(h, h as u32, prev, &producers[0], Sig::Valid, &params);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block must apply");
    }
    let block_a = block(6, 100, prev, &producers[0], Sig::Valid, &params);
    node.apply_block(block_a.clone(), ValidationMode::Light)
        .await
        .expect("fixture: A must apply");
    assert_eq!(node.chain_state.read().await.best_hash, block_a.hash());
    Fx {
        node,
        producers,
        params,
        base_tip: prev,
        block_a,
        _tmp: temp,
    }
}

/// B(h6) -> D1(h7) -> D2(h8) on `base_tip`, each by its slot leader, signed per `sigs`.
pub fn branch(fx: &Fx, sigs: [Sig; 3]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(3);
    let mut prev = fx.base_tip;
    for (i, sig) in sigs.iter().enumerate() {
        let slot = BRANCH_SLOTS[i];
        let b = block(
            6 + i as u64,
            slot,
            prev,
            leader(&fx.producers, slot),
            *sig,
            &fx.params,
        );
        prev = b.hash();
        out.push(b);
    }
    out
}

pub async fn snap(node: &Node) -> Snap {
    let (best_hash, best_height) = {
        let cs = node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    Snap {
        best_hash,
        best_height,
        canonical_h6: node.block_store.get_hash_by_height(6).expect("store read"),
        persisted_tip: node.state_db.get_chain_state().map(|c| c.best_hash),
    }
}

pub async fn weight_recorded(node: &Node, hash: &Hash) -> bool {
    node.sync_manager
        .read()
        .await
        .reorg_handler()
        .get_block_weight(hash)
        .is_some()
}

pub async fn cached(node: &Node, hash: &Hash) -> Option<Block> {
    node.fork_block_cache.read().await.get(hash).cloned()
}

pub async fn stuck_fork_raised(node: &Node) -> bool {
    node.sync_manager.write().await.take_stuck_fork_signal()
}

pub async fn seed_cache(node: &Node, blocks: &[Block]) {
    let mut cache = node.fork_block_cache.write().await;
    for b in blocks {
        cache.insert(b.hash(), b.clone());
    }
}

/// Rollback A, apply `branch`; `branch[..2]` cached, `branch[2]` is the trigger.
pub async fn run_execute_reorg(fx: &mut Fx, branch: &[Block]) -> anyhow::Result<()> {
    seed_cache(&fx.node, &branch[..2]).await;
    let plan = ReorgResult {
        rollback: vec![fx.block_a.hash()],
        common_ancestor: fx.base_tip,
        new_blocks: branch.iter().map(|b| b.hash()).collect(),
        weight_delta: 2,
    };
    fx.node.execute_reorg(plan, branch[2].clone()).await
}

pub async fn run_walk(fx: &mut Fx, branch: &[Block]) -> anyhow::Result<()> {
    let recovery = CompletedRecovery {
        blocks: branch.to_vec(),
        connection_point: fx.base_tip,
    };
    fx.node.handle_completed_fork_recovery(recovery).await
}
