//! INC-I-236 M4 shared fixtures: headers fed through `Node::handle_new_block`.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// Node key = `reporter` (index 0); `victim` is a registered genesis producer.
pub async fn reporter_node() -> (Node, KeyPair, KeyPair, TempDir) {
    let (reporter, victim) = (KeyPair::generate(), KeyPair::generate());
    let temp = TempDir::new().unwrap();
    let node = Node::new_for_test(
        temp.path().to_path_buf(),
        vec![reporter.clone(), victim.clone()],
    )
    .await
    .expect("Node::new_for_test failed");
    (node, reporter, victim, temp)
}

/// Block for `producer` at `slot` with a real VDF output; signed by `signer` when
/// given (else `pi` empty). `salt` varies the coinbase so two calls for one slot conflict.
pub async fn header_block(
    node: &mut Node,
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &PublicKey,
    signer: Option<&KeyPair>,
    salt: u64,
) -> Block {
    let params = node.params.clone();
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(params.block_reward(height) + salt, pool, height, 0);
    let mut header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase)),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: params.genesis_time + slot as u64 * params.slot_duration,
        slot,
        producer: *producer,
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let (vdf_output, _) = node
        .compute_block_vdf(&prev_hash, &header)
        .await
        .expect("fixture: vdf");
    header.vdf_output = vdf_output;
    if let Some(kp) = signer {
        crate::block_sig::sign_header(&mut header, kp);
    }
    Block::new(header, vec![coinbase])
}

/// Canonical h=1 (slot 1) and h=2 (`tip_slot`) applied directly; returns (h1, h2).
pub async fn canonical_two(node: &mut Node, producer: &KeyPair, tip_slot: u32) -> (Block, Block) {
    let genesis = node.chain_state.read().await.best_hash;
    let pk = *producer.public_key();
    let b1 = header_block(node, 1, 1, genesis, &pk, Some(producer), 0).await;
    let b2 = header_block(node, 2, tip_slot, b1.hash(), &pk, Some(producer), 0).await;
    for b in [&b1, &b2] {
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: canonical block must apply");
    }
    assert_eq!(
        node.chain_state.read().await.best_hash,
        b2.hash(),
        "fixture: tip"
    );
    (b1, b2)
}

/// Drains the detector's pending proofs; counts those naming `victim`.
pub async fn proofs_against(node: &Node, victim: &PublicKey) -> usize {
    let proofs = node
        .equivocation_detector
        .write()
        .await
        .take_pending_proofs();
    proofs.iter().filter(|p| p.producer == *victim).count()
}

pub async fn slash_txs_against(node: &Node, victim: &PublicKey) -> usize {
    let pool = node.mempool.read().await;
    pool.iter()
        .filter_map(|(_, e)| e.tx.slash_data())
        .filter(|d| d.producer_pubkey == *victim)
        .count()
}

/// Reads and clears the stuck-fork flag.
pub async fn stuck_fork_raised(node: &Node) -> bool {
    node.sync_manager.write().await.take_stuck_fork_signal()
}

/// Result ignored: the side effects under test fire before any validation verdict.
pub async fn feed(node: &mut Node, block: Block) {
    let _ = node
        .handle_new_block(block, network::PeerId::random())
        .await;
}
