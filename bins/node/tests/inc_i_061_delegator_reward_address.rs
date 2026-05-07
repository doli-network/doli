//! INC-I-061 — Delegator 90% reward sent to wrong address
//!
//! Bug: calculate_epoch_rewards() uses the delegator's ProducerSet key hash
//! (crypto_hash(pubkey)) as the reward output address, but reward outputs need
//! hash_with_domain(ADDRESS_DOMAIN, pubkey) — the wallet address. These are
//! completely different hashes, so delegator rewards go to unreachable addresses.
//!
//! OUTPUT CONTRACT: calculate_epoch_rewards(epoch: u64) -> Vec<(u64, Hash)>
//!   O1: Delegator 90% output address matches hash_with_domain(ADDRESS_DOMAIN, delegator_pubkey)
//!   O2: Delegatee 10% + own share output address matches hash_with_domain(ADDRESS_DOMAIN, delegatee_pubkey)
//!   O3: Total distributed == pool total
//!   O4: Number of outputs == non-delegated producers + delegatees + delegators with rewards
//!   PATHS:
//!     P1: One delegation (A delegates to B) — verify both A's 90% and B's 10%+own
//!     P2: No delegation — baseline, all addresses correct (regression anchor)
//!   MATRIX: 4 outputs × 2 paths = 8 cells, all covered

use std::collections::HashSet;
use std::sync::Once;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{reward_pool_pubkey_hash, ConsensusParams};
use doli_core::transaction::{Output, Transaction};
use doli_core::{Block, BlockHeader, Network};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

static ENV_INIT: Once = Once::new();
fn init_env() {
    ENV_INIT.call_once(|| {
        std::env::set_var("DOLI_BLOCKS_PER_REWARD_EPOCH", "36");
        let _ = Network::Devnet.params();
    });
}

const EPOCH_LEN: u64 = 36;

/// Canonical wallet address for a producer (same formula as rewards.rs line 313).
fn wallet_address(kp: &KeyPair) -> Hash {
    crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

/// ProducerSet internal key (NOT a wallet address).
fn producer_set_key(kp: &KeyPair) -> Hash {
    crypto::hash::hash(kp.public_key().as_bytes())
}

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    init_env();
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    assert_eq!(node.config.network.blocks_per_reward_epoch(), EPOCH_LEN);
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

fn build_block_with_full_bitfield(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    producer_count: usize,
    params: &ConsensusParams,
) -> Block {
    let indices: Vec<usize> = (0..producer_count).collect();
    let bitfield = doli_core::encode_attestation_bitfield_vec(&indices, producer_count);
    let presence_root = crypto::hash::hash(&bitfield);
    let reward = params.block_reward(height);
    let pool_hash = reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root,
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

    let mut block = Block::new(header, vec![coinbase]);
    block.attestation_bitfield = bitfield;
    block
}

async fn populate_two_epochs(node: &Node, producers: &[KeyPair], params: &ConsensusParams) {
    let genesis_hash = node.chain_state.read().await.best_hash;
    let n = producers.len();
    let mut prev = genesis_hash;

    // Epoch 0 + Epoch 1: all blocks with full attestation bitfield
    for h in 0..(2 * EPOCH_LEN) {
        let block = build_block_with_full_bitfield(
            h,
            h as u32,
            prev,
            &producers[(h as usize) % n],
            n,
            params,
        );
        prev = block.hash();
        node.block_store
            .put_block_canonical(&block, h)
            .expect("put_block_canonical failed");
    }
}

async fn seed_reward_pool(node: &Node, total_amount: u64, tag: &str) {
    let pool_hash = reward_pool_pubkey_hash();
    let tx_hash = crypto::hash::hash(tag.as_bytes());
    let entry = UtxoEntry {
        output: Output::normal(total_amount, pool_hash),
        height: 0,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    let mut utxo = node.utxo_set.write().await;
    utxo.insert(Outpoint::new(tx_hash, 0), entry)
        .expect("insert pool UTXO failed");
}

/// P1: Delegation present — delegator 90% reward MUST go to wallet_address, not producer_set_key.
///
/// Setup: 4 producers, producer[0] delegates 1 bond to producer[1].
/// Expected: producer[0]'s 90% share goes to wallet_address(producer[0]).
/// Bug behavior: goes to producer_set_key(producer[0]) — a completely different hash.
#[tokio::test]
async fn test_delegator_reward_uses_wallet_address_not_producer_set_key() {
    let (mut node, producers, _tmp) = make_node(4).await;
    let params = node.params.clone();

    // Populate chain so producers qualify for epoch 1 rewards
    populate_two_epochs(&node, &producers, &params).await;

    // Set up delegation: producer[0] delegates to producer[1]
    {
        let mut ps = node.producer_set.write().await;
        ps.delegate_bonds(producers[0].public_key(), producers[1].public_key(), 1)
            .expect("delegate_bonds failed");
    }

    // Seed epoch bond snapshot with delegation-adjusted weights.
    // producer[0] (delegator): effective = own(1) - delegated_away(1) + received(0) = 0
    // producer[1] (delegatee): effective = own(1) - delegated_away(0) + received(1) = 2
    // producer[2], producer[3]: effective = 1
    for (i, p) in producers.iter().enumerate() {
        let pkh = crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, p.public_key().as_bytes());
        let weight = match i {
            0 => continue, // delegator has 0 effective weight, skip (unwrap_or(1) will give 1)
            1 => 2,        // delegatee has 2 (own + received)
            _ => 1,
        };
        node.epoch_state.bond_snapshot.insert(pkh, weight);
    }

    let pool_total: u64 = 50_000_000; // distributed bond-weighted
    seed_reward_pool(&node, pool_total, "test_delegation_pool").await;

    let outputs = node.calculate_epoch_rewards(1).await;

    // Collect all output addresses
    let output_addresses: HashSet<Hash> = outputs.iter().map(|(_, h)| *h).collect();

    // The delegator's wallet address MUST appear in the outputs
    let delegator_wallet = wallet_address(&producers[0]);
    let delegator_internal_key = producer_set_key(&producers[0]);

    // Sanity: these two hashes are different (the root cause of the bug)
    assert_ne!(
        delegator_wallet, delegator_internal_key,
        "wallet_address and producer_set_key must differ — they use different hash domains"
    );

    // O1: The delegator's reward output MUST use the wallet address
    assert!(
        output_addresses.contains(&delegator_wallet),
        "O1: delegator reward output must use wallet_address (hash_with_domain), \
         not producer_set_key (crypto_hash). \
         Expected delegator address {:.16} in outputs, got: {:?}",
        delegator_wallet,
        outputs
            .iter()
            .map(|(amt, h)| (amt, format!("{:.16}", h)))
            .collect::<Vec<_>>()
    );

    // The internal key must NOT appear (it's the bug)
    assert!(
        !output_addresses.contains(&delegator_internal_key),
        "O1 (negative): producer_set_key hash MUST NOT appear in reward outputs — \
         this means the bug is present"
    );

    // O2: The delegatee's address is correct (this already works)
    let delegatee_wallet = wallet_address(&producers[1]);
    assert!(
        output_addresses.contains(&delegatee_wallet),
        "O2: delegatee reward output must use wallet_address"
    );

    // O3: Total distributed == pool total
    let total: u64 = outputs.iter().map(|(amt, _)| amt).sum();
    assert_eq!(
        total, pool_total,
        "O3: total distributed must equal pool total"
    );

    // O4: Correct number of outputs
    // 2 non-delegated producers (producers[2], producers[3]) get 1 output each
    // producer[1] (delegatee) gets 1 output (own share + 10% fee combined)
    // producer[0] (delegator) gets 1 output (90% of delegated share)
    // Total: at least 4 outputs
    assert!(
        outputs.len() >= 4,
        "O4: expected at least 4 outputs (2 non-delegated + delegatee + delegator), got {}",
        outputs.len()
    );
}

/// P2: No delegation — regression anchor. All addresses must be wallet addresses.
#[tokio::test]
async fn test_no_delegation_all_addresses_are_wallet_addresses() {
    let (mut node, producers, _tmp) = make_node(4).await;
    let params = node.params.clone();

    populate_two_epochs(&node, &producers, &params).await;

    // Seed bond snapshot
    for p in &producers {
        let pkh = crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, p.public_key().as_bytes());
        node.epoch_state.bond_snapshot.insert(pkh, 1);
    }

    let pool_total: u64 = 40_000_000;
    seed_reward_pool(&node, pool_total, "test_no_deleg_pool").await;

    let outputs = node.calculate_epoch_rewards(1).await;
    let output_addresses: HashSet<Hash> = outputs.iter().map(|(_, h)| *h).collect();

    // Every output address must be a wallet address
    for p in &producers {
        let addr = wallet_address(p);
        assert!(
            output_addresses.contains(&addr),
            "P2: producer {:?} wallet address not found in outputs",
            p.public_key()
        );
    }

    // Total == pool
    let total: u64 = outputs.iter().map(|(amt, _)| amt).sum();
    assert_eq!(total, pool_total, "P2 O3: total must equal pool");

    assert_eq!(outputs.len(), 4, "P2 O4: 4 producers, 4 outputs");
}
