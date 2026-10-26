//! INC-I-262 M2 / R-5 — the offline replay (`check-producer-set`, `recover`)
//! must apply the canonical EpochReward that spends the reward pool, the block
//! the live sync path and Full-mode economics accept.
//!
// covers: validation_checks producer_set_check chain
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()>
//   O1 return value: Ok | Err("[ECON_EPOCH_PRE_INPUTS] ...") | Err("[ECON_EPOCH_INPUTS_MISMATCH] ...")
//   O2 state_db UTXOs: every pool outpoint the EpochReward names is spent
//   O3 chain_state.best_height == the boundary; state root equal to the live node's root
// PATHS: P-LIVE Light apply on the sync node | P-REPLAY Replay apply on a fresh replay node
//        | P-FULL Full economics on the live tip (validate_block_economics, the arm apply_block calls)
// INPUT PARTITIONS: EpochReward inputs == sorted pool outpoints | one pool outpoint swapped for a foreign one
// MATRIX:
//   O1+O2+O3 x P-REPLAY x canonical -> replay_applies_canonical_epoch_reward_with_pool_inputs [EXPECTED RED]
//   O1 x P-FULL x swapped input -> full_mode_still_refuses_epoch_reward_inputs_that_miss_the_pool (lock)
//   O1 x P-LIVE + P-FULL x canonical, O3 before the boundary -> PRECONDITIONs in the fixture

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::{Block, Network};
use doli_node::node::attestation::commit::block_presence_root_at;
use doli_node::node::Node;
use storage::UtxoSet;
use tempfile::TempDir;

use super::inc_i_255_m2_common::producer_auth;
use crate::block_sig::sign_header;
use crate::inc_i_156_m1_harness::{
    build_block_with_txs, coinbase_for, install_production_utxo_backend,
};

const GENESIS: u64 = 40;
const EPOCH: u64 = 4;
// The epoch frozen at GENESIS + EPOCH is the first one holding the producer.
const BOUNDARY: u64 = GENESIS + 2 * EPOCH;
const COMPLETED: u64 = BOUNDARY / EPOCH - 1;

struct Live {
    _tmp: TempDir,
    node: Node,
    blocks: Vec<Block>,
    pool_inputs: Vec<(Hash, u32)>,
    root_before: Hash,
    producer: KeyPair,
}

async fn replay_node(dir: std::path::PathBuf) -> Node {
    let node = Node::new_for_replay(dir, Network::Devnet)
        .await
        .expect("FIXTURE: Node::new_for_replay");
    assert_eq!(
        node.config.network.genesis_blocks(),
        GENESIS,
        "PRECONDITION: devnet genesis window"
    );
    assert_eq!(
        node.config.network.blocks_per_reward_epoch(),
        EPOCH,
        "PRECONDITION: devnet epoch"
    );
    node
}

/// Genesis-window VDF-proof Registration the genesis completion derives the producer from.
fn genesis_registration(kp: &KeyPair) -> Transaction {
    let mut tx = Transaction::new_registration(Vec::new(), *kp.public_key(), 0, u64::MAX, 0);
    let mut d = tx.registration_data().expect("FIXTURE: registration data");
    let bls = BlsKeyPair::from_seed(&[0x63; 32]).expect("FIXTURE: BLS seed");
    d.bls_pubkey = bls.public_key().as_bytes().to_vec();
    d.bls_pop = bls
        .proof_of_possession()
        .expect("FIXTURE: PoP")
        .as_bytes()
        .to_vec();
    d.vdf_output = vec![0x11; 32];
    tx.extra_data = bincode::serialize(&d).expect("FIXTURE: encode registration data");
    producer_auth(tx, kp)
}

/// Producer bit set, presence root committed to it, header re-signed.
fn attested(node: &Node, mut block: Block, producer: &KeyPair, height: u64) -> Block {
    let ah = node
        .config
        .network
        .params()
        .inc_i_178_attestation_bls_activation_height;
    let bf = vec![0x01];
    block.header.presence_root = block_presence_root_at(ah, height, &bf, &[]);
    sign_header(&mut block.header, producer);
    block.attestation_bitfield = bf;
    block
}

/// The committed UTXOs, whatever variant the node holds in memory.
fn on_disk(node: &Node) -> UtxoSet {
    UtxoSet::from_state_db(node.state_db.clone())
}

fn sorted_pool(node: &Node) -> Vec<(Hash, u32)> {
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let mut ops: Vec<(Hash, u32)> = on_disk(node)
        .get_by_pubkey_hash(&pool)
        .iter()
        .map(|(op, _)| (op.tx_hash, op.index))
        .collect();
    ops.sort();
    ops
}

async fn state_root(node: &Node) -> Hash {
    let cs = node.chain_state.read().await;
    let ps = node.producer_set.read().await;
    storage::compute_state_root(&cs, &on_disk(node), &ps).expect("state root")
}

/// The boundary block the builder assembles: coinbase, then an EpochReward
/// spending `inputs` and paying the node's own reward calculation.
async fn boundary_block(live: &Live, inputs: Vec<(Hash, u32)>) -> Block {
    let outputs = live
        .node
        .calculate_epoch_rewards(COMPLETED)
        .await
        .expect("PRECONDITION: the live store holds the whole completed epoch");
    assert!(
        !outputs.is_empty(),
        "PRECONDITION: the completed epoch pays a reward"
    );
    let txs = vec![
        coinbase_for(BOUNDARY, BOUNDARY as u32, &live.node.params),
        Transaction::new_epoch_reward_coinbase(inputs, outputs, BOUNDARY, COMPLETED),
    ];
    let prev = live.blocks.last().expect("FIXTURE: chain").hash();
    build_block_with_txs(
        BOUNDARY as u32,
        prev,
        &live.producer,
        &live.node.params,
        txs,
    )
}

/// A live sync node (production UTXO backend, Light apply) at tip BOUNDARY - 1.
async fn live_tip() -> Live {
    let tmp = TempDir::new().unwrap();
    let producer = KeyPair::generate();
    let mut node = replay_node(tmp.path().join("live")).await;
    install_production_utxo_backend(&node).await;
    let mut blocks = Vec::new();
    let mut prev = node.chain_state.read().await.best_hash;
    for h in 1..BOUNDARY {
        let mut txs = vec![coinbase_for(h, h as u32, &node.params)];
        if h == 1 {
            txs.push(genesis_registration(&producer));
        }
        let mut block = build_block_with_txs(h as u32, prev, &producer, &node.params, txs);
        // The genesis completion at GENESIS + 1 is the first block with an active producer.
        if h > GENESIS + 1 {
            block = attested(&node, block, &producer, h);
        }
        prev = block.hash();
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("FIXTURE: live apply h={h}: {e:#}"));
        blocks.push(block);
    }
    let pool_inputs = sorted_pool(&node);
    assert!(
        pool_inputs.len() >= 2,
        "PRECONDITION: the pool holds UTXOs before the boundary: {pool_inputs:?}"
    );
    let root_before = state_root(&node).await;
    Live {
        _tmp: tmp,
        node,
        blocks,
        pool_inputs,
        root_before,
        producer,
    }
}

// REQ-DEF-005 — Decision: a failure means check-producer-set and recover die at the first reward epoch of every real chain, so the residue check never reaches a verdict and recovery cannot rebuild state.
#[tokio::test]
async fn replay_applies_canonical_epoch_reward_with_pool_inputs() {
    let mut live = live_tip().await;
    let block = boundary_block(&live, live.pool_inputs.clone()).await;
    let full = live
        .node
        .validate_block_economics(&block, BOUNDARY, ValidationMode::Full)
        .await;
    assert!(
        full.is_ok(),
        "PRECONDITION: Full economics accepts the canonical boundary block: {full:?}"
    );
    live.node
        .apply_block(block.clone(), ValidationMode::Light)
        .await
        .unwrap_or_else(|e| panic!("PRECONDITION: the live sync path applies the boundary: {e:#}"));
    live.blocks.push(block);

    let tmp = TempDir::new().unwrap();
    let mut replay = replay_node(tmp.path().join("replay")).await;
    for (i, b) in live.blocks.iter().enumerate() {
        let h = i as u64 + 1;
        let r = replay.apply_block(b.clone(), ValidationMode::Replay).await;
        assert!(
            r.is_ok(),
            "Replay refused the canonical block at h={h} (EpochReward spending {} pool inputs) \
             that the live path and Full economics accept: {:#}",
            live.pool_inputs.len(),
            r.unwrap_err()
        );
        if h == BOUNDARY - 1 {
            assert_eq!(
                state_root(&replay).await,
                live.root_before,
                "PRECONDITION: replay and live agree before the boundary"
            );
        }
    }
    assert_eq!(
        replay.chain_state.read().await.best_height,
        BOUNDARY,
        "replay tip"
    );
    let left = sorted_pool(&replay);
    assert!(
        live.pool_inputs.iter().all(|op| !left.contains(op)),
        "replay left a spent pool outpoint unspent: {left:?}"
    );
    assert_eq!(
        state_root(&replay).await,
        state_root(&live.node).await,
        "replay state root differs from the live node after the boundary"
    );
}

// REQ-DEF-005 — Decision: a failure means a fix for the replay arm also stopped Full mode from refusing an EpochReward whose inputs leave a pool UTXO unspent.
#[tokio::test]
async fn full_mode_still_refuses_epoch_reward_inputs_that_miss_the_pool() {
    let live = live_tip().await;
    let mut wrong = live.pool_inputs.clone();
    wrong.pop();
    wrong.push((Hash::from_bytes([0xEE; 32]), 0));
    wrong.sort();
    let block = boundary_block(&live, wrong).await;
    let full = live
        .node
        .validate_block_economics(&block, BOUNDARY, ValidationMode::Full)
        .await
        .map_err(|e| e.to_string());
    assert!(
        full.as_ref()
            .is_err_and(|e| e.contains("[ECON_EPOCH_INPUTS_MISMATCH]")),
        "Full mode must refuse an EpochReward that misses a pool outpoint: {full:?}"
    );
}
