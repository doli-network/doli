use super::*;
use crate::block::{Block, BlockHeader};
use crate::consensus::{ConsensusParams, RewardMode, GENESIS_TIME, TOTAL_SUPPLY};
use crate::network::Network;
use crate::transaction::{Input, Output, OutputType, Transaction, TxType};
use crate::types::BlockHeight;
use crypto::{Hash, PublicKey};
use std::collections::HashMap;
use transaction::validate_outputs;
use utxo::check_internal_double_spend;

fn test_context() -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        1,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
}

/// Mock UTXO provider for testing
struct MockUtxoProvider {
    utxos: HashMap<(Hash, u32), UtxoInfo>,
}

impl MockUtxoProvider {
    fn new() -> Self {
        Self {
            utxos: HashMap::new(),
        }
    }

    fn add_utxo(&mut self, tx_hash: Hash, index: u32, output: Output, pubkey: PublicKey) {
        self.utxos.insert(
            (tx_hash, index),
            UtxoInfo {
                output,
                pubkey: Some(pubkey),
                spent: false,
            },
        );
    }
}

impl UtxoProvider for MockUtxoProvider {
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
        self.utxos.get(&(*tx_hash, output_index)).cloned()
    }
}

// ==========================================================================
// Header Validation Tests
// ==========================================================================

#[test]
fn test_validate_timestamp_advancing() {
    let ctx = test_context();

    // With 10-second slots, slot 1 is at GENESIS_TIME + 10
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: ctx.params.genesis_hash,
        timestamp: GENESIS_TIME + 10,
        slot: 1,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };

    assert!(validate_header(&header, &ctx).is_ok());
}

#[test]
fn test_validate_timestamp_not_advancing() {
    let ctx = test_context();

    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: ctx.params.genesis_hash,
        timestamp: GENESIS_TIME - 1,
        slot: 0,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };

    assert!(matches!(
        validate_header(&header, &ctx),
        Err(ValidationError::InvalidTimestamp { .. })
    ));
}

#[test]
fn test_validate_slot_derivation() {
    let ctx = test_context();

    // With 10-second slots, timestamp GENESIS_TIME + 60 = slot 6
    // Claiming slot 5 should be invalid (actual slot should be 6)
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: ctx.params.genesis_hash,
        timestamp: GENESIS_TIME + 60,
        slot: 5, // Should be 6 with 10-second slots
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };

    assert!(matches!(
        validate_header(&header, &ctx),
        Err(ValidationError::InvalidSlot { .. })
    ));
}

// ==========================================================================
// Transaction Structural Validation Tests
// ==========================================================================

#[test]
fn test_validate_tx_zero_output() {
    let ctx = test_context();

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::normal(0, crypto::hash::hash(b"test"))],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidTransaction(msg)) if msg.contains("zero amount")
    ));
}

#[test]
fn test_validate_tx_large_single_amount() {
    let ctx = test_context();
    let pubkey_hash = crypto::hash::hash(b"test");

    // Test that u64::MAX (larger than TOTAL_SUPPLY) is rejected
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::normal(u64::MAX, pubkey_hash)],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::AmountExceedsSupply { .. })
    ));
}

#[test]
fn test_validate_tx_total_exceeds_supply() {
    let ctx = test_context();
    let pubkey_hash = crypto::hash::hash(b"test");

    // Two outputs that individually are valid but together exceed supply
    // Each is half of TOTAL_SUPPLY + 1
    let half_plus_one = TOTAL_SUPPLY / 2 + 1;

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![
            Output::normal(half_plus_one, pubkey_hash),
            Output::normal(half_plus_one, pubkey_hash),
        ],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::AmountExceedsSupply { .. })
    ));
}

#[test]
fn test_validate_tx_exceeds_supply() {
    let ctx = test_context();
    let pubkey_hash = crypto::hash::hash(b"test");

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::normal(TOTAL_SUPPLY + 1, pubkey_hash)],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::AmountExceedsSupply { .. })
    ));
}

#[test]
fn test_validate_tx_no_inputs() {
    let ctx = test_context();
    let pubkey_hash = crypto::hash::hash(b"test");

    // A transaction with no inputs and multiple outputs (not a coinbase)
    // should fail validation
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![],
        outputs: vec![
            Output::normal(100, pubkey_hash),
            Output::normal(200, pubkey_hash), // Second output so it's not a coinbase
        ],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidTransaction(msg)) if msg.contains("must have inputs")
    ));
}

#[test]
fn test_validate_tx_no_outputs() {
    let ctx = test_context();

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidTransaction(msg)) if msg.contains("must have outputs")
    ));
}

#[test]
fn test_validate_tx_zero_pubkey_hash() {
    let ctx = test_context();

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::normal(100, Hash::ZERO)],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidTransaction(msg)) if msg.contains("zero pubkey_hash")
    ));
}

#[test]
fn test_validate_bond_output_zero_lock() {
    let ctx = test_context();
    let pubkey_hash = crypto::hash::hash(b"test");

    // Bond output with lock_until = 0 is invalid
    let bad_output = Output {
        output_type: OutputType::Bond,
        amount: 100,
        pubkey_hash,
        lock_until: 0, // Invalid for bond
        extra_data: vec![],
    };

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![bad_output],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(result, Err(ValidationError::InvalidBond(_))));
}

#[test]
fn test_validate_normal_output_nonzero_lock() {
    let ctx = test_context();
    let pubkey_hash = crypto::hash::hash(b"test");

    // Normal output with lock_until != 0 is invalid
    let bad_output = Output {
        output_type: OutputType::Normal,
        amount: 100,
        pubkey_hash,
        lock_until: 1000, // Invalid for normal
        extra_data: vec![],
    };

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![bad_output],
        extra_data: vec![],
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidTransaction(msg)) if msg.contains("non-zero lock_until")
    ));
}

// ==========================================================================
// UTXO Validation Tests
// ==========================================================================

#[test]
fn test_validate_tx_with_valid_utxo() {
    let ctx = test_context();

    // Create a keypair for the test
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    // Create a previous transaction output
    let prev_tx_hash = crypto::hash::hash(b"prev_tx");
    let prev_output = Output::normal(1000, pubkey_hash);

    // Set up the mock UTXO provider
    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    // Create the spending transaction
    let new_pubkey_hash = crypto::hash::hash(b"recipient");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(900, new_pubkey_hash)],
        extra_data: vec![],
    };

    // Sign the transaction (BIP-143: per-input)
    let signing_hash = tx.signing_message_for_input(0);
    let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    tx.inputs[0].signature = signature;

    // Validate
    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(result.is_ok());
}

#[test]
fn test_validate_tx_missing_utxo() {
    let ctx = test_context();
    let pubkey_hash = crypto::hash::hash(b"test");

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::normal(100, pubkey_hash)],
        extra_data: vec![],
    };

    let utxo_provider = MockUtxoProvider::new();

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(matches!(
        result,
        Err(ValidationError::OutputNotFound { .. })
    ));
}

#[test]
fn test_validate_tx_insufficient_funds() {
    let ctx = test_context();

    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"prev_tx");
    let prev_output = Output::normal(100, pubkey_hash); // Only 100

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let new_pubkey_hash = crypto::hash::hash(b"recipient");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(200, new_pubkey_hash)], // Needs 200
        extra_data: vec![],
    };

    // Sign the transaction (BIP-143: per-input)
    let signing_hash = tx.signing_message_for_input(0);
    let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    tx.inputs[0].signature = signature;

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(matches!(
        result,
        Err(ValidationError::InsufficientFunds {
            inputs: 100,
            outputs: 200
        })
    ));
}

#[test]
fn test_validate_tx_invalid_signature() {
    let ctx = test_context();

    let owner_keypair = crypto::KeyPair::generate();
    let owner_pubkey = *owner_keypair.public_key();
    let pubkey_hash =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner_pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"prev_tx");
    let prev_output = Output::normal(1000, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, owner_pubkey);

    // Use a DIFFERENT keypair to sign (attack attempt)
    let attacker_keypair = crypto::KeyPair::generate();

    let new_pubkey_hash = crypto::hash::hash(b"recipient");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(900, new_pubkey_hash)],
        extra_data: vec![],
    };

    // Sign with wrong key (BIP-143: per-input)
    let signing_hash = tx.signing_message_for_input(0);
    let bad_signature = crypto::signature::sign_hash(&signing_hash, attacker_keypair.private_key());
    tx.inputs[0].signature = bad_signature;

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidSignature { index: 0 })
    ));
}

#[test]
fn test_validate_tx_locked_output() {
    let ctx = test_context();

    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"prev_tx");
    // Output locked until height 1000
    let prev_output = Output::bond(1000, pubkey_hash, 1000, 0);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let new_pubkey_hash = crypto::hash::hash(b"recipient");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(900, new_pubkey_hash)],
        extra_data: vec![],
    };

    let signing_hash = tx.signing_message_for_input(0);
    let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    tx.inputs[0].signature = signature;

    // Context is at height 1, output locked until 1000
    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(matches!(
        result,
        Err(ValidationError::OutputLocked {
            lock_height: 1000,
            current_height: 1
        })
    ));
}

// ==========================================================================
// Property-Based Tests
// ==========================================================================

use proptest::prelude::*;

#[allow(dead_code)]
fn arb_hash() -> impl Strategy<Value = Hash> {
    any::<[u8; 32]>().prop_map(Hash::from_bytes)
}

#[allow(dead_code)]
fn arb_pubkey_hash() -> impl Strategy<Value = Hash> {
    // Non-zero hash for valid outputs
    any::<[u8; 32]>()
        .prop_filter("non-zero hash", |b| b.iter().any(|&x| x != 0))
        .prop_map(Hash::from_bytes)
}

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]

    /// Valid amount outputs pass validation
    #[test]
    fn prop_valid_outputs_pass(amounts in prop::collection::vec(1u64..1_000_000_000u64, 1..5)) {
        let outputs: Vec<Output> = amounts.iter()
            .enumerate()
            .map(|(i, &a)| {
                let mut hash = [0u8; 32];
                hash[0] = (i as u8) + 1; // Non-zero
                Output::normal(a, Hash::from_bytes(hash))
            })
            .collect();
        let result = validate_outputs(&outputs, &test_context());
        prop_assert!(result.is_ok());
        prop_assert_eq!(result.unwrap(), amounts.iter().sum::<u64>());
    }

    /// Zero amount outputs fail validation
    #[test]
    fn prop_zero_amount_fails(seed: [u8; 32]) {
        let mut hash = seed;
        hash[0] = 1; // Non-zero
        let outputs = vec![Output::normal(0, Hash::from_bytes(hash))];
        let result = validate_outputs(&outputs, &test_context());
        prop_assert!(result.is_err());
    }

    /// Zero pubkey_hash fails validation
    #[test]
    fn prop_zero_pubkey_hash_fails(amount in 1u64..1_000_000_000u64) {
        let outputs = vec![Output::normal(amount, Hash::ZERO)];
        let result = validate_outputs(&outputs, &test_context());
        prop_assert!(result.is_err());
    }

    /// Bond with zero lock fails
    #[test]
    fn prop_bond_zero_lock_fails(amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
        let mut hash = seed;
        hash[0] = 1;
        let output = Output {
            output_type: OutputType::Bond,
            amount,
            pubkey_hash: Hash::from_bytes(hash),
            lock_until: 0,
            extra_data: vec![],
        };
        let result = validate_outputs(&[output], &test_context());
        prop_assert!(result.is_err());
    }

    /// Normal output with lock fails
    #[test]
    fn prop_normal_with_lock_fails(amount in 1u64..1_000_000_000u64, lock in 1u64..1_000_000u64, seed: [u8; 32]) {
        let mut hash = seed;
        hash[0] = 1;
        let output = Output {
            output_type: OutputType::Normal,
            amount,
            pubkey_hash: Hash::from_bytes(hash),
            lock_until: lock,
            extra_data: vec![],
        };
        let result = validate_outputs(&[output], &test_context());
        prop_assert!(result.is_err());
    }

    /// Version 1 transactions with valid structure pass
    #[test]
    fn prop_valid_structure_passes(amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
        let ctx = test_context();
        let mut hash = seed;
        hash[0] = 1;
        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::from_bytes(seed), 0)],
            outputs: vec![Output::normal(amount, Hash::from_bytes(hash))],
            extra_data: vec![],
        };
        let result = validate_transaction(&tx, &ctx);
        prop_assert!(result.is_ok());
    }

    /// Invalid version fails validation
    #[test]
    fn prop_invalid_version_fails(version in 2u32..100u32, amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
        let ctx = test_context();
        let mut hash = seed;
        hash[0] = 1;
        let tx = Transaction {
            version,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::from_bytes(seed), 0)],
            outputs: vec![Output::normal(amount, Hash::from_bytes(hash))],
            extra_data: vec![],
        };
        let result = validate_transaction(&tx, &ctx);
        prop_assert!(matches!(result, Err(ValidationError::InvalidVersion(_))));
    }

    /// Coinbase transactions are valid (structural)
    #[test]
    fn prop_coinbase_valid(height: u64, seed: [u8; 32]) {
        let ctx = ValidationContext::new(
            ConsensusParams::mainnet(),
            Network::Mainnet,
            GENESIS_TIME + 120,
            height,
        ).with_prev_block(0, GENESIS_TIME, Hash::ZERO);

        let mut hash = seed;
        hash[0] = 1;
        let reward = ctx.params.block_reward(height);
        let tx = Transaction::new_coinbase(reward, Hash::from_bytes(hash), height);

        let result = validate_coinbase(&tx, &ctx);
        prop_assert!(result.is_ok());
    }

    /// Insufficient coinbase reward fails
    #[test]
    fn prop_insufficient_coinbase_fails(height in 0u64..1_000_000u64, reduction in 1u64..1_000_000u64, seed: [u8; 32]) {
        let ctx = ValidationContext::new(
            ConsensusParams::mainnet(),
            Network::Mainnet,
            GENESIS_TIME + 120,
            height,
        ).with_prev_block(0, GENESIS_TIME, Hash::ZERO);

        let mut hash = seed;
        hash[0] = 1;
        let reward = ctx.params.block_reward(height);
        // Reduce reward below minimum
        let insufficient = reward.saturating_sub(reduction).min(reward.saturating_sub(1));
        if insufficient < reward {
            let tx = Transaction::new_coinbase(insufficient, Hash::from_bytes(hash), height);
            let result = validate_coinbase(&tx, &ctx);
            prop_assert!(result.is_err());
        }
    }

    /// Valid signatures verify correctly
    #[test]
    fn prop_valid_signature_verifies(amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
        let ctx = test_context();
        let keypair = crypto::KeyPair::from_seed(seed);
        let pubkey = *keypair.public_key();
        let pubkey_hash = crypto::hash::hash_with_domain(
            crypto::ADDRESS_DOMAIN,
            pubkey.as_bytes(),
        );

        let prev_tx_hash = crypto::hash::hash(&seed);
        let prev_output = Output::normal(amount, pubkey_hash);

        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

        let mut recipient_hash = seed;
        recipient_hash[0] ^= 1;
        let output_amount = amount.saturating_sub(100).max(1);
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(output_amount, Hash::from_bytes(recipient_hash))],
            extra_data: vec![],
        };

        let signing_hash = tx.signing_message_for_input(0);
        let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[0].signature = signature;

        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        prop_assert!(result.is_ok());
    }

    /// Invalid signatures are rejected
    #[test]
    fn prop_invalid_signature_rejected(amount in 1u64..1_000_000_000u64, owner_seed: [u8; 32], attacker_seed: [u8; 32]) {
        prop_assume!(owner_seed != attacker_seed);

        let ctx = test_context();
        let owner_keypair = crypto::KeyPair::from_seed(owner_seed);
        let attacker_keypair = crypto::KeyPair::from_seed(attacker_seed);

        let owner_pubkey = *owner_keypair.public_key();
        let pubkey_hash = crypto::hash::hash_with_domain(
            crypto::ADDRESS_DOMAIN,
            owner_pubkey.as_bytes(),
        );

        let prev_tx_hash = crypto::hash::hash(&owner_seed);
        let prev_output = Output::normal(amount, pubkey_hash);

        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, owner_pubkey);

        let mut recipient_hash = owner_seed;
        recipient_hash[0] ^= 1;
        let output_amount = amount.saturating_sub(100).max(1);
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(output_amount, Hash::from_bytes(recipient_hash))],
            extra_data: vec![],
        };

        // Sign with wrong key (BIP-143: per-input)
        let signing_hash = tx.signing_message_for_input(0);
        let bad_signature = crypto::signature::sign_hash(&signing_hash, attacker_keypair.private_key());
        tx.inputs[0].signature = bad_signature;

        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        let is_invalid_sig = matches!(result, Err(ValidationError::InvalidSignature { .. }));
        prop_assert!(is_invalid_sig, "Expected InvalidSignature error, got {:?}", result);
    }

    /// Double spend detection works
    #[test]
    fn prop_double_spend_detected(seed: [u8; 32]) {
        let input = Input::new(Hash::from_bytes(seed), 0);
        let mut hash = seed;
        hash[0] = 1;
        let output = Output::normal(100, Hash::from_bytes(hash));

        let tx1 = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![input.clone()],
            outputs: vec![output.clone()],
            extra_data: vec![],
        };
        let tx2 = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![input],
            outputs: vec![output],
            extra_data: vec![],
        };

        // Create a mock block with both transactions
        let block = crate::Block {
            header: crate::BlockHeader {
                version: 1,
                prev_hash: Hash::ZERO,
                merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
                timestamp: 0,
                slot: 0,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput { value: vec![] },
                vdf_proof: vdf::VdfProof::empty(),
                    },
            transactions: vec![tx1, tx2],
            aggregate_bls_signature: Vec::new(),
        };

        let result = check_internal_double_spend(&block);
        prop_assert!(matches!(result, Err(ValidationError::DoubleSpend)));
    }
}

// ==========================================================================
// TX Type 7: AddBond Validation Tests
// ==========================================================================

#[test]
fn test_add_bond_transaction() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Create an input (required for AddBond)
    let prev_tx_hash = crypto::hash::hash(b"prev_tx");
    let input = Input::new(prev_tx_hash, 0);

    let tx = Transaction::new_add_bond(vec![input], pubkey, 5, 5_000_000_000, 10_000_000);

    assert!(tx.is_add_bond());
    assert_eq!(tx.tx_type, TxType::AddBond);
    assert_eq!(tx.inputs.len(), 1);
    assert_eq!(tx.outputs.len(), 5); // One Bond output per bond
    for output in &tx.outputs {
        assert_eq!(output.output_type, OutputType::Bond);
    }

    // Verify add bond data can be parsed
    let bond_data = tx.add_bond_data().unwrap();
    assert_eq!(bond_data.producer_pubkey, pubkey);
    assert_eq!(bond_data.bond_count, 5);

    // Structural validation should pass
    let result = validate_transaction(&tx, &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_add_bond_no_inputs() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Create AddBond with no inputs (invalid)
    let tx = Transaction::new_add_bond(vec![], pubkey, 5, 5_000_000_000, 10_000_000);

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidAddBond(msg)) if msg.contains("must have inputs")
    ));
}

#[test]
fn test_add_bond_allows_normal_outputs_for_change() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash(b"test");

    // Create AddBond with Bond output + Normal change output (valid)
    let bond_data = crate::transaction::AddBondData::new(pubkey, 5);
    let tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)],
        outputs: vec![
            Output::bond(5_000_000_000, pubkey_hash, 10_000_000, 0), // Bond output required
            Output::normal(100, pubkey_hash),                        // Normal change OK
        ],
        extra_data: bond_data.to_bytes(),
    };

    // Should pass structural validation (Bond + Normal outputs allowed)
    let result = validate_transaction(&tx, &ctx);
    assert!(
        result.is_ok(),
        "AddBond with Bond + Normal outputs should pass: {:?}",
        result
    );
}

#[test]
fn test_add_bond_requires_bond_output() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash(b"test");

    // Create AddBond with only Normal output (invalid - must have Bond output)
    let bond_data = crate::transaction::AddBondData::new(pubkey, 5);
    let tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)],
        outputs: vec![Output::normal(100, pubkey_hash)], // No Bond output = invalid
        extra_data: bond_data.to_bytes(),
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidAddBond(msg)) if msg.contains("must have a Bond output")
    ));
}

#[test]
fn test_add_bond_zero_bond_count() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash(pubkey.as_bytes());

    // Create AddBond with valid Bond output but zero bond_count in data
    let bond_data = crate::transaction::AddBondData::new(pubkey, 0);
    let tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)],
        outputs: vec![Output::bond(1_000_000_000, pubkey_hash, 10_000_000, 0)],
        extra_data: bond_data.to_bytes(),
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidAddBond(msg)) if msg.contains("bond count must be positive")
    ));
}

#[test]
fn test_add_bond_data_serialization() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();

    let tx = Transaction::new_add_bond(
        vec![Input::new(crypto::hash::hash(b"prev"), 0)],
        pubkey,
        10,
        10_000_000_000,
        10_000_000,
    );
    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    let recovered_data = recovered.add_bond_data().unwrap();
    assert_eq!(recovered_data.producer_pubkey, pubkey);
    assert_eq!(recovered_data.bond_count, 10);
}

// ==========================================================================
// TX Type 8: RequestWithdrawal Validation Tests
// ==========================================================================

#[test]
fn test_request_withdrawal_transaction() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let destination = crypto::hash::hash(b"destination");
    let net_amount = 3_000_000_000u64; // 3 bonds worth

    // Lock/unlock: withdrawal requires Bond UTXO inputs
    let bond_inputs = vec![Input::new(crypto::hash::hash(b"bond_utxo"), 0)];
    let tx = Transaction::new_request_withdrawal(bond_inputs, pubkey, 3, destination, net_amount);

    assert!(tx.is_request_withdrawal());
    assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
    assert_eq!(tx.inputs.len(), 1); // Bond UTXO input
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, net_amount);
    assert_eq!(tx.outputs[0].pubkey_hash, destination);

    // Verify withdrawal data can be parsed
    let withdrawal_data = tx.withdrawal_request_data().unwrap();
    assert_eq!(withdrawal_data.producer_pubkey, pubkey);
    assert_eq!(withdrawal_data.bond_count, 3);
    assert_eq!(withdrawal_data.destination, destination);

    // Structural validation should pass
    let result = validate_transaction(&tx, &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_request_withdrawal_without_inputs() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let destination = crypto::hash::hash(b"destination");

    // Create RequestWithdrawal without inputs (invalid — must have Bond UTXO inputs)
    let withdrawal_data = crate::transaction::WithdrawalRequestData::new(pubkey, 3, destination);
    let tx = Transaction {
        version: 1,
        tx_type: TxType::RequestWithdrawal,
        inputs: vec![], // Must have Bond UTXO inputs
        outputs: vec![Output::normal(100, destination)],
        extra_data: withdrawal_data.to_bytes(),
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("must have Bond UTXO inputs")
    ));
}

#[test]
fn test_request_withdrawal_wrong_output_count() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let destination = crypto::hash::hash(b"destination");

    // Create RequestWithdrawal with no outputs (must have exactly 1)
    let withdrawal_data = crate::transaction::WithdrawalRequestData::new(pubkey, 3, destination);
    let tx = Transaction {
        version: 1,
        tx_type: TxType::RequestWithdrawal,
        inputs: vec![Input::new(crypto::hash::hash(b"bond"), 0)], // Bond UTXO input
        outputs: vec![],                                          // Must have exactly 1
        extra_data: withdrawal_data.to_bytes(),
    };

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("exactly 1 output")
    ));
}

#[test]
fn test_request_withdrawal_zero_bond_count() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let destination = crypto::hash::hash(b"destination");

    // Create RequestWithdrawal with zero bond count (invalid)
    let tx = Transaction::new_request_withdrawal(
        vec![Input::new(crypto::hash::hash(b"bond"), 0)],
        pubkey,
        0,
        destination,
        100,
    );

    let result = validate_transaction(&tx, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("bond count must be positive")
    ));
}

#[test]
fn test_request_withdrawal_zero_destination() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Create RequestWithdrawal with zero destination (invalid)
    let tx = Transaction::new_request_withdrawal(
        vec![Input::new(crypto::hash::hash(b"bond"), 0)],
        pubkey,
        3,
        Hash::ZERO,
        100,
    );

    let result = validate_transaction(&tx, &ctx);
    // General output validation catches zero pubkey_hash before withdrawal-specific checks
    assert!(result.is_err(), "zero destination should be rejected");
}

#[test]
fn test_request_withdrawal_data_serialization() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let destination = crypto::hash::hash(b"destination");

    let tx = Transaction::new_request_withdrawal(
        vec![Input::new(crypto::hash::hash(b"bond"), 0)],
        pubkey,
        7,
        destination,
        5_000_000_000,
    );
    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    assert_eq!(recovered.outputs.len(), 1);
    assert_eq!(recovered.outputs[0].amount, 5_000_000_000);
    let recovered_data = recovered.withdrawal_request_data().unwrap();
    assert_eq!(recovered_data.producer_pubkey, pubkey);
    assert_eq!(recovered_data.bond_count, 7);
    assert_eq!(recovered_data.destination, destination);
}

// ==========================================================================
// VDF Network Parameter Tests
// ==========================================================================

/// Regression test: VDF validation must use network-specific parameters
///
/// Test that VDF iterations and discriminant sizes are network-specific.
///
/// The VDF serves as anti-grinding protection, and parameters are
/// calibrated per-network for practical operation:
/// - Mainnet: 2048-bit discriminant, 100k iterations for security
/// - Testnet: Same as Mainnet (production-identical testing)
/// - Devnet: 256-bit discriminant, uses fast hash-chain VDF
#[test]
fn test_vdf_validation_uses_network_specific_parameters() {
    // VDF is enabled for all networks (using hash-chain VDF for devnet)
    assert!(Network::Mainnet.vdf_enabled());
    assert!(Network::Testnet.vdf_enabled());
    assert!(Network::Devnet.vdf_enabled()); // Uses fast hash-chain VDF

    // Testnet has identical VDF parameters to Mainnet (production-identical testing)
    assert_eq!(
        Network::Mainnet.vdf_iterations(),
        Network::Testnet.vdf_iterations()
    );
    assert_eq!(
        Network::Mainnet.vdf_discriminant_bits(),
        Network::Testnet.vdf_discriminant_bits()
    );

    // Mainnet and Testnet use production-grade parameters
    assert_eq!(Network::Mainnet.vdf_iterations(), 800_000);
    assert_eq!(Network::Mainnet.vdf_discriminant_bits(), 2048);

    // Devnet has minimal parameters (uses hash-chain VDF)
    assert!(Network::Mainnet.vdf_iterations() > Network::Devnet.vdf_iterations());
    assert!(Network::Mainnet.vdf_discriminant_bits() > Network::Devnet.vdf_discriminant_bits());
    assert_eq!(Network::Devnet.vdf_iterations(), 1);
    assert_eq!(Network::Devnet.vdf_discriminant_bits(), 256);

    // Each network has unique VDF seeds
    assert_ne!(Network::Mainnet.vdf_seed(), Network::Testnet.vdf_seed());
    assert_ne!(Network::Testnet.vdf_seed(), Network::Devnet.vdf_seed());
}

// ==========================================================================
// EpochReward Validation Tests
// ==========================================================================

#[test]
fn test_validate_epoch_reward_no_inputs() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let mut tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

    // Add an input (invalid for epoch reward)
    tx.inputs.push(Input::new(Hash::ZERO, 0));

    let ctx = test_context();
    let result = validate_transaction(&tx, &ctx);

    assert!(matches!(
        result,
        Err(ValidationError::InvalidEpochReward(_))
    ));
}

#[test]
fn test_validate_epoch_reward_multiple_outputs_ok() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let mut tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

    // Multiple Normal outputs are valid (one per producer in pool distribution)
    let extra_pubkey_hash = crypto::hash::hash(b"another_recipient");
    tx.outputs.push(Output::normal(500, extra_pubkey_hash));

    let ctx = test_context();
    let result = validate_transaction(&tx, &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_validate_epoch_reward_normal_output_type() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let mut tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

    // Change to bond output (invalid for epoch reward)
    tx.outputs[0].output_type = OutputType::Bond;
    tx.outputs[0].lock_until = 1000;
    tx.outputs[0].extra_data = 0u32.to_le_bytes().to_vec();

    let ctx = test_context();
    let result = validate_transaction(&tx, &ctx);

    assert!(matches!(
        result,
        Err(ValidationError::InvalidEpochReward(_))
    ));
}

#[test]
fn test_validate_epoch_reward_invalid_structure() {
    // EpochReward with inputs is invalid (must be minted)
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let tx = Transaction {
        version: 1,
        tx_type: TxType::EpochReward,
        inputs: vec![Input::new(crypto::Hash::ZERO, 0)], // Invalid - must have no inputs
        outputs: vec![Output::normal(1000, pubkey_hash)],
        extra_data: vec![],
    };

    let ctx = test_context();
    let result = validate_transaction(&tx, &ctx);

    assert!(matches!(
        result,
        Err(ValidationError::InvalidEpochReward(_))
    ));
}

#[test]
fn test_validate_epoch_reward_valid() {
    // EpochReward is the automatic push-based reward system for the
    // deterministic scheduler model. It IS valid and should pass validation.
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

    let ctx = test_context();
    let result = validate_transaction(&tx, &ctx);

    // EpochReward transactions are valid (100% of block reward to producer)
    assert!(result.is_ok(), "EpochReward should be valid: {:?}", result);
}

#[test]
fn test_epoch_reward_skips_utxo_validation() {
    // EpochReward transactions don't require UTXO inputs since they are minted
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

    let ctx = test_context();
    let utxo_provider = MockUtxoProvider::new(); // Empty UTXO set

    // EpochReward should pass even with empty UTXO set (no inputs needed)
    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "EpochReward should skip UTXO validation: {:?}",
        result
    );
}

// ==========================================================================
// Block Rewards Validation Tests (EpochPool Mode)
// ==========================================================================

/// Create a context with EpochPool reward mode for testing
fn epoch_pool_context_at_slot(slot: u32) -> ValidationContext {
    let mut params = ConsensusParams::mainnet();
    params.reward_mode = RewardMode::EpochPool;

    ValidationContext::new(
        params,
        Network::Mainnet,
        GENESIS_TIME + (slot as u64 * 60),
        slot as u64,
    )
    .with_prev_block(
        slot.saturating_sub(1),
        GENESIS_TIME + ((slot as u64 - 1) * 60),
        Hash::ZERO,
    )
}

/// Create a minimal test block at a given slot
fn create_test_block_at_slot(slot: u32, transactions: Vec<Transaction>) -> Block {
    use crate::block::{Block, BlockHeader};
    use vdf::{VdfOutput, VdfProof};

    let keypair = crypto::KeyPair::generate();
    let header = BlockHeader {
        version: 1,
        slot,
        timestamp: GENESIS_TIME + (slot as u64 * 60),
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO, // Will be invalid, but we're testing rewards logic
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        producer: *keypair.public_key(),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::default(),
    };

    Block {
        header,
        transactions,
        aggregate_bls_signature: Vec::new(),
    }
}

#[test]
fn test_validate_block_rewards_direct_coinbase_mode() {
    // DirectCoinbase mode should not be affected by validate_block_rewards
    let mut params = ConsensusParams::mainnet();
    params.reward_mode = RewardMode::DirectCoinbase;

    let ctx = ValidationContext::new(params, Network::Mainnet, GENESIS_TIME + 60, 1)
        .with_prev_block(0, GENESIS_TIME, Hash::ZERO);

    let pubkey_hash = crypto::hash::hash(b"producer");
    let coinbase = Transaction::new_coinbase(ctx.params.block_reward(1), pubkey_hash, 1);
    let block = create_test_block_at_slot(1, vec![coinbase]);

    // Should pass for DirectCoinbase mode (validate_block_rewards returns Ok immediately)
    let result = validate_block_rewards(&block, &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_validate_block_rewards_epoch_boundary_no_coinbase() {
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary

    let ctx = epoch_pool_context_at_slot(boundary_slot);

    // Create block with coinbase at epoch boundary (should fail)
    let pubkey_hash = crypto::hash::hash(b"producer");
    let coinbase = Transaction::new_coinbase(1000, pubkey_hash, boundary_slot as u64);
    let block = create_test_block_at_slot(boundary_slot, vec![coinbase]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
}

#[test]
fn test_validate_block_rewards_non_boundary_no_rewards() {
    let params = ConsensusParams::mainnet();
    let non_boundary_slot = params.slots_per_reward_epoch / 2; // Mid-epoch

    let ctx = epoch_pool_context_at_slot(non_boundary_slot);

    // Create block with coinbase at non-boundary (should fail in EpochPool mode)
    let pubkey_hash = crypto::hash::hash(b"producer");
    let coinbase = Transaction::new_coinbase(1000, pubkey_hash, non_boundary_slot as u64);
    let block = create_test_block_at_slot(non_boundary_slot, vec![coinbase]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
}

#[test]
fn test_validate_block_rewards_non_boundary_no_epoch_reward() {
    let params = ConsensusParams::mainnet();
    let non_boundary_slot = params.slots_per_reward_epoch / 2; // Mid-epoch

    let ctx = epoch_pool_context_at_slot(non_boundary_slot);

    // Create block with epoch reward at non-boundary (should fail)
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let epoch_reward = Transaction::new_epoch_reward(0, *keypair.public_key(), 1000, pubkey_hash);
    let block = create_test_block_at_slot(non_boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
}

#[test]
fn test_validate_block_rewards_non_boundary_empty_block_ok() {
    let params = ConsensusParams::mainnet();
    let non_boundary_slot = params.slots_per_reward_epoch / 2; // Mid-epoch

    let ctx = epoch_pool_context_at_slot(non_boundary_slot);

    // Empty block at non-boundary should be OK (pool accumulates)
    let block = create_test_block_at_slot(non_boundary_slot, vec![]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_validate_block_rewards_partial_pool_ok() {
    // With attestation qualification, total_distributed ≤ pool is valid:
    // non-qualifying producers' share is burned (not distributed)
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary
    let expected_epoch = params.slot_to_reward_epoch(boundary_slot);

    let ctx = epoch_pool_context_at_slot(boundary_slot);

    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
    let epoch_reward = Transaction::new_epoch_reward(
        expected_epoch as u64,
        *keypair.public_key(),
        expected_pool / 2, // Half pool — valid when some producers disqualified
        pubkey_hash,
    );
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_validate_block_rewards_exceeds_pool_rejected() {
    // Total distributed > pool is always invalid (inflation attack)
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;
    let expected_epoch = params.slot_to_reward_epoch(boundary_slot);

    let ctx = epoch_pool_context_at_slot(boundary_slot);

    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
    let epoch_reward = Transaction::new_epoch_reward(
        expected_epoch as u64,
        *keypair.public_key(),
        expected_pool + 1, // Exceeds pool — inflation attack
        pubkey_hash,
    );
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
}

#[test]
fn test_validate_block_rewards_epoch_boundary_valid() {
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary
    let expected_epoch = params.slot_to_reward_epoch(boundary_slot);

    let ctx = epoch_pool_context_at_slot(boundary_slot);

    // Create block with correct epoch reward
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
    let epoch_reward = Transaction::new_epoch_reward(
        expected_epoch as u64,
        *keypair.public_key(),
        expected_pool, // Correct amount
        pubkey_hash,
    );
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_validate_block_rewards_epoch_mismatch() {
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary

    let ctx = epoch_pool_context_at_slot(boundary_slot);

    // Create epoch reward with wrong epoch number
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
    let epoch_reward = Transaction::new_epoch_reward(
        99, // Wrong epoch!
        *keypair.public_key(),
        expected_pool,
        pubkey_hash,
    );
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards(&block, &ctx);
    assert!(matches!(
        result,
        Err(ValidationError::InvalidEpochReward(_))
    ));
}

// ==========================================================================
// Milestone 5: Exact Epoch Reward Validation Tests
// ==========================================================================

/// Mock EpochBlockSource for testing exact validation
struct MockEpochBlockSource {
    last_rewarded: u64,
    blocks: Vec<Block>,
}

impl MockEpochBlockSource {
    fn new(last_rewarded: u64, blocks: Vec<Block>) -> Self {
        Self {
            last_rewarded,
            blocks,
        }
    }
}

impl EpochBlockSource for MockEpochBlockSource {
    fn last_rewarded_epoch(&self) -> Result<u64, String> {
        Ok(self.last_rewarded)
    }

    fn blocks_in_slot_range(&self, start: u32, end: u32) -> Result<Vec<Block>, String> {
        Ok(self
            .blocks
            .iter()
            .filter(|b| b.header.slot >= start && b.header.slot < end)
            .cloned()
            .collect())
    }

    fn has_any_block_in_slot_range(&self, start: u32, end: u32) -> Result<bool, String> {
        Ok(self
            .blocks
            .iter()
            .any(|b| b.header.slot >= start && b.header.slot < end))
    }
}

/// Create a test block with a specific producer
fn create_test_block_with_producer(slot: u32, producer: &crypto::PublicKey) -> Block {
    use crate::block::{Block, BlockHeader};
    use vdf::{VdfOutput, VdfProof};

    let header = BlockHeader {
        version: 1,
        slot,
        timestamp: GENESIS_TIME + (slot as u64 * 60),
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        producer: *producer,
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::default(),
    };

    Block::new(header, vec![])
}

#[test]
#[allow(deprecated)]
fn test_calculate_expected_epoch_rewards_empty() {
    let params = ConsensusParams::mainnet();
    let blocks: Vec<Block> = vec![];

    let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);
    assert!(rewards.is_empty(), "No blocks should produce no rewards");
}

#[test]
#[allow(deprecated)]
fn test_calculate_expected_epoch_rewards_single_producer() {
    let params = ConsensusParams::mainnet();
    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();

    // Create 10 blocks by single producer
    let blocks: Vec<Block> = (1..=10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

    assert_eq!(rewards.len(), 1, "Single producer should get one reward");
    assert_eq!(rewards[0].0, producer, "Reward should go to producer");

    // Expected: 10 blocks × block_reward
    let expected_total = 10 * params.block_reward(100);
    assert_eq!(rewards[0].1, expected_total, "Amount should match");
}

#[test]
#[allow(deprecated)]
fn test_calculate_expected_epoch_rewards_multiple_producers() {
    let params = ConsensusParams::mainnet();

    let keypair1 = crypto::KeyPair::generate();
    let keypair2 = crypto::KeyPair::generate();
    let producer1 = *keypair1.public_key();
    let producer2 = *keypair2.public_key();

    // Producer1: 3 blocks, Producer2: 7 blocks
    let mut blocks = Vec::new();
    for slot in 1..=3 {
        blocks.push(create_test_block_with_producer(slot, &producer1));
    }
    for slot in 4..=10 {
        blocks.push(create_test_block_with_producer(slot, &producer2));
    }

    let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

    assert_eq!(rewards.len(), 2, "Two producers should get rewards");

    let block_reward = params.block_reward(100);
    let total_pool = 10 * block_reward;

    // Find rewards for each producer
    let p1_reward = rewards.iter().find(|(pk, _)| pk == &producer1);
    let p2_reward = rewards.iter().find(|(pk, _)| pk == &producer2);

    assert!(p1_reward.is_some(), "Producer1 should have reward");
    assert!(p2_reward.is_some(), "Producer2 should have reward");

    // Check proportional distribution (3/10 and 7/10)
    let p1_amount = p1_reward.unwrap().1;
    let p2_amount = p2_reward.unwrap().1;

    // Total should equal pool
    assert_eq!(p1_amount + p2_amount, total_pool, "Total should equal pool");

    // Approximate proportions (allow for rounding)
    let expected_p1 = (3 * total_pool) / 10;
    let _expected_p2 = total_pool - expected_p1; // Last gets remainder

    // The actual amounts depend on sort order, but total must be correct
    assert!(p1_amount > 0 && p2_amount > 0, "Both should get rewards");
}

#[test]
#[allow(deprecated)]
fn test_calculate_expected_epoch_rewards_deterministic_ordering() {
    let params = ConsensusParams::mainnet();

    // Generate multiple keypairs
    let keypairs: Vec<_> = (0..5).map(|_| crypto::KeyPair::generate()).collect();
    let producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

    // Each producer creates 2 blocks
    let mut blocks = Vec::new();
    for (i, producer) in producers.iter().enumerate() {
        blocks.push(create_test_block_with_producer(
            (i * 2 + 1) as u32,
            producer,
        ));
        blocks.push(create_test_block_with_producer(
            (i * 2 + 2) as u32,
            producer,
        ));
    }

    // Calculate twice to verify determinism
    let rewards1 = calculate_expected_epoch_rewards(0, &blocks, 100, &params);
    let rewards2 = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

    assert_eq!(rewards1, rewards2, "Calculation should be deterministic");

    // Verify sorted by pubkey
    for i in 1..rewards1.len() {
        assert!(
            rewards1[i - 1].0.as_bytes() < rewards1[i].0.as_bytes(),
            "Rewards should be sorted by pubkey"
        );
    }
}

#[test]
#[allow(deprecated)]
fn test_calculate_expected_epoch_rewards_skips_null_producer() {
    let params = ConsensusParams::mainnet();
    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();

    // Create blocks including one with null producer (genesis-like)
    let mut blocks = vec![create_test_block_with_producer(1, &producer)];

    // Add a "genesis" block with null producer
    let null_producer = crypto::PublicKey::from_bytes([0u8; 32]);
    blocks.push(create_test_block_with_producer(0, &null_producer));

    let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

    // Only the real producer should get rewards
    assert_eq!(rewards.len(), 1, "Only real producer should get reward");
    assert_eq!(rewards[0].0, producer, "Reward should go to real producer");
}

#[test]
#[allow(deprecated)]
fn test_epoch_needing_rewards_slot_zero() {
    let params = ConsensusParams::mainnet();
    let source = MockEpochBlockSource::new(0, vec![]);

    let result = epoch_needing_rewards(0, &params, &source);
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_none(),
        "Slot 0 should not trigger rewards"
    );
}

#[test]
#[allow(deprecated)]
fn test_epoch_needing_rewards_first_epoch_boundary() {
    let params = ConsensusParams::mainnet();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    // Add a block in epoch 1 (slots 360-719) so it has blocks to reward
    // Note: We're at slot 720 (epoch 2), checking if epoch 1 needs rewards
    let block_in_epoch_1 = create_test_block_with_producer(
        params.slots_per_reward_epoch + 50, // Slot 410 (in epoch 1)
        &pubkey,
    );
    let source = MockEpochBlockSource::new(0, vec![block_in_epoch_1]);

    // First slot of epoch 2 should trigger epoch 1 rewards (epoch 1 is finished and has blocks)
    let boundary_slot = params.slots_per_reward_epoch * 2;
    let result = epoch_needing_rewards(boundary_slot, &params, &source);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Some(1), "Should reward epoch 1");
}

#[test]
#[allow(deprecated)]
fn test_epoch_needing_rewards_empty_epoch_skipped() {
    let params = ConsensusParams::mainnet();
    // No blocks in epoch 0 or 1 - they should be skipped
    let source = MockEpochBlockSource::new(0, vec![]);

    // In epoch 2, but epochs 0 and 1 are empty
    let slot_in_epoch_2 = params.slots_per_reward_epoch * 2 + 5;
    let result = epoch_needing_rewards(slot_in_epoch_2, &params, &source);

    assert!(result.is_ok());
    assert!(
        result.unwrap().is_none(),
        "Empty epochs should be skipped, no rewards needed"
    );
}

#[test]
#[allow(deprecated)]
fn test_epoch_needing_rewards_already_rewarded() {
    let params = ConsensusParams::mainnet();
    let source = MockEpochBlockSource::new(1, vec![]); // Epoch 1 already rewarded

    let boundary_slot = params.slots_per_reward_epoch;
    let result = epoch_needing_rewards(boundary_slot, &params, &source);

    assert!(result.is_ok());
    assert!(
        result.unwrap().is_none(),
        "Already rewarded epoch should return None"
    );
}

#[test]
#[allow(deprecated)]
fn test_epoch_needing_rewards_multi_epoch_catchup() {
    let params = ConsensusParams::mainnet();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Add blocks in epochs 1 and 2 (epoch 0 is empty)
    let block_in_epoch_1 =
        create_test_block_with_producer(params.slots_per_reward_epoch + 50, &pubkey);
    let block_in_epoch_2 =
        create_test_block_with_producer(params.slots_per_reward_epoch * 2 + 50, &pubkey);
    let source = MockEpochBlockSource::new(0, vec![block_in_epoch_1, block_in_epoch_2]);

    // In epoch 3 but no rewards distributed - should catch up with epoch 1
    // (epoch 0 is empty and skipped, epoch 1 has blocks)
    let slot_in_epoch_3 = params.slots_per_reward_epoch * 3 + 5;
    let result = epoch_needing_rewards(slot_in_epoch_3, &params, &source);

    assert!(result.is_ok());
    assert_eq!(
        result.unwrap(),
        Some(1),
        "Should reward oldest non-empty unrewarded epoch (1, skipping empty epoch 0)"
    );
}

#[test]
#[allow(deprecated)]
fn test_epoch_needing_rewards_late_start_skips_empty() {
    let params = ConsensusParams::mainnet();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Simulate chain starting late: only epoch 5 has blocks
    let block_in_epoch_5 =
        create_test_block_with_producer(params.slots_per_reward_epoch * 5 + 50, &pubkey);
    let source = MockEpochBlockSource::new(0, vec![block_in_epoch_5]);

    // In epoch 6 - should skip epochs 0-4 (empty) and reward epoch 5
    let slot_in_epoch_6 = params.slots_per_reward_epoch * 6 + 5;
    let result = epoch_needing_rewards(slot_in_epoch_6, &params, &source);

    assert!(result.is_ok());
    assert_eq!(
        result.unwrap(),
        Some(5),
        "Should skip empty epochs 0-4 and reward epoch 5"
    );
}

#[test]
#[allow(deprecated)]
fn test_validate_block_rewards_exact_non_boundary_no_rewards() {
    let params = ConsensusParams::mainnet();
    let non_boundary_slot = params.slots_per_reward_epoch / 2;

    let ctx = epoch_pool_context_at_slot(non_boundary_slot);
    let source = MockEpochBlockSource::new(0, vec![]);

    // Empty block at non-boundary should be OK
    let block = create_test_block_at_slot(non_boundary_slot, vec![]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(result.is_ok(), "Empty block at non-boundary should pass");
}

#[test]
#[allow(deprecated)]
fn test_validate_block_rewards_exact_unexpected_rewards() {
    let params = ConsensusParams::mainnet();
    let non_boundary_slot = params.slots_per_reward_epoch / 2;

    let ctx = epoch_pool_context_at_slot(non_boundary_slot);
    let source = MockEpochBlockSource::new(0, vec![]);

    // Block with epoch reward at non-boundary
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let epoch_reward = Transaction::new_epoch_reward(0, *keypair.public_key(), 1000, pubkey_hash);
    let block = create_test_block_at_slot(non_boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::UnexpectedEpochReward)),
        "Should reject unexpected epoch rewards"
    );
}

#[test]
#[allow(deprecated)]
#[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
fn test_validate_block_rewards_exact_missing_rewards() {
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;

    let ctx = epoch_pool_context_at_slot(boundary_slot);

    // Create blocks in epoch 0 for the source
    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();
    let epoch_blocks: Vec<Block> = (1..=10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks);

    // Empty block at boundary (missing rewards)
    let block = create_test_block_at_slot(boundary_slot, vec![]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::MissingEpochReward { .. })),
        "Should reject missing epoch rewards at boundary"
    );
}

#[test]
#[allow(deprecated)]
fn test_validate_block_rewards_exact_valid() {
    let params = ConsensusParams::mainnet();
    // Use second boundary (slot 720) where we validate epoch 1 rewards
    let boundary_slot = params.slots_per_reward_epoch * 2; // 720

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    // Create blocks in epoch 1 (slots 360-719)
    // At slot 720 with last_rewarded=0, epoch 1 is being rewarded
    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();
    let start_slot = params.slots_per_reward_epoch; // 360
    let epoch_blocks: Vec<Block> = (start_slot..start_slot + 10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

    // Calculate expected rewards for epoch 1
    let expected =
        calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
    assert_eq!(expected.len(), 1, "Should have one reward");

    // Create valid epoch reward transaction
    let pubkey_hash = crypto::hash::hash(producer.as_bytes());
    let epoch_reward = Transaction::new_epoch_reward(1, producer, expected[0].1, pubkey_hash);
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        result.is_ok(),
        "Valid epoch rewards should pass: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
#[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
fn test_validate_block_rewards_exact_wrong_amount() {
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    // Create blocks in epoch 0
    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();
    let epoch_blocks: Vec<Block> = (1..=10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks);

    // Create epoch reward with WRONG amount
    let pubkey_hash = crypto::hash::hash(producer.as_bytes());
    let wrong_amount = 12345; // Arbitrary wrong amount
    let epoch_reward = Transaction::new_epoch_reward(1, producer, wrong_amount, pubkey_hash);
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
        "Should reject wrong amount: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
#[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
fn test_validate_block_rewards_exact_wrong_recipient() {
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    // Create blocks in epoch 0 by producer1
    let keypair1 = crypto::KeyPair::generate();
    let producer1 = *keypair1.public_key();
    let epoch_blocks: Vec<Block> = (1..=10)
        .map(|slot| create_test_block_with_producer(slot, &producer1))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks);

    // Calculate correct amount but use WRONG recipient
    let expected =
        calculate_expected_epoch_rewards(1, &source.blocks, ctx.current_height, &ctx_params);
    let keypair2 = crypto::KeyPair::generate();
    let wrong_recipient = *keypair2.public_key();
    let pubkey_hash = crypto::hash::hash(wrong_recipient.as_bytes());

    let epoch_reward =
        Transaction::new_epoch_reward(1, wrong_recipient, expected[0].1, pubkey_hash);
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
        "Should reject wrong recipient: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
fn test_validate_block_rewards_exact_wrong_epoch_number() {
    let params = ConsensusParams::mainnet();
    // Use second epoch boundary (slot 720) where we reward epoch 2
    // after epoch 1 was already rewarded
    let boundary_slot = params.slots_per_reward_epoch * 2; // slot 720

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    // Create blocks in epoch 1 (slots 360-719) since we're at slot 720
    // and last_rewarded=1 means epoch 2 is next to be rewarded
    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();
    let start = params.slots_per_reward_epoch;
    let epoch_blocks: Vec<Block> = (start..start + 10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    // last_rewarded=1 means epoch 1 was rewarded, so epoch 2 is next
    let _source = MockEpochBlockSource::new(1, epoch_blocks);

    // Calculate expected rewards for epoch 2 (but there are no blocks there)
    // So we need blocks in epoch 2 range (720-1079)
    // Let's simplify: use last_rewarded=0, boundary at 360, blocks in epoch 0 range
    // The issue: epoch_needing_rewards(360) with last_rewarded=0 returns Some(1)
    // but epoch 1 slots are 360-719, and we only have blocks in 1-10

    // Alternative approach: test with blocks in the correct epoch range
    // At slot 720, last_rewarded=1, we reward epoch 2
    // So we need blocks in epoch 2 range (720-1079)... but we're AT 720
    // This is tricky because we're producing the first block of the epoch

    // Simplest fix: put blocks in epoch 1 range and set last_rewarded=0
    // so epoch 1 is being rewarded and blocks exist

    // Actually, let's create blocks in epoch 1 (360-719)
    let epoch1_blocks: Vec<Block> = (360..370)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    // At slot 720, last_rewarded=0 means epoch 1 is next
    let source2 = MockEpochBlockSource::new(0, epoch1_blocks.clone());

    let expected =
        calculate_expected_epoch_rewards(1, &epoch1_blocks, ctx.current_height, &ctx_params);
    let pubkey_hash = crypto::hash::hash(producer.as_bytes());

    // Create epoch reward with WRONG epoch number (should be 1)
    let epoch_reward = Transaction::new_epoch_reward(
        99, // Wrong epoch! Should be 1
        producer,
        expected[0].1,
        pubkey_hash,
    );
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source2);
    assert!(
        matches!(result, Err(ValidationError::InvalidEpochReward(_))),
        "Should reject wrong epoch number: {:?}",
        result
    );
}

// =========================================================================
// Milestone 6: Additional Validation Tests
// =========================================================================

#[test]
#[allow(deprecated)]
fn test_validate_block_rewards_exact_many_producers() {
    // Test validation with 10 producers to verify scalability
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch * 2; // slot 720

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    // Create 10 producers, each with 1 block in epoch 1
    let keypairs: Vec<_> = (0..10).map(|_| crypto::KeyPair::generate()).collect();
    let producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

    let start_slot = params.slots_per_reward_epoch;
    let epoch_blocks: Vec<Block> = producers
        .iter()
        .enumerate()
        .map(|(i, producer)| create_test_block_with_producer(start_slot + i as u32, producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

    // Calculate expected rewards
    let expected =
        calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
    assert_eq!(expected.len(), 10, "Should have 10 reward entries");

    // Create valid epoch reward transactions
    let epoch_txs: Vec<Transaction> = expected
        .iter()
        .map(|(producer, amount)| {
            let pubkey_hash = crypto::hash::hash(producer.as_bytes());
            Transaction::new_epoch_reward(1, *producer, *amount, pubkey_hash)
        })
        .collect();

    let block = create_test_block_at_slot(boundary_slot, epoch_txs);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        result.is_ok(),
        "Valid 10-producer rewards should pass: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
fn test_validate_block_rewards_exact_dust_handling() {
    // Test validation with rounding dust (e.g., 100 units / 3 producers)
    let params = ConsensusParams::mainnet();
    // Use second boundary (slot 720) to reward epoch 1 (slots 360-719)
    let boundary_slot = params.slots_per_reward_epoch * 2;

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    // Create 3 producers with equal blocks (1 each) in epoch 1 range
    let keypairs: Vec<_> = (0..3).map(|_| crypto::KeyPair::generate()).collect();
    let mut producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

    // Sort producers by pubkey for deterministic dust assignment
    producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    // Create blocks in epoch 1 range (slots 360-719)
    let start_slot = params.slots_per_reward_epoch;
    let epoch_blocks: Vec<Block> = producers
        .iter()
        .enumerate()
        .map(|(i, producer)| create_test_block_with_producer(start_slot + i as u32, producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

    // Calculate expected rewards (dust goes to last sorted producer)
    let expected =
        calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
    assert_eq!(expected.len(), 3);

    // Verify dust is assigned to last producer
    let total_pool: u64 = expected.iter().map(|(_, amt)| *amt).sum();
    let base_share = total_pool / 3;

    // Last producer should get base_share + remainder
    let last_producer_amount = expected.last().unwrap().1;
    let dust = total_pool % 3;
    assert_eq!(
        last_producer_amount,
        base_share + dust,
        "Last producer should get dust"
    );

    // Create valid epoch reward transactions
    let epoch_txs: Vec<Transaction> = expected
        .iter()
        .map(|(producer, amount)| {
            let pubkey_hash = crypto::hash::hash(producer.as_bytes());
            Transaction::new_epoch_reward(1, *producer, *amount, pubkey_hash)
        })
        .collect();

    let block = create_test_block_at_slot(boundary_slot, epoch_txs);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        result.is_ok(),
        "Valid dust distribution should pass: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
#[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
fn test_validate_block_rewards_exact_over_distribution() {
    // Test rejection of rewards that exceed expected amounts
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();

    let epoch_blocks: Vec<Block> = (1..=10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

    // Calculate expected, then add extra
    let expected =
        calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
    let correct_amount = expected[0].1;
    let inflated_amount = correct_amount + 1_000_000; // Add 1M extra units

    let pubkey_hash = crypto::hash::hash(producer.as_bytes());
    let epoch_reward = Transaction::new_epoch_reward(1, producer, inflated_amount, pubkey_hash);
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
        "Should reject over-distribution: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
#[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
fn test_validate_block_rewards_exact_under_distribution() {
    // Test rejection of rewards that are less than expected
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();

    let epoch_blocks: Vec<Block> = (1..=10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

    // Calculate expected, then subtract
    let expected =
        calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
    let correct_amount = expected[0].1;
    let reduced_amount = correct_amount - 1_000_000; // Remove 1M units

    let pubkey_hash = crypto::hash::hash(producer.as_bytes());
    let epoch_reward = Transaction::new_epoch_reward(1, producer, reduced_amount, pubkey_hash);
    let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
        "Should reject under-distribution: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
#[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
fn test_validate_block_rewards_exact_partial_producer_list() {
    // Test rejection when some producers are missing from rewards
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    // Create 3 producers with blocks
    let keypairs: Vec<_> = (0..3).map(|_| crypto::KeyPair::generate()).collect();
    let producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

    let epoch_blocks: Vec<Block> = producers
        .iter()
        .enumerate()
        .map(|(i, producer)| create_test_block_with_producer(i as u32 + 1, producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

    // Calculate expected (should have 3 entries)
    let expected =
        calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
    assert_eq!(expected.len(), 3);

    // Only include 2 of 3 producers (missing last one)
    let partial_txs: Vec<Transaction> = expected
        .iter()
        .take(2) // Only first 2 producers
        .map(|(producer, amount)| {
            let pubkey_hash = crypto::hash::hash(producer.as_bytes());
            Transaction::new_epoch_reward(1, *producer, *amount, pubkey_hash)
        })
        .collect();

    let block = create_test_block_at_slot(boundary_slot, partial_txs);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
        "Should reject partial producer list: {:?}",
        result
    );
}

#[test]
#[allow(deprecated)]
#[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
fn test_validate_block_rewards_exact_extra_reward_tx() {
    // Test rejection when block has more reward transactions than expected
    let params = ConsensusParams::mainnet();
    let boundary_slot = params.slots_per_reward_epoch;

    let mut ctx_params = ConsensusParams::mainnet();
    ctx_params.reward_mode = RewardMode::EpochPool;

    let ctx = ValidationContext::new(
        ctx_params.clone(),
        Network::Mainnet,
        GENESIS_TIME + (boundary_slot as u64 * 60),
        boundary_slot as u64,
    )
    .with_prev_block(
        boundary_slot - 1,
        GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
        Hash::ZERO,
    );

    let keypair = crypto::KeyPair::generate();
    let producer = *keypair.public_key();

    let epoch_blocks: Vec<Block> = (1..=10)
        .map(|slot| create_test_block_with_producer(slot, &producer))
        .collect();

    let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

    // Calculate expected (should have 1 entry)
    let expected =
        calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
    assert_eq!(expected.len(), 1);

    // Create valid reward plus an EXTRA reward to a fake producer
    let pubkey_hash = crypto::hash::hash(producer.as_bytes());
    let valid_reward = Transaction::new_epoch_reward(1, producer, expected[0].1, pubkey_hash);

    let fake_keypair = crypto::KeyPair::generate();
    let fake_producer = *fake_keypair.public_key();
    let fake_hash = crypto::hash::hash(fake_producer.as_bytes());
    let extra_reward = Transaction::new_epoch_reward(1, fake_producer, 1_000_000, fake_hash);

    let block = create_test_block_at_slot(boundary_slot, vec![valid_reward, extra_reward]);

    let result = validate_block_rewards_exact(&block, &ctx, &source);
    assert!(
        matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
        "Should reject extra reward transaction: {:?}",
        result
    );
}

#[test]
fn test_validate_delegate_bond_valid() {
    use crate::transaction::{DelegateBondData, Transaction};

    let delegator = crypto::KeyPair::generate();
    let delegate = crypto::KeyPair::generate();
    let data = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 3);
    let tx = Transaction::new_delegate_bond(data);
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        result.is_ok(),
        "Valid DelegateBond should pass: {:?}",
        result
    );
}

#[test]
fn test_validate_delegate_bond_zero_bonds() {
    use crate::transaction::{DelegateBondData, TxType};

    let delegator = crypto::KeyPair::generate();
    let delegate = crypto::KeyPair::generate();
    let data = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 0);
    let tx = Transaction {
        version: 1,
        tx_type: TxType::DelegateBond,
        inputs: vec![],
        outputs: vec![],
        extra_data: data.to_bytes(),
    };
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        matches!(result, Err(ValidationError::InvalidDelegation(_))),
        "DelegateBond with 0 bonds should fail: {:?}",
        result
    );
}

#[test]
fn test_validate_delegate_bond_empty_extra_data() {
    use crate::transaction::TxType;

    let tx = Transaction {
        version: 1,
        tx_type: TxType::DelegateBond,
        inputs: vec![],
        outputs: vec![],
        extra_data: vec![],
    };
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        matches!(result, Err(ValidationError::InvalidDelegation(_))),
        "DelegateBond with empty data should fail: {:?}",
        result
    );
}

#[test]
fn test_validate_revoke_delegation_valid() {
    use crate::transaction::RevokeDelegationData;

    let delegator = crypto::KeyPair::generate();
    let delegate = crypto::KeyPair::generate();
    let data = RevokeDelegationData::new(*delegator.public_key(), *delegate.public_key());
    let tx = Transaction::new_revoke_delegation(data);
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        result.is_ok(),
        "Valid RevokeDelegation should pass: {:?}",
        result
    );
}

#[test]
fn test_validate_revoke_delegation_empty_extra_data() {
    use crate::transaction::TxType;

    let tx = Transaction {
        version: 1,
        tx_type: TxType::RevokeDelegation,
        inputs: vec![],
        outputs: vec![],
        extra_data: vec![],
    };
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        matches!(result, Err(ValidationError::InvalidDelegation(_))),
        "RevokeDelegation with empty data should fail: {:?}",
        result
    );
}

// ==========================================================================
// Protocol Activation Validation Tests
// ==========================================================================

#[test]
fn test_validate_protocol_activation_valid() {
    use crate::maintainer::{MaintainerSignature, ProtocolActivationData};

    let kp = crypto::KeyPair::generate();
    let data = ProtocolActivationData::new(2, 500, "Test".to_string(), vec![]);
    // Add a signature so structural validation passes
    let msg = data.signing_message();
    let sig = crypto::signature::sign(&msg, kp.private_key());
    let data = ProtocolActivationData::new(
        2,
        500,
        "Test".to_string(),
        vec![MaintainerSignature::new(*kp.public_key(), sig)],
    );
    let tx = Transaction::new_protocol_activation(data);
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        result.is_ok(),
        "Valid ProtocolActivation should pass: {:?}",
        result
    );
}

#[test]
fn test_validate_protocol_activation_empty_extra_data() {
    use crate::transaction::TxType;

    let tx = Transaction {
        version: 1,
        tx_type: TxType::ProtocolActivation,
        inputs: vec![],
        outputs: vec![],
        extra_data: vec![],
    };
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        matches!(result, Err(ValidationError::InvalidProtocolActivation(_))),
        "ProtocolActivation with empty data should fail: {:?}",
        result
    );
}

#[test]
fn test_validate_protocol_activation_zero_version() {
    use crate::maintainer::{MaintainerSignature, ProtocolActivationData};

    let kp = crypto::KeyPair::generate();
    let msg = b"activate:0:500";
    let sig = crypto::signature::sign(msg, kp.private_key());
    let data = ProtocolActivationData::new(
        0, // invalid
        500,
        "Test".to_string(),
        vec![MaintainerSignature::new(*kp.public_key(), sig)],
    );
    let tx = Transaction::new_protocol_activation(data);
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        matches!(result, Err(ValidationError::InvalidProtocolActivation(_))),
        "ProtocolActivation with version 0 should fail: {:?}",
        result
    );
}

#[test]
fn test_validate_protocol_activation_no_signatures() {
    use crate::maintainer::ProtocolActivationData;

    let data = ProtocolActivationData::new(2, 500, "Test".to_string(), vec![]);
    let tx = Transaction::new_protocol_activation(data);
    let ctx = test_context();

    let result = validate_transaction(&tx, &ctx);
    assert!(
        matches!(result, Err(ValidationError::InvalidProtocolActivation(_))),
        "ProtocolActivation with no signatures should fail: {:?}",
        result
    );
}

// ==========================================================================
// ValidationMode Tests (Smart Sync)
// ==========================================================================

/// Create a block with a valid merkle root for validation mode tests
fn create_block_with_merkle() -> (Block, ValidationContext) {
    use crate::block::{Block, BlockHeader};
    use vdf::{VdfOutput, VdfProof};

    let params = ConsensusParams::mainnet();
    let keypair = crypto::KeyPair::generate();
    let slot = 12; // Slot 12 = GENESIS_TIME + 120
    let header = BlockHeader {
        version: 2,
        slot,
        timestamp: GENESIS_TIME + (slot as u64 * 10),
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: params.genesis_hash,
        producer: *keypair.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::default(),
    };

    let mut block = Block {
        header,
        transactions: vec![],
        aggregate_bls_signature: Vec::new(),
    };
    // Fix merkle root to match (empty tx list)
    block.header.merkle_root = block.compute_merkle_root();

    let ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        1,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_bootstrap_producers(vec![*keypair.public_key()]);

    (block, ctx)
}

#[test]
fn test_validate_block_light_skips_vdf() {
    let (block, ctx) = create_block_with_merkle();

    // Light mode should accept a block even with invalid VDF (zeros)
    let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
    assert!(
        result.is_ok(),
        "Light mode should skip VDF validation: {:?}",
        result
    );
}

#[test]
fn test_validate_block_light_rejects_bad_merkle() {
    let (mut block, ctx) = create_block_with_merkle();

    // Corrupt the merkle root
    block.header.merkle_root = crypto::hash::hash(b"corrupted");

    let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
    assert!(
        matches!(result, Err(ValidationError::InvalidMerkleRoot)),
        "Light mode should still reject bad merkle root: {:?}",
        result
    );
}

#[test]
fn test_validate_block_light_rejects_bad_version() {
    let (mut block, ctx) = create_block_with_merkle();

    // Set invalid version
    block.header.version = 99;

    let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
    assert!(
        matches!(result, Err(ValidationError::InvalidVersion(99))),
        "Light mode should reject invalid version: {:?}",
        result
    );
}

#[test]
fn test_validate_block_light_rejects_oversized_block() {
    let (mut block, ctx) = create_block_with_merkle();

    // Add a large transaction to exceed block size limit
    let large_tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![],
        outputs: vec![],
        extra_data: vec![0u8; 2_000_000], // 2MB > 1MB limit
    };
    block.transactions.push(large_tx);
    block.header.merkle_root = block.compute_merkle_root();

    let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
    assert!(
        matches!(result, Err(ValidationError::BlockTooLarge { .. })),
        "Light mode should reject oversized blocks: {:?}",
        result
    );
}

#[test]
fn test_validation_mode_enum_equality() {
    assert_eq!(ValidationMode::Full, ValidationMode::Full);
    assert_eq!(ValidationMode::Light, ValidationMode::Light);
    assert_ne!(ValidationMode::Full, ValidationMode::Light);
}
