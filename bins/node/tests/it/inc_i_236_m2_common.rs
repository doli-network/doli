//! INC-I-236 M2 shared fixtures: single-producer nodes driven through the real
//! production code.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::block::{Block, BlockHeader};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::{ValidationContext, ValidationError};
use doli_core::Network;
use doli_node::node::Node;
use tempfile::TempDir;

use super::inc_i_178_m0_common::wait_for_fresh_second;

pub async fn producer_node(kp: &KeyPair) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

/// Testnet shape: the only network whose full `try_produce_block` path is
/// drivable in-process (devnet's 1 s slot is below the propagation floor).
pub async fn testnet_producer_node(kp: &KeyPair) -> (Node, TempDir) {
    let (mut node, temp) = producer_node(kp).await;
    node.config.network = Network::Testnet;
    node.params = ConsensusParams::testnet();
    (node, temp)
}

/// Drive the real production entry point until one block is applied.
pub async fn produce_one(node: &mut Node) -> Block {
    let start = node.chain_state.read().await.best_height;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        node.try_produce_block()
            .await
            .expect("try_produce_block must return Ok");
        let h = node.chain_state.read().await.best_height;
        if h > start {
            return node
                .block_store
                .get_block_by_height(h)
                .expect("block_store read")
                .expect("produced block must be stored");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("FIXTURE: try_produce_block did not mint within 30 s");
}

/// Devnet block from the real builder + VDF steps of `try_produce_block`,
/// assembled as it does (child of the current tip).
pub async fn build_devnet_block(node: &mut Node) -> Block {
    let (prev_hash, prev_slot, height) = {
        let st = node.chain_state.read().await;
        (st.best_hash, st.best_slot, st.best_height + 1)
    };
    let pk = *node.producer_key.as_ref().unwrap().public_key();
    for _ in 0..8 {
        let now = wait_for_fresh_second();
        let slot = node.params.timestamp_to_slot(now);
        let built = node
            .build_block_content(prev_hash, prev_slot, height, slot, pk)
            .await
            .expect("build_block_content returned Err");
        let Some((header, transactions, bf)) = built else {
            continue;
        };
        let (vdf_output, vdf_proof) = node
            .compute_block_vdf(&prev_hash, &header)
            .await
            .expect("compute_block_vdf returned Err");
        let header = BlockHeader {
            vdf_output,
            vdf_proof,
            ..header
        };
        return Block {
            header,
            transactions,
            aggregate_bls_signature: Vec::new(),
            attestation_bitfield: bf,
        };
    }
    panic!("FIXTURE: build_block_content returned None on every attempt");
}

pub fn sign_into_pi(block: &mut Block, kp: &KeyPair) {
    let msg = block.header.signing_message();
    block.header.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
}

pub fn strict_verifies(block: &Block) -> bool {
    let Ok(sig) = crypto::Signature::try_from_slice(&block.header.vdf_proof.pi) else {
        return false;
    };
    crypto::signature_strict::verify_strict(
        block.header.signing_message().as_bytes(),
        &sig,
        &block.header.producer,
    )
    .is_ok()
}

pub fn hash_with_pi_cleared(block: &Block) -> Hash {
    let mut h = block.header.clone();
    h.vdf_proof.pi.clear();
    h.hash()
}

/// Parent (slot, timestamp, hash) of the next block, derived as the node does.
pub async fn tip(node: &Node) -> (u32, u64, Hash) {
    let st = node.chain_state.read().await;
    let ts = node
        .block_store
        .get_header(&st.best_hash)
        .ok()
        .flatten()
        .map(|h| h.timestamp)
        .unwrap_or(0);
    (st.best_slot, ts, st.best_hash)
}

/// Standalone ctx on the node's network for `block` (child of `prev`), INC-I-236 AH set.
pub fn node_ctx(
    node: &Node,
    block: &Block,
    prev: (u32, u64, Hash),
    height: u64,
    ah: u64,
) -> ValidationContext {
    let mut ctx = ValidationContext::new(
        node.params.clone(),
        node.config.network,
        block.header.timestamp,
        height,
    )
    .with_prev_block(prev.0, prev.1, prev.2)
    .with_epoch_producer_list(vec![block.header.producer])
    .with_fork_id(
        node.current_fork_id(),
        node.config.network.params().fork_id_activation_height,
    );
    ctx.inc_i_236_block_signature_activation_height = ah;
    ctx
}

pub fn is_sig_error(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(ValidationError::InvalidBlockSignature { .. }))
}
