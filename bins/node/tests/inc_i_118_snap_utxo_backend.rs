//! INC-I-118 Reproduction Test — Snap-synced node's UtxoSet backend divergence
//!
//! TDD RED PHASE: After snap-sync install, `self.utxo_set` is left as a frozen
//! InMemory copy of the snapshot. Post-snap `apply_block` writes UTXO changes
//! ONLY to `state_db` (Phase 3 BlockBatch), never to `self.utxo_set`. Therefore
//! any consensus read through `self.utxo_set` (pool balance, state root) sees
//! stale snapshot-time data forever. A continuous node holds the RocksDb variant
//! whose reads route through state_db, so it stays correct.
//!
//! This test MUST FAIL on buggy code (pool_after == pool_before) and PASS once
//! the fix converts InMemory -> RocksDb after snap install.
//!
// OUTPUT CONTRACT: fn Node::apply_snap_snapshot(snapshot: VerifiedSnapshot) -> Result<()>
//   Outputs:
//     O1: self.utxo_set — replaced with deserialized snapshot state (InMemory variant)
//     O2: self.chain_state — updated to snapshot height/hash
//     O3: state_db — atomically replaced with snapshot UTXOs via atomic_replace
//   Paths:
//     P1: success + buggy InMemory — utxo_set is frozen InMemory; post-snap apply_block
//         writes go to state_db only; reads through self.utxo_set see stale data
//     P2: success + fixed RocksDb — utxo_set converted to RocksDb(state_db) after install;
//         reads through self.utxo_set route to state_db; post-snap writes visible
//
// OUTPUT CONTRACT: fn Node::apply_block(block, mode, peer) -> Result<()>  [post-snap path]
//   Outputs:
//     O4: state_db — new coinbase UTXO written via BlockBatch
//     O5: self.utxo_set — NOT written (Phase 3: "no utxo_store writes")
//     O6: self.chain_state — height/hash updated
//
// INPUT PARTITIONS:
//   IP1: single post-snap block with coinbase -> pool grows by block_reward in state_db,
//        but frozen InMemory sees no change (buggy) vs. RocksDb sees change (fixed)
//   IP2: only one partition needed — the backend enum variant is the sole differentiator.
//        All coinbase amounts exercise the same code path; the divergence is structural,
//        not value-dependent.
//
// OUTPUTS x PATHS MATRIX (this test):
//   | Output                     | P1 (buggy InMemory)                      | P2 (fixed RocksDb)           |
//   |----------------------------|------------------------------------------|------------------------------|
//   | O1 utxo_set pool balance   | STALE (== pool_before) <-- ASSERT FAILS  | FRESH (== pool_before+reward)|
//   | O4 state_db pool balance   | CORRECT (written by BlockBatch)           | CORRECT                      |

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;

/// REQ-118-001 (Must): A snap-synced node's reward-pool balance after install
/// MUST equal canonical at the snapshot height, AND post-snap apply_block writes
/// MUST be visible through self.utxo_set.
///
/// This test reproduces the INC-I-118 bug: after snap sync installs an InMemory
/// UtxoSet, a subsequent apply_block writes a new pool UTXO to state_db via
/// BlockBatch, but the frozen InMemory set does not see the write.
///
/// On BUGGY code: pool_after == pool_before (assertion FAILS — RED)
/// On FIXED code: pool_after == pool_before + block_reward (assertion PASSES — GREEN)
#[tokio::test]
async fn test_post_snap_utxo_write_visible_through_utxo_set() {
    // ---- Step 1: Build a node with 2 producers ----
    let temp = tempfile::TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..2).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");

    let params = node.params.clone();
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();

    // ---- Step 2: Apply block at height 1 (coinbase -> reward pool) ----
    // This writes the pool UTXO to state_db via BlockBatch (Phase 3).
    // Note: new_for_test creates InMemory utxo_set, so self.utxo_set stays empty.
    // But state_db gets the correct pool UTXO — this is fine because in production
    // the snapshot source is a continuous node with RocksDb-backed reads.
    let genesis_hash = node.chain_state.read().await.best_hash;
    let block1 = build_test_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block1.clone(), ValidationMode::Full, None)
        .await
        .expect("apply_block(h=1) failed");

    // ---- Step 3: Serialize snapshot from state_db (the authoritative store) ----
    // In production, a snapshot source node has RocksDb-backed UtxoSet, so
    // serialize_canonical() routes through state_db. We simulate the same by
    // reading utxo_bytes directly from state_db, and cs/ps via bincode.
    let (cs_bytes, utxo_bytes, ps_bytes, state_root, best_hash, best_height) = {
        let cs = node.chain_state.read().await;
        let ps = node.producer_set.read().await;
        let cs_bytes = bincode::serialize(&*cs).unwrap();
        let ps_bytes = bincode::serialize(&*ps).unwrap();
        // Read UTXO bytes from state_db — the sole authoritative store after Phase 4.
        // This is what a production snapshot source node would serialize.
        let utxo_bytes = node.state_db.serialize_canonical_utxo();
        // Compute state root from the raw bytes (same path as apply_snap_snapshot uses)
        let root = storage::compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes)
            .expect("compute_state_root_from_bytes failed");
        (
            cs_bytes,
            utxo_bytes,
            ps_bytes,
            root,
            cs.best_hash,
            cs.best_height,
        )
    };

    // Sanity: the snapshot UTXO bytes contain the pool UTXO from block 1
    {
        let snap_utxo = storage::UtxoSet::deserialize_canonical(&utxo_bytes)
            .expect("snapshot utxo deserialization failed");
        let snap_pool: u64 = snap_utxo
            .get_by_pubkey_hash(&pool_hash)
            .iter()
            .map(|(_, e)| e.output.amount)
            .sum();
        let h1_reward = params.block_reward(1);
        assert_eq!(
            snap_pool, h1_reward,
            "sanity: snapshot UTXO bytes must contain the block-1 pool reward ({} != {})",
            snap_pool, h1_reward
        );
    }

    let snapshot = network::VerifiedSnapshot {
        block_height: best_height,
        block_hash: best_hash,
        state_root,
        chain_state: cs_bytes,
        utxo_set: utxo_bytes,
        producer_set: ps_bytes,
        block_header_bytes: None,
        epoch_state_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
    };

    // ---- Step 4: Apply snap snapshot ----
    // On buggy code: self.utxo_set becomes InMemory (frozen copy of snapshot).
    // On fixed code: self.utxo_set would be converted to RocksDb(state_db).
    node.apply_snap_snapshot(snapshot)
        .await
        .expect("apply_snap_snapshot failed");

    // ---- Step 5: Read pool balance BEFORE the post-snap block ----
    let pool_before: u64 = {
        let utxo = node.utxo_set.read().await;
        utxo.get_by_pubkey_hash(&pool_hash)
            .iter()
            .map(|(_, entry)| entry.output.amount)
            .sum()
    };

    // Sanity: pool_before should equal the block-1 reward (snapshot installed correctly)
    let height1_reward = params.block_reward(1);
    assert_eq!(
        pool_before, height1_reward,
        "sanity: pool balance via self.utxo_set after snap install should equal block-1 reward ({} != {})",
        pool_before, height1_reward
    );

    // ---- Step 6: Apply a POST-snap block at snapshot_height + 1 ----
    let post_snap_height = best_height + 1;
    let post_snap_slot = 2u32; // slot > block1's slot=1
    let post_snap_block = build_test_block(
        post_snap_height,
        post_snap_slot,
        best_hash,
        &producers[0],
        &params,
    );
    let post_snap_reward = params.block_reward(post_snap_height);

    node.apply_block(post_snap_block, ValidationMode::Full, None)
        .await
        .expect("apply_block(post-snap) failed");

    // ---- Step 7: Read pool balance AFTER via self.utxo_set ----
    let pool_after: u64 = {
        let utxo = node.utxo_set.read().await;
        utxo.get_by_pubkey_hash(&pool_hash)
            .iter()
            .map(|(_, entry)| entry.output.amount)
            .sum()
    };

    // ---- Step 8: ASSERT pool balance grew by exactly the post-snap block reward ----
    //
    // On BUGGY code: self.utxo_set is the frozen InMemory copy from the snapshot.
    //   apply_block wrote the new pool UTXO to state_db only (Phase 3 BlockBatch).
    //   The InMemory copy does not see the write.
    //   pool_after == pool_before == height1_reward  --> ASSERTION FAILS (RED)
    //
    // On FIXED code: self.utxo_set is RocksDb(state_db), reads route through state_db.
    //   pool_after == pool_before + post_snap_reward  --> ASSERTION PASSES (GREEN)
    assert_eq!(
        pool_after,
        pool_before + post_snap_reward,
        "INC-I-118 REPRODUCTION: post-snap UTXO write NOT visible through self.utxo_set!\n\
         pool_before={}, pool_after={}, expected={} (pool_before + block_reward={})\n\
         Root cause: apply_snap_snapshot leaves self.utxo_set as frozen InMemory;\n\
         apply_block writes only to state_db via BlockBatch, never to InMemory set.",
        pool_before,
        pool_after,
        pool_before + post_snap_reward,
        post_snap_reward
    );
}

// ============================================================
// Helper — mirrors bins/node/tests/diagnostic_d2_emit_test.rs
// ============================================================

fn build_test_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &doli_core::consensus::ConsensusParams,
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
        vdf_output: vdf::VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}
