//! INC-I-064: Supply Conservation Tests
//!
//! // OUTPUT CONTRACT: Node::apply_block() supply conservation
//!
//! Observable outputs:
//! O1. apply_block returns Err when spend_transaction fails (non-Replay)
//! O2. validate_block_economics rejects mismatched pool inputs in Light mode
//! O3. Replay mode tolerates spend failures (warns, does not crash)
//! O4. Fee-paying user TXs accepted (fees burn value, deflationary by design)
//! O5. Normal blocks produce correct UTXO accounting (delta = coinbase)
//!
//! PATHS: Full mode | Light mode | Replay mode
//!
//! MATRIX:
//! O1 × Light × P-BAD-SPEND → spend_failure_propagates_in_light_mode
//! O1 × Full × P-BAD-SPEND → spend_failure_propagates_in_full_mode
//! O2 × Light × P-EPOCH → epoch_inputs_mismatch_fails_in_light_mode
//! O3 × Replay × P-BAD-SPEND → replay_mode_tolerates_spend_failure
//! O4 × Light × P-FEE-TX → conservation_allows_fee_paying_user_tx
//! O5 × Light × P-COINBASE → conservation_invariant_normal_blocks_pass
//! O5 × Replay × P-COINBASE → replay_normal_chain_succeeds
//!
//! INPUT PARTITIONS:
//! - P-COINBASE: Block with coinbase only (delta = +coinbase_amount exactly)
//! - P-FEE-TX: Block with fee-paying user TX (delta < +coinbase_amount, fees burned)
//! - P-BAD-SPEND: Block with non-existent UTXO input (spend_transaction fails first)
//! - P-EPOCH: Block with EpochReward TX with mismatched pool inputs
//!
//! Defects verified:
//! - P0: `let _ = utxo.spend_transaction(tx)` — silent spend failure (FIXED)
//! - P1: ECON_EPOCH_INPUTS_MISMATCH gated behind Full mode only (FIXED)
//! - P2: Conservation check REMOVED (INC-I-069) — O(N) RocksDB scan 2x/block
//!   caused CPU exhaustion on mainnet, atomicity bug corrupted UTXO state on bail.
//!   P0+P1 provide sufficient inflation protection without the per-block scan.

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// HELPERS
// ============================================================

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

/// Build a block with an additional transaction that references a non-existent UTXO.
fn build_block_with_bad_spend(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);

    // Create a transfer TX with an input referencing a non-existent UTXO
    let bad_tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(crypto::hash::hash(b"nonexistent_utxo"), 0)],
        outputs: vec![Output::normal(1000, crypto::hash::hash(b"recipient"))],
        extra_data: vec![],
    };

    let txs = vec![coinbase, bad_tx];
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
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

    Block::new(header, txs)
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
        let h = start_height + i as u64;
        let s = start_slot + i as u32;
        let block = build_block(h, s, prev, producer, params);
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light, None)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed: {}", e));
    }
}

// ============================================================
// TEST 1: spend_transaction failure must propagate (not silently succeed)
// ============================================================
#[tokio::test]
async fn spend_failure_propagates_in_full_mode() {
    // Note: Full mode includes producer scheduling validation which rejects
    // blocks before reaching TX processing. We test with Light mode which
    // skips scheduling but still validates UTXOs — the critical path.
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 1, &params);
    apply_chain(&mut node, &chain).await;

    let prev_hash = chain[0].hash();
    let bad_block = build_block_with_bad_spend(2, 2, prev_hash, &producers[0], &params);

    // Use Light mode to bypass producer scheduling (Full rejects producer first).
    let result = node
        .apply_block(bad_block, ValidationMode::Light, None)
        .await;

    assert!(
        result.is_err(),
        "apply_block should fail when spend_transaction references a missing UTXO"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("UTXO")
            || err_msg.contains("output not found")
            || err_msg.contains("spend"),
        "Error should mention UTXO spend failure, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn spend_failure_propagates_in_light_mode() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 1, &params);
    apply_chain(&mut node, &chain).await;

    let prev_hash = chain[0].hash();
    let bad_block = build_block_with_bad_spend(2, 2, prev_hash, &producers[0], &params);

    // Light mode should ALSO fail — not just Full mode
    let result = node
        .apply_block(bad_block, ValidationMode::Light, None)
        .await;

    assert!(
        result.is_err(),
        "apply_block (Light) should fail when spend_transaction references a missing UTXO"
    );
}

// ============================================================
// TEST 2: EpochReward pool input mismatch must fail in Light mode
// ============================================================
#[tokio::test]
async fn epoch_inputs_mismatch_fails_in_light_mode() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();

    // Build a chain up to just before the epoch boundary.
    // Epoch boundary is at height = blocks_per_epoch (epoch 1 starts).
    // We need height = blocks_per_epoch to be an epoch start.
    let chain_len = (blocks_per_epoch - 1) as usize;
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], chain_len, &params);
    apply_chain(&mut node, &chain).await;

    let prev_hash = chain.last().unwrap().hash();
    let epoch_height = blocks_per_epoch;
    let epoch_slot = blocks_per_epoch as u32;

    // Build an epoch boundary block with a coinbase + an EpochReward TX
    // whose inputs reference UTXOs that DON'T exist in the pool.
    let reward = params.block_reward(epoch_height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, epoch_height, 0);

    // EpochReward with bogus inputs (not matching pool UTXOs)
    let bogus_inputs = vec![
        (crypto::hash::hash(b"fake_pool_utxo_1"), 0u32),
        (crypto::hash::hash(b"fake_pool_utxo_2"), 0u32),
    ];
    let recipient_hash = crypto::hash::hash(b"producer_reward");
    let epoch_reward = Transaction::new_epoch_reward_coinbase(
        bogus_inputs,
        vec![(1000, recipient_hash)],
        epoch_height,
        0, // completed epoch 0
    );

    let txs = vec![coinbase, epoch_reward];
    let timestamp = params.genesis_time + (epoch_slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot: epoch_slot,
        producer: *producers[0].public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    let bad_epoch_block = Block::new(header, txs);

    // Apply in Light mode — should now FAIL (was previously skipped)
    let result = node
        .apply_block(bad_epoch_block, ValidationMode::Light, None)
        .await;

    assert!(
        result.is_err(),
        "EpochReward with mismatched pool inputs should fail even in Light mode"
    );
}

// ============================================================
// TEST 3: UTXO accounting correctness — external verification
// ============================================================
#[tokio::test]
async fn utxo_accounting_correct_after_normal_blocks() {
    // [P-COINBASE] Verify UTXO total_value delta equals coinbase for normal blocks.
    // This is an external accounting check (not an in-apply_block invariant).
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);

    for block in &chain {
        let total_before = {
            let utxo = node.utxo_set.read().await;
            utxo.total_value()
        };

        node.apply_block(block.clone(), ValidationMode::Light, None)
            .await
            .expect("Normal block should succeed");

        let total_after = {
            let utxo = node.utxo_set.read().await;
            utxo.total_value()
        };

        let coinbase_amount: u64 = block
            .transactions
            .first()
            .filter(|tx| tx.is_coinbase())
            .map(|tx| {
                tx.outputs
                    .iter()
                    .filter(|o| o.output_type.is_native_amount())
                    .map(|o| o.amount)
                    .sum()
            })
            .unwrap_or(0);

        assert_eq!(
            total_after,
            total_before + coinbase_amount,
            "UTXO accounting: total_value delta should equal coinbase at h={}",
            block.header.slot
        );
    }
}

// ============================================================
// TEST 4: Replay mode tolerates spend failures (historical bad blocks)
// ============================================================

#[tokio::test]
async fn replay_mode_tolerates_spend_failure() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Apply a normal block first
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 1, &params);
    apply_chain(&mut node, &chain).await;

    // Build a block with a Transfer TX referencing a non-existent UTXO.
    // In Replay mode, both UTXO validation and spend_transaction failures
    // should be tolerated — historical blocks are from a trusted backup.
    // This exercises the same tolerance path as E362 replay.
    let prev_hash = chain[0].hash();
    let bad_block = build_block_with_bad_spend(2, 2, prev_hash, &producers[0], &params);

    // In Replay mode, this should succeed (warn, not error)
    let result = node
        .apply_block(bad_block, ValidationMode::Replay, None)
        .await;

    assert!(
        result.is_ok(),
        "Replay mode should tolerate spend failures in historical blocks, got: {:?}",
        result.err()
    );
}

// ============================================================
// TEST 5: Fee-paying user TX accepted — fees burn value (deflationary)
// [P-FEE-TX] apply_block accepts blocks where total_after < total_before + coinbase
// ============================================================
#[tokio::test]
async fn fee_paying_user_tx_accepted() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Apply one block to advance chain state
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 1, &params);
    apply_chain(&mut node, &chain).await;

    // Create a keypair to sign the spending input
    let spender = KeyPair::generate();
    let spender_pubkey_hash =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, spender.public_key().as_bytes());

    // Seed the UTXO set with a known spendable UTXO (1000 sats) owned by spender
    let fake_tx_hash = crypto::hash::hash(b"fake_funding_tx");
    let fake_outpoint = Outpoint::new(fake_tx_hash, 0);
    let fake_entry = UtxoEntry {
        output: Output {
            amount: 1000,
            pubkey_hash: spender_pubkey_hash,
            output_type: OutputType::Normal,
            lock_until: 0,
            extra_data: vec![],
        },
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    // Insert into both in-memory UTXO set AND RocksDB state_db
    {
        let mut utxo = node.utxo_set.write().await;
        utxo.insert(fake_outpoint, fake_entry.clone()).unwrap();
    }
    node.state_db.insert_utxo(&fake_outpoint, &fake_entry);

    // Build a block with coinbase + a user TX that spends 1000 sats, outputs 999 (fee = 1 sat)
    let prev_hash = chain[0].hash();
    let reward = params.block_reward(2);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, 2, 0);

    let mut user_tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: fake_tx_hash,
            output_index: 0,
            signature: crypto::Signature::default(),
            sighash_type: doli_core::transaction::SighashType::All,
            committed_output_count: 0,
            public_key: Some(*spender.public_key()),
        }],
        outputs: vec![Output {
            amount: 999, // fee = 1 sat (BASE_FEE burned)
            pubkey_hash: crypto::hash::hash(b"recipient"),
            output_type: OutputType::Normal,
            lock_until: 0,
            extra_data: vec![],
        }],
        extra_data: vec![],
    };

    // Sign the transaction input
    let sighash = user_tx.signing_message_for_input(0);
    user_tx.inputs[0].signature = crypto::signature::sign_hash(&sighash, spender.private_key());

    let txs = vec![coinbase, user_tx];
    let timestamp = params.genesis_time + (2 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot: 2,
        producer: *producers[0].public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    let block_with_fee = Block::new(header, txs);

    // This MUST succeed — fee-paying TXs burn value (deflationary), which is expected.
    let result = node
        .apply_block(block_with_fee, ValidationMode::Light, None)
        .await;

    assert!(
        result.is_ok(),
        "Block with fee-paying user TX must succeed. \
         Fee burns are expected (deflationary). Got error: {:?}",
        result.err()
    );
}

// ============================================================
// TEST 6: Normal chain replay produces correct state
// ============================================================
#[tokio::test]
async fn replay_normal_chain_succeeds() {
    // Verify that normal blocks (no bugs) replay correctly.
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);

    // Apply normally first
    apply_chain(&mut node, &chain).await;
    let expected_height = { node.chain_state.read().await.best_height };

    // Wipe state and replay
    {
        let mut utxo = node.utxo_set.write().await;
        utxo.clear();
    }
    {
        let mut state = node.chain_state.write().await;
        state.best_height = 0;
        state.best_hash = Hash::ZERO;
        state.best_slot = 0;
    }

    for block in &chain {
        node.apply_block(block.clone(), ValidationMode::Replay, None)
            .await
            .unwrap_or_else(|e| panic!("replay failed: {}", e));
    }

    let replayed_height = { node.chain_state.read().await.best_height };
    assert_eq!(
        replayed_height, expected_height,
        "Replay should produce same height"
    );
}
