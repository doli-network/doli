//! INC-I-147 defect D4 — a rolled-back block is permanently un-re-appliable.
//!
//! `handle_new_block` (`bins/node/src/node/block_handling.rs:126`) short-circuits on a
//! bare possession check:
//!
//! ```ignore
//! if self.block_store.get_block(&block_hash)?.is_some() {
//!     return Ok(());              // <- no canonical-chain term
//! }
//! ```
//!
//! `remove_canonical_entry` (`crates/storage/src/block_store/writes.rs:203-218`) deletes
//! only `CF_HEIGHT_INDEX` and `CF_HASH_TO_HEIGHT` — the block BODY is left in place. So
//! after a rollback the body still exists, the possession check still returns true, and
//! the node refuses the block forever even though it is no longer on its canonical chain.
//!
//! This is what turned a transient poison-rollback into a durable fork. The layer ABOVE
//! (`network_events.rs:102-114`) and the layer BELOW (`apply_block/mod.rs:80-108`) both
//! fall through correctly for exactly this case; the middle layer makes the lower one
//! unreachable.
//!
//! MEASURED on the live testnet specimen (2026-07-31, RUN_ID=475): n7 logged **159**
//! `status=already_known` refusals while stranded on the losing fork, and zero
//! layer-3 warnings appeared in 783 MB of logs.
//!
//! Note the load-bearing tension recorded in the diagnosis: the poison rollback that
//! creates this state is ALSO the fleet's only escape from the finality wedge (measured
//! 6/6 producers escaped via it; the 2 nodes that never rolled back never escaped). D4's
//! fix is sequencing-safe precisely because it only ADDS the ability to recover — it
//! removes no escape path.

// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, block: Block, source_peer: PeerId) -> Result<()>
//   Outputs:
//     O1: return — Result<()> (Ok in all partitions here; the defect is silent)
//     O2: chain_state.best_height — must advance back to the block's height on re-apply
//     O3: chain_state.best_hash — must equal the re-applied block's hash
//     O4: block_store canonical entry at the height — must be restored
//   Paths:
//     P1: already_known short-circuit (block_handling.rs:126)
//   INPUT PARTITIONS:
//     P1a: body present AND canonical at its height — correctly a no-op (existing behavior)
//     P1b: body present BUT canonical entry removed by rollback — MUST re-apply, currently
//          short-circuits (INC-I-147 D4)
//   MATRIX: 4 outputs x 2 partitions = 8 cells
//     P1a: O1(Ok) + O2(unchanged) + O3(unchanged) + O4(unchanged)
//          — rolled_back_block_reapply_is_noop_when_still_canonical
//     P1b: O1(Ok) + O2(restored) + O3(== block.hash()) + O4(restored)
//          — rolled_back_block_can_be_re_applied  [FAIL test]

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use doli_node::node::RollbackOutcome;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// Helpers adapted from bins/node/tests/inc_i_081_direct_apply_fallback.rs

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

fn build_block(
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

fn build_chain(
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

async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", block.header.slot, e));
    }
}

/// P1b — the defect.
///
/// A producer poisons its own block, rolls back its tip, and the network re-delivers the
/// block it just discarded. It must be re-appliable. It is not: the surviving body makes
/// the possession check succeed forever.
///
/// MUST FAIL before the D4 fix.
#[tokio::test]
async fn rolled_back_block_can_be_re_applied() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base).await;

    let tip_block = base[4].clone();
    let tip_hash = tip_block.hash();
    assert_eq!(
        node.chain_state.read().await.best_height,
        5,
        "setup: chain should be at height 5"
    );

    // The poison rollback: discard our own tip.
    let rolled = node
        .rollback_one_block()
        .await
        .expect("setup: rollback_one_block errored")
        == RollbackOutcome::RolledBack;
    assert!(rolled, "setup: rollback_one_block should report success");
    assert_eq!(
        node.chain_state.read().await.best_height,
        4,
        "setup: tip must be rolled back to height 4"
    );

    // PRECONDITION — the enabler. remove_canonical_entry (writes.rs:203-218) deletes only
    // CF_HEIGHT_INDEX and CF_HASH_TO_HEIGHT, so the body survives the rollback.
    assert!(
        node.block_store
            .get_block(&tip_hash)
            .expect("block_store read failed")
            .is_some(),
        "precondition: the block BODY must still be present after rollback — this is what \
         makes the bare possession check at block_handling.rs:126 return true"
    );
    assert!(
        node.block_store
            .get_hash_by_height(5)
            .expect("block_store read failed")
            .is_none(),
        "precondition: the canonical entry at height 5 must have been removed by rollback"
    );

    // The network re-delivers the block we discarded (measured: n7 saw this 159 times).
    let peer = network::PeerId::random();
    node.handle_new_block(tip_block, peer)
        .await
        .expect("handle_new_block returned Err");

    // O2 — the assertion that fails today.
    assert_eq!(
        node.chain_state.read().await.best_height,
        5,
        "INC-I-147 D4: a rolled-back block MUST be re-appliable. handle_new_block \
         short-circuits on a bare `get_block(&hash).is_some()` check \
         (block_handling.rs:126) that carries no canonical-chain term, so the body that \
         survived remove_canonical_entry makes the block permanently un-re-appliable. \
         Measured on the live specimen: n7 refused the canonical block 159 times with \
         status=already_known and never rejoined the chain."
    );

    // O3 + O4
    assert_eq!(
        node.chain_state.read().await.best_hash,
        tip_hash,
        "chain tip hash must equal the re-applied block"
    );
    assert_eq!(
        node.block_store
            .get_hash_by_height(5)
            .expect("block_store read failed"),
        Some(tip_hash),
        "canonical entry at height 5 must be restored"
    );
}

/// P1a — the behavior that must NOT regress.
///
/// When the block is still canonical at its height, re-delivery is correctly a no-op.
/// Without this, a D4 fix could turn every duplicate gossip block into a redundant
/// re-apply.
#[tokio::test]
async fn rolled_back_block_reapply_is_noop_when_still_canonical() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base).await;

    let tip_block = base[4].clone();
    let tip_hash = tip_block.hash();
    let height_before = node.chain_state.read().await.best_height;

    // No rollback — the block is still canonical at its height.
    let peer = network::PeerId::random();
    node.handle_new_block(tip_block, peer)
        .await
        .expect("handle_new_block returned Err");

    assert_eq!(
        node.chain_state.read().await.best_height,
        height_before,
        "re-delivering a still-canonical block must not change the tip height"
    );
    assert_eq!(
        node.chain_state.read().await.best_hash,
        tip_hash,
        "re-delivering a still-canonical block must not change the tip hash"
    );
}
