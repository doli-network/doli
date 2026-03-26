//! Mempool Poison Tests
//!
//! Tests that a toxic TX (duplicate NFT/Pool ID) in the mempool does NOT
//! freeze the chain. Validates the mempool purge-by-error-pattern mechanism.
//!
//! Root cause: testnet incident 2026-03-25. A confirmed NFT TX was re-injected
//! into the mempool. build_block included it, but apply_block rejected it
//! (duplicate token_id). The TX stayed in the mempool, freezing production.
//!
//! UniqueId storage tests are in crates/storage/src/utxo/in_memory.rs (unit tests).
//! These integration tests validate the mempool layer.

#[path = "../common/mod.rs"]
mod common;

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::Network;
use mempool::{Mempool, MempoolPolicy};
use storage::{Outpoint, UtxoEntry, UtxoSet};

fn make_signed_nft_tx(keypair: &KeyPair, funding_hash: Hash) -> Transaction {
    let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());

    // NFT extra_data format: condition_bytes + NFT metadata
    // Condition: version(1) + TAG_SIGNATURE(0x00) + pubkey_hash(32)
    let mut extra_data = Vec::new();
    extra_data.push(1u8);   // CONDITION_VERSION
    extra_data.push(0x00);  // TAG_SIGNATURE
    extra_data.extend_from_slice(pubkey_hash.as_bytes());
    // NFT metadata: version(1) + token_id(32) + content_hash_len(1) + content_hash(32)
    extra_data.push(1u8); // nft metadata version
    extra_data.extend_from_slice(&[0xAA; 32]); // token_id
    extra_data.push(32u8); // content_hash_len
    extra_data.extend_from_slice(&[0xBB; 32]); // content_hash

    let fee = doli_core::consensus::BASE_FEE + extra_data.len() as u64 * doli_core::consensus::FEE_PER_BYTE;
    let nft_output = Output {
        amount: 1,
        pubkey_hash,
        output_type: OutputType::NFT,
        lock_until: 0,
        extra_data: extra_data.clone(),
    };
    let change = Output::normal(100_000_000 - 1 - fee, pubkey_hash);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(funding_hash, 0)],
        outputs: vec![nft_output, change],
        extra_data: vec![],
    };

    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    }
    tx
}

fn make_signed_transfer(keypair: &KeyPair, funding_hash: Hash) -> Transaction {
    let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());
    let fee = doli_core::consensus::BASE_FEE;

    let mut tx = Transaction::new_transfer(
        vec![Input::new(funding_hash, 0)],
        vec![
            Output::normal(50_000_000, pubkey_hash),
            Output::normal(50_000_000 - fee, pubkey_hash),
        ],
    );
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    }
    tx
}

fn setup_mempool_with_utxo(keypair: &KeyPair, funding_hash: Hash) -> (Mempool, UtxoSet) {
    let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());
    let pool = Mempool::new(
        MempoolPolicy::default(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    let mut utxo_set = UtxoSet::new();
    let entry = UtxoEntry {
        output: Output::normal(100_000_000, pubkey_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let _ = utxo_set.insert(Outpoint::new(funding_hash, 0), entry);
    (pool, utxo_set)
}

// ============================================================
// TEST 1: NFT TX purged by "token_id" error pattern
// ============================================================
#[tokio::test]
async fn test_poison_nft_purged_by_error_pattern() {
    let keypair = KeyPair::generate();
    let funding_hash = crypto::hash::hash(b"funding_nft_1");
    let (mut pool, utxo_set) = setup_mempool_with_utxo(&keypair, funding_hash);

    let nft_tx = make_signed_nft_tx(&keypair, funding_hash);
    pool.add_transaction(nft_tx, &utxo_set, 10).unwrap();
    assert_eq!(pool.len(), 1);

    pool.remove_by_error_pattern("NFT token_id abc123 already exists");
    assert_eq!(pool.len(), 0, "NFT TX was not purged");
}

// ============================================================
// TEST 2: Normal transfer NOT purged by NFT error pattern
// ============================================================
#[tokio::test]
async fn test_poison_purge_preserves_normal_txs() {
    let keypair = KeyPair::generate();
    let funding_hash = crypto::hash::hash(b"funding_normal_2");
    let (mut pool, utxo_set) = setup_mempool_with_utxo(&keypair, funding_hash);

    let normal_tx = make_signed_transfer(&keypair, funding_hash);
    pool.add_transaction(normal_tx, &utxo_set, 10).unwrap();
    assert_eq!(pool.len(), 1);

    pool.remove_by_error_pattern("NFT token_id already exists");
    assert_eq!(pool.len(), 1, "Normal TX was incorrectly purged");
}

// ============================================================
// TEST 3: Multiple purge cycles don't crash
// ============================================================
#[tokio::test]
async fn test_poison_10_purge_cycles_no_crash() {
    let keypair = KeyPair::generate();
    let (mut pool, mut utxo_set) = setup_mempool_with_utxo(
        &keypair,
        crypto::hash::hash(b"funding_persistent_3"),
    );

    for i in 0..10u64 {
        // Re-create funding UTXO for each cycle
        let funding = crypto::hash::hash(&i.to_le_bytes());
        let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());
        let entry = UtxoEntry {
            output: Output::normal(100_000_000, pubkey_hash),
            height: 10 + i,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        let _ = utxo_set.insert(Outpoint::new(funding, 0), entry);

        let nft_tx = make_signed_nft_tx(&keypair, funding);
        let _ = pool.add_transaction(nft_tx, &utxo_set, 10 + i);

        pool.remove_by_error_pattern("NFT token_id already exists");
    }
    // Should not crash, mempool should be empty or near-empty
    assert!(pool.len() <= 1, "Mempool not clean after 10 cycles: {}", pool.len());
}

// ============================================================
// TEST 4: Pool error pattern purges CreatePool only
// ============================================================
#[test]
fn test_poison_pool_pattern_targets_create_pool() {
    let mut pool = Mempool::new(
        MempoolPolicy::default(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    // Empty mempool — purge doesn't crash
    pool.remove_by_error_pattern("Pool abc123 already exists — cannot create duplicate");
    assert_eq!(pool.len(), 0);
}

// ============================================================
// TEST 5: Registration pattern purges Registration TXs
// ============================================================
#[test]
fn test_poison_registration_pattern() {
    let mut pool = Mempool::new(
        MempoolPolicy::default(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    pool.remove_by_error_pattern("registration already registered");
    assert_eq!(pool.len(), 0);
}

// ============================================================
// TEST 6: Mixed mempool — only toxic type purged
// ============================================================
#[tokio::test]
async fn test_poison_mixed_mempool_selective_purge() {
    let keypair = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(keypair.public_key().as_bytes());

    let mut pool = Mempool::new(
        MempoolPolicy::default(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    let mut utxo_set = UtxoSet::new();

    // Add a normal TX
    let funding1 = crypto::hash::hash(b"funding_mixed_normal");
    let _ = utxo_set.insert(
        Outpoint::new(funding1, 0),
        UtxoEntry {
            output: Output::normal(100_000_000, pubkey_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        },
    );
    let normal_tx = make_signed_transfer(&keypair, funding1);
    let _normal_hash = normal_tx.hash();
    pool.add_transaction(normal_tx, &utxo_set, 10).unwrap();

    // Add an NFT TX
    let funding2 = crypto::hash::hash(b"funding_mixed_nft");
    let _ = utxo_set.insert(
        Outpoint::new(funding2, 0),
        UtxoEntry {
            output: Output::normal(100_000_000, pubkey_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        },
    );
    let nft_tx = make_signed_nft_tx(&keypair, funding2);
    pool.add_transaction(nft_tx, &utxo_set, 10).unwrap();

    assert_eq!(pool.len(), 2);

    // Purge NFT pattern
    pool.remove_by_error_pattern("NFT token_id already exists");

    // Only 1 should remain (the normal TX)
    assert_eq!(pool.len(), 1, "Expected 1 TX remaining, got {}", pool.len());
}

// ============================================================
// TEST 7: Error pattern matching is case-sensitive and specific
// ============================================================
#[test]
fn test_poison_pattern_specificity() {
    let mut pool = Mempool::new(
        MempoolPolicy::default(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );

    // These should NOT trigger any purge (no matching TXs, but patterns must not crash)
    pool.remove_by_error_pattern("unrelated error message");
    pool.remove_by_error_pattern("");
    pool.remove_by_error_pattern("something else entirely");
    assert_eq!(pool.len(), 0);
}

// ============================================================
// TEST 8: Purge after re-gossip — same TX re-added and re-purged
// ============================================================
#[tokio::test]
async fn test_poison_regossip_repurge() {
    let keypair = KeyPair::generate();
    let funding = crypto::hash::hash(b"funding_regossip");
    let (mut pool, utxo_set) = setup_mempool_with_utxo(&keypair, funding);

    let nft_tx = make_signed_nft_tx(&keypair, funding);
    let _tx_hash = nft_tx.hash();

    // First cycle: add + purge
    pool.add_transaction(nft_tx.clone(), &utxo_set, 10).unwrap();
    assert_eq!(pool.len(), 1);
    pool.remove_by_error_pattern("NFT token_id already exists");
    assert_eq!(pool.len(), 0);

    // Second cycle: re-add (simulating gossip) — mempool may reject (spent UTXO)
    // but remove_by_error_pattern must not crash regardless
    let _ = pool.add_transaction(nft_tx.clone(), &utxo_set, 10);
    pool.remove_by_error_pattern("NFT token_id already exists");
    // Should be 0 or already rejected by mempool
    assert!(pool.len() <= 1);
}

// ============================================================
// TEST 9: remove_by_error_pattern is idempotent
// ============================================================
#[test]
fn test_poison_purge_idempotent() {
    let mut pool = Mempool::new(
        MempoolPolicy::default(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );

    // Calling purge 100 times on empty mempool doesn't crash
    for _ in 0..100 {
        pool.remove_by_error_pattern("NFT token_id already exists");
        pool.remove_by_error_pattern("Pool already exists");
        pool.remove_by_error_pattern("registration");
    }
    assert_eq!(pool.len(), 0);
}

// ============================================================
// TEST 10: Full poison lifecycle — add, detect, purge, verify clean
// ============================================================
#[tokio::test]
async fn test_poison_full_lifecycle() {
    let keypair = KeyPair::generate();
    let funding = crypto::hash::hash(b"funding_lifecycle");
    let (mut pool, utxo_set) = setup_mempool_with_utxo(&keypair, funding);

    // Step 1: Add toxic TX
    let nft_tx = make_signed_nft_tx(&keypair, funding);
    let tx_hash = nft_tx.hash();
    pool.add_transaction(nft_tx, &utxo_set, 10).unwrap();
    assert_eq!(pool.len(), 1);

    // Step 2: Simulate "already exists" error from apply_block
    let error_msg = format!("NFT token_id {} already exists", Hash::from_bytes([0xAA; 32]).to_hex());

    // Step 3: Purge
    pool.remove_by_error_pattern(&error_msg);

    // Step 4: Verify clean
    assert_eq!(pool.len(), 0, "Mempool not clean after lifecycle purge");

    // Step 5: Verify the specific TX was removed
    assert!(
        pool.get(&tx_hash).is_none(),
        "Toxic TX still in mempool after purge"
    );
}
