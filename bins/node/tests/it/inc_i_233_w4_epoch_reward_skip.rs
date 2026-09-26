//! INC-I-233 W4 — a Full-mode node whose `calculate_epoch_rewards` fails must
//! still run the structural EpochReward checks that Light mode runs.
//
// OUTPUT CONTRACT: fn validate_block_economics(&self, block: &Block, height: u64, mode: ValidationMode) -> Result<()>
//   (bins/node/src/node/validation_checks/mod.rs; the [INC_I_081_VALIDATION_SKIP] arm
//    calls rewards.rs calculate_epoch_rewards, which fails on an incomplete block store)
//   O1 return value  Ok | Err("[ECON_EPOCH_NO_INPUTS] ...")
// PATHS:
//   P-FULL-SKIP  Full mode, calculate_epoch_rewards Err (empty block store, epoch 1)
//   P-LIGHT      Light mode (no reward calculation) — control, rule already enforced
// INPUT PARTITIONS: EpochReward with 0 inputs paying the whole pool balance to one key
// MATRIX:
//   O1 x P-FULL-SKIP -> w4_full_mode_skip_still_rejects_epoch_reward_without_inputs [EXPECTED RED]
//   O1 x P-LIGHT     -> w4_light_mode_rejects_epoch_reward_without_inputs (control)
//   O1 x P-FULL-SKIP x correct pool inputs -> w4_full_mode_skip_admits_epoch_reward_with_pool_inputs
//      (the kept early Ok(()) after the input checks: no over-rejection)

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Output, Transaction};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const POOL: u64 = 5_000_000_000;

async fn setup() -> (Node, KeyPair, TempDir, u64) {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    // First epoch start after the genesis window with completed_epoch >= 1
    // (epoch 0 is exempt from the incomplete-store fail-fast). The test store
    // holds no blocks, so calculate_epoch_rewards fails for that epoch.
    let bpe = node.config.network.blocks_per_reward_epoch();
    let height = (2..)
        .map(|k| k * bpe)
        .find(|&h| !node.config.network.is_in_genesis(h))
        .expect("boundary");
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let entry = UtxoEntry {
        output: Output::normal(POOL, pool_hash),
        height: 0,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    let op = Outpoint::new(POOL_OP.0, POOL_OP.1);
    node.state_db
        .insert_utxo(&op, &entry)
        .expect("seed state_db");
    node.utxo_set
        .write()
        .await
        .insert(op, entry)
        .expect("seed utxo_set");
    (node, kp, temp, height)
}

const POOL_OP: (Hash, u32) = (Hash::from_bytes([0x9A; 32]), 0);

fn block(node: &Node, producer: &KeyPair, height: u64) -> Block {
    block_with_inputs(node, producer, height, vec![])
}

fn block_with_inputs(
    node: &Node,
    producer: &KeyPair,
    height: u64,
    pool_inputs: Vec<(Hash, u32)>,
) -> Block {
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let slot = height as u32;
    let attacker =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer.public_key().as_bytes());
    let txs = vec![
        Transaction::new_coinbase(node.params.block_reward(height), pool_hash, height, slot),
        // No pool inputs: pays the pool balance out while the pool UTXO survives.
        Transaction::new_epoch_reward_coinbase(
            pool_inputs,
            vec![(POOL, attacker)],
            height,
            height / bpe(node) - 1,
        ),
    ];
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: doli_core::block::compute_merkle_root(&txs),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (slot as u64 * node.params.slot_duration),
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
    Block::new(header, txs)
}

fn bpe(node: &Node) -> u64 {
    node.config.network.blocks_per_reward_epoch()
}

async fn verdict(mode: ValidationMode) -> Result<(), String> {
    let (node, kp, _t, height) = setup().await;
    let b = block(&node, &kp, height);
    let v = node
        .validate_block_economics(&b, height, mode)
        .await
        .map_err(|e| e.to_string());
    println!("[INC-I-233][W4] h={height} mode={mode:?} verdict={v:?}");
    v
}

#[tokio::test]
async fn w4_light_mode_rejects_epoch_reward_without_inputs() {
    let v = verdict(ValidationMode::Light).await;
    assert!(
        v.as_ref()
            .is_err_and(|e| e.contains("ECON_EPOCH_NO_INPUTS")),
        "control: Light mode must reject an input-less EpochReward: {v:?}"
    );
}

#[tokio::test]
async fn w4_full_mode_skip_still_rejects_epoch_reward_without_inputs() {
    let v = verdict(ValidationMode::Full).await;
    assert!(
        v.as_ref()
            .is_err_and(|e| e.contains("ECON_EPOCH_NO_INPUTS")),
        "W4: a Full-mode node that cannot compute rewards accepted an input-less \
         EpochReward that Light mode rejects: {v:?}"
    );
}

#[tokio::test]
async fn w4_full_mode_skip_admits_epoch_reward_with_pool_inputs() {
    let (node, kp, _t, height) = setup().await;
    let b = block_with_inputs(&node, &kp, height, vec![POOL_OP]);
    let v = node
        .validate_block_economics(&b, height, ValidationMode::Full)
        .await
        .map_err(|e| e.to_string());
    println!("[INC-I-233][W4] h={height} mode=Full inputs=pool verdict={v:?}");
    assert!(
        v.is_ok(),
        "a structurally valid EpochReward must still be admitted when the \
         reward calculation fails (Full degrades to Light): {v:?}"
    );
}
