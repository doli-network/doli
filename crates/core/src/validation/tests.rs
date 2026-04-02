use super::*;
use crate::block::{Block, BlockHeader};
use crate::consensus::{ConsensusParams, GENESIS_TIME, TOTAL_SUPPLY};
use crate::network::Network;
use crate::transaction::{Input, Output, OutputType, Transaction, TxType};
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
        missed_producers: Vec::new(),
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
        missed_producers: Vec::new(),
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
        missed_producers: Vec::new(),
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
                missed_producers: Vec::new(),
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

    // Mainnet and Testnet use lightweight VDF (bond is the real Sybil protection)
    assert_eq!(Network::Mainnet.vdf_iterations(), 1_000);
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
        missed_producers: Vec::new(),
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

// ==================== Per-Byte Fee Validation Tests ====================

/// Test 8: Transfer with exact minimum fee (1 sat) passes validation
#[test]
fn test_fee_validation_transfer_exact_minimum() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"fee_test_8");
    // Input: 101 sats. Output: 100 sats. Fee: 1 sat = BASE_FEE. Passes.
    let prev_output = Output::normal(101, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let recipient = crypto::hash::hash(b"recipient_8");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(100, recipient)],
        extra_data: vec![],
    };

    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "Exact minimum fee should pass: {:?}",
        result
    );
}

/// Test 9: Transfer with zero fee (input == output) fails InsufficientFee
#[test]
fn test_fee_validation_transfer_zero_fee_rejected() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"fee_test_9");
    // Input: 100 sats. Output: 100 sats. Fee: 0. Rejected.
    let prev_output = Output::normal(100, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let recipient = crypto::hash::hash(b"recipient_9");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(100, recipient)],
        extra_data: vec![],
    };

    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        matches!(
            result,
            Err(ValidationError::InsufficientFee { actual: 0, .. })
        ),
        "Zero fee should be rejected: {:?}",
        result
    );
}

/// Test 10: AddBond with bond output (4 bytes extra_data) needs 5 sat fee
#[test]
fn test_fee_validation_add_bond_exact_fee() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"fee_test_10");
    let bond_amount: u64 = 1_000_000_000;
    // Input covers bond_amount + 1 sat fee (BASE_FEE + 4/100 = 1, integer division)
    let prev_output = Output::normal(bond_amount + 1, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    // Build AddBond TX with 1 bond output
    let bond_data = crate::transaction::AddBondData::new(pubkey, 1);
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::bond(bond_amount, pubkey_hash, u64::MAX, 0)],
        extra_data: bond_data.to_bytes(),
    };

    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    // Bond output has 4 bytes extra_data, so min fee = 1 + 4/100 = 1 (integer division)
    assert_eq!(tx.minimum_fee(), 1);
    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "AddBond with exact per-byte fee should pass: {:?}",
        result
    );
}

/// Test 11: AddBond with bond output but only 0 sat fee (needs 1) fails
#[test]
fn test_fee_validation_add_bond_insufficient_per_byte() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"fee_test_11");
    let bond_amount: u64 = 1_000_000_000;
    // Input covers bond_amount + 0 sats fee — but needs 1 (BASE_FEE; 4 bytes / 100 = 0)
    let prev_output = Output::normal(bond_amount, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let bond_data = crate::transaction::AddBondData::new(pubkey, 1);
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::bond(bond_amount, pubkey_hash, u64::MAX, 0)],
        extra_data: bond_data.to_bytes(),
    };

    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        matches!(
            result,
            Err(ValidationError::InsufficientFee {
                actual: 0,
                minimum: 1,
                ..
            })
        ),
        "AddBond with 0 sat fee (needs 1) should fail: {:?}",
        result
    );
}

/// Test 12: Overpaying fee is always fine (generous fee accepted)
#[test]
fn test_fee_validation_overpay_accepted() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"fee_test_12");
    // Input: 1000 sats. Output: 50 sats. Fee: 950 sats >> 1 sat minimum.
    let prev_output = Output::normal(1000, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let recipient = crypto::hash::hash(b"recipient_12");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(50, recipient)],
        extra_data: vec![],
    };

    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "Overpaying fee should be accepted: {:?}",
        result
    );
}

/// Test 13: Coinbase transactions bypass fee check entirely
#[test]
fn test_fee_validation_coinbase_bypass() {
    let ctx = test_context();
    let utxo_provider = MockUtxoProvider::new();

    // Coinbase: no inputs, single output, minted from thin air.
    // Must not trigger InsufficientFee.
    let coinbase = Transaction::new_coinbase(100_000_000, crypto::hash::hash(b"producer_13"), 1);

    let result = utxo::validate_transaction_with_utxos(&coinbase, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "Coinbase should bypass fee check: {:?}",
        result
    );
}

/// A5 (validation): minimum_fee consistency — validate_transaction_with_utxos uses
/// the same formula as Transaction::minimum_fee().
/// Uses a Bond output (4 bytes extra_data) to avoid covenant activation height issues.
#[test]
fn test_fee_validation_consistency_with_transaction_minimum_fee() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"fee_consistency");
    // Bond output: 4 bytes extra_data => minimum_fee = 1 + 4 = 5
    let bond_amount: u64 = 1_000_000_000;
    let min_fee: u64 = 1; // BASE_FEE + 4*1/100 = 1 (integer division)
    let prev_output = Output::normal(bond_amount + min_fee, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let bond_data = crate::transaction::AddBondData::new(pubkey, 1);
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::bond(bond_amount, pubkey_hash, u64::MAX, 0)],
        extra_data: bond_data.to_bytes(),
    };

    // Verify Transaction::minimum_fee() matches expected
    assert_eq!(tx.minimum_fee(), min_fee);

    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    // Validation should pass because fee exactly matches minimum
    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "Fee matching minimum_fee() should pass validation: {:?}",
        result
    );
}

/// A6 (validation): Fee check is deterministic — same tx validates the same way twice.
#[test]
fn test_fee_validation_determinism() {
    let ctx = test_context();
    let keypair = crypto::KeyPair::generate();
    let pubkey = *keypair.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    let prev_tx_hash = crypto::hash::hash(b"fee_det");
    let prev_output = Output::normal(500, pubkey_hash);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

    let recipient = crypto::hash::hash(b"det_recipient");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(prev_tx_hash, 0)],
        outputs: vec![Output::normal(100, recipient)],
        extra_data: vec![],
    };

    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let result1 = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    let result2 = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);

    assert!(result1.is_ok());
    assert!(result2.is_ok());
}

// ==========================================================================
// UniqueIdIndex — within-TX duplicate NFT token_id validation tests
// ==========================================================================

/// Context with covenants active (devnet, height 1000)
fn covenant_context() -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::for_network(Network::Devnet),
        Network::Devnet,
        GENESIS_TIME + 10_000,
        1000,
    )
    .with_prev_block(999, GENESIS_TIME + 9_990, Hash::ZERO)
}

// Test 14: duplicate token_id within a single TX is rejected
#[test]
fn test_validate_duplicate_token_id_within_tx() {
    let ctx = covenant_context();
    let minter = crypto::hash::hash(b"minter_dup");
    let nonce = 1u64.to_le_bytes().to_vec();
    let token_id = Output::compute_nft_token_id(&minter, &nonce);
    let cond = crate::conditions::Condition::signature(minter);

    let nft1 = Output::nft(1, minter, token_id, b"data1", &cond).unwrap();
    let nft2 = Output::nft(1, minter, token_id, b"data2", &cond).unwrap();

    let result = validate_outputs(&[nft1, nft2], &ctx);
    assert!(result.is_err(), "should reject duplicate token_id");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("duplicate NFT token_id"),
        "error should mention duplicate, got: {}",
        err_msg
    );
}

// Test 15: unique token_ids within a single TX are accepted
#[test]
fn test_validate_unique_token_ids_within_tx() {
    let ctx = covenant_context();
    let minter = crypto::hash::hash(b"minter_uniq");
    let cond = crate::conditions::Condition::signature(minter);

    let nonce1 = 1u64.to_le_bytes().to_vec();
    let nonce2 = 2u64.to_le_bytes().to_vec();
    let id1 = Output::compute_nft_token_id(&minter, &nonce1);
    let id2 = Output::compute_nft_token_id(&minter, &nonce2);

    let nft1 = Output::nft(1, minter, id1, b"a", &cond).unwrap();
    let nft2 = Output::nft(1, minter, id2, b"b", &cond).unwrap();

    let result = validate_outputs(&[nft1, nft2], &ctx);
    assert!(result.is_ok(), "unique token_ids should pass: {:?}", result);
}

// Test 16: batch of 100 unique NFTs is accepted
#[test]
fn test_validate_batch_100_unique_nfts() {
    let ctx = covenant_context();
    let minter = crypto::hash::hash(b"minter_batch");
    let cond = crate::conditions::Condition::signature(minter);

    let mut outputs = Vec::with_capacity(100);
    for i in 0u64..100 {
        let nonce = i.to_le_bytes().to_vec();
        let token_id = Output::compute_nft_token_id(&minter, &nonce);
        outputs.push(Output::nft(1, minter, token_id, b"x", &cond).unwrap());
    }

    let result = validate_outputs(&outputs, &ctx);
    assert!(result.is_ok(), "100 unique NFTs should pass: {:?}", result);
}

// Test 17: mixed outputs with NFTs and normal outputs pass
#[test]
fn test_validate_mixed_outputs_with_unique_nfts() {
    let ctx = covenant_context();
    let minter = crypto::hash::hash(b"minter_mixed");
    let cond = crate::conditions::Condition::signature(minter);

    let nonce = 1u64.to_le_bytes().to_vec();
    let token_id = Output::compute_nft_token_id(&minter, &nonce);
    let nft = Output::nft(1, minter, token_id, b"nft", &cond).unwrap();
    let normal = Output::normal(1000, minter);

    let result = validate_outputs(&[normal, nft], &ctx);
    assert!(result.is_ok(), "mixed outputs should pass: {:?}", result);
}

// ==========================================================================
// P0-001 Signature Verification Enforcement Tests
// ==========================================================================

/// Helper: create a signed transaction with a valid UTXO for P0-001 tests.
fn p0001_setup() -> (Transaction, MockUtxoProvider, crypto::KeyPair) {
    let kp = crypto::KeyPair::from_seed([42u8; 32]);
    let pubkey = *kp.public_key();
    let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());
    let prev_tx_hash = crypto::hash::hash(b"p0001_prev_tx");

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(
        prev_tx_hash,
        0,
        Output {
            output_type: OutputType::Normal,
            amount: 1_000_000,
            pubkey_hash,
            lock_until: 0,
            extra_data: Vec::new(),
        },
        pubkey,
    );

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash,
            output_index: 0,
            signature: crypto::Signature::default(), // Will be overwritten
            sighash_type: crate::transaction::SighashType::All,
            committed_output_count: 0,
            public_key: Some(pubkey),
        }],
        outputs: vec![Output {
            output_type: OutputType::Normal,
            amount: 999_000,
            pubkey_hash,
            lock_until: 0,
            extra_data: Vec::new(),
        }],
        extra_data: Vec::new(),
    };

    // Sign the transaction
    let signing_hash = tx.signing_message_for_input(0);
    let sig = crypto::signature::sign(signing_hash.as_bytes(), kp.private_key());
    let mut tx = tx;
    tx.inputs[0].signature = sig;

    (tx, utxo_provider, kp)
}

/// P0-001 regression: after activation height, inputs without public_key are rejected.
#[test]
fn test_p0001_missing_pubkey_rejected_after_activation() {
    let (mut tx, mut utxo_provider, _kp) = p0001_setup();

    // Remove public_key to simulate pre-fork transaction at post-fork height
    tx.inputs[0].public_key = None;

    // Also remove UTXO pubkey (production UTXO set doesn't store pubkeys)
    let prev_tx_hash = tx.inputs[0].prev_tx_hash;
    let output = utxo_provider
        .utxos
        .get(&(prev_tx_hash, 0))
        .unwrap()
        .output
        .clone();
    utxo_provider.utxos.insert(
        (prev_tx_hash, 0),
        UtxoInfo {
            output,
            pubkey: None,
            spent: false,
        },
    );

    // Height >= activation (100), sig_verification_height = 100
    let ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        100,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(100);

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        matches!(
            result,
            Err(ValidationError::MissingPublicKey {
                index: 0,
                activation_height: 100
            })
        ),
        "Must reject missing public_key after activation height, got: {:?}",
        result,
    );
}

/// P0-001: before activation height, missing public_key is accepted (backward compat).
#[test]
fn test_p0001_missing_pubkey_accepted_before_activation() {
    let (mut tx, mut utxo_provider, _kp) = p0001_setup();

    // Remove public_key — simulates pre-fork transactions
    tx.inputs[0].public_key = None;

    // Also remove UTXO pubkey so there's truly no pubkey to verify with
    let prev_tx_hash = tx.inputs[0].prev_tx_hash;
    let output = utxo_provider
        .utxos
        .get(&(prev_tx_hash, 0))
        .unwrap()
        .output
        .clone();
    utxo_provider.utxos.insert(
        (prev_tx_hash, 0),
        UtxoInfo {
            output,
            pubkey: None,
            spent: false,
        },
    );

    // Height 50 < activation 100 — should skip verification
    let ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        50,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(100);

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "Must accept missing public_key before activation height, got: {:?}",
        result,
    );
}

/// P0-001: valid signature with correct public_key passes after activation.
#[test]
fn test_p0001_valid_sig_passes_after_activation() {
    let (tx, utxo_provider, _kp) = p0001_setup();

    // Height 100 >= activation 100, but we have a valid pubkey + sig
    let ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        100,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(100);

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_ok(),
        "Must accept valid signature with public_key after activation, got: {:?}",
        result,
    );
}

/// P0-001: wrong public_key (hash mismatch) is rejected even after activation.
#[test]
fn test_p0001_wrong_pubkey_rejected() {
    let (mut tx, utxo_provider, _kp) = p0001_setup();

    // Set a different (wrong) public key
    let wrong_kp = crypto::KeyPair::from_seed([99u8; 32]);
    tx.inputs[0].public_key = Some(*wrong_kp.public_key());

    let ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        100,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(100);

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        matches!(result, Err(ValidationError::PubkeyHashMismatch { .. })),
        "Must reject wrong public_key (hash mismatch), got: {:?}",
        result,
    );
}

/// P0-001: correct public_key but invalid signature is rejected.
#[test]
fn test_p0001_invalid_signature_rejected() {
    let (mut tx, utxo_provider, _kp) = p0001_setup();

    // Corrupt the signature
    let mut sig_bytes = tx.inputs[0].signature.as_bytes().to_vec();
    sig_bytes[0] ^= 0xFF;
    tx.inputs[0].signature = crypto::Signature::from_bytes(sig_bytes.try_into().unwrap());

    let ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        100,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(100);

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        matches!(result, Err(ValidationError::InvalidSignature { index: 0 })),
        "Must reject invalid signature, got: {:?}",
        result,
    );
}

/// P0-001: at exactly the activation height boundary, enforcement is active.
#[test]
fn test_p0001_enforcement_at_exact_boundary() {
    let (mut tx, mut utxo_provider, _kp) = p0001_setup();
    tx.inputs[0].public_key = None;

    // Remove UTXO pubkey too
    let prev_tx_hash = tx.inputs[0].prev_tx_hash;
    let output = utxo_provider
        .utxos
        .get(&(prev_tx_hash, 0))
        .unwrap()
        .output
        .clone();
    utxo_provider.utxos.insert(
        (prev_tx_hash, 0),
        UtxoInfo {
            output,
            pubkey: None,
            spent: false,
        },
    );

    // Height == activation height: enforcement IS active (>=, not >)
    let ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        100,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(100);

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        matches!(result, Err(ValidationError::MissingPublicKey { .. })),
        "Must enforce at exact boundary height, got: {:?}",
        result,
    );

    // Height == activation - 1: NOT enforced
    let ctx_before = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        99,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(100);

    let result_before = utxo::validate_transaction_with_utxos(&tx, &ctx_before, &utxo_provider);
    assert!(
        result_before.is_ok(),
        "Must NOT enforce at height - 1, got: {:?}",
        result_before,
    );
}

// ==========================================================================
// NFT Fractionalization Tests
// ==========================================================================

/// Create a test context with covenants activated (height >= 2000 for Mainnet).
fn test_context_with_covenants() -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 30000,
        3000,
    )
    .with_prev_block(2999, GENESIS_TIME + 29990, Hash::ZERO)
}

/// Helper: create a simple NFT output with a Signature condition.
fn make_nft_output(amount: u64, pubkey_hash: Hash, token_id: Hash, content: &[u8]) -> Output {
    let condition = crate::conditions::Condition::Signature(pubkey_hash);
    Output::nft(amount, pubkey_hash, token_id, content, &condition).unwrap()
}

/// Helper: create a fractionalized NFT output (NFT + FRAC metadata appended).
fn make_fractionalized_nft_output(
    amount: u64,
    pubkey_hash: Hash,
    token_id: Hash,
    content: &[u8],
    total_shares: u64,
) -> Output {
    let base = make_nft_output(amount, pubkey_hash, token_id, content);
    let frac_asset_id = Output::fraction_asset_id(&token_id);
    let extra_data =
        Output::build_fractionalized_extra_data(&base.extra_data, &frac_asset_id, total_shares);
    Output { extra_data, ..base }
}

/// Helper: create a FungibleAsset output for fraction tokens.
fn make_fraction_token_output(
    amount: u64,
    pubkey_hash: Hash,
    fraction_asset_id: Hash,
    total_supply: u64,
) -> Output {
    let condition = crate::conditions::Condition::Signature(pubkey_hash);
    Output::fungible_asset(
        amount,
        pubkey_hash,
        fraction_asset_id,
        total_supply,
        "FRAC",
        &condition,
    )
    .unwrap()
}

#[test]
fn test_fractionalize_nft_happy_path() {
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let owner_hash = Hash::from_bytes([0x01; 32]);
    let content = b"test-content-hash";
    let total_shares = 1000u64;
    let frac_asset_id = Output::fraction_asset_id(&token_id);

    let nft_input_hash = Hash::from_bytes([0xAA; 32]);
    let fee_input_hash = Hash::from_bytes([0xBB; 32]);

    let tx = Transaction {
        version: 1,
        tx_type: TxType::FractionalizeNft,
        inputs: vec![Input::new(nft_input_hash, 0), Input::new(fee_input_hash, 0)],
        outputs: vec![
            make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares),
            make_fraction_token_output(total_shares, owner_hash, frac_asset_id, total_shares),
        ],
        extra_data: Vec::new(),
    };

    let result = transaction::validate_transaction(&tx, &ctx);
    assert!(
        result.is_ok(),
        "FractionalizeNft structural validation failed: {:?}",
        result
    );
}

#[test]
fn test_fractionalize_nft_already_fractionalized_rejected() {
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let owner_hash = Hash::from_bytes([0x01; 32]);
    let content = b"test-content";
    let total_shares = 1000u64;
    let frac_asset_id = Output::fraction_asset_id(&token_id);
    let nft_input_hash = Hash::from_bytes([0xAA; 32]);

    let mut utxo_provider = MockUtxoProvider::new();
    let fractionalized_nft =
        make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares);
    let keypair = crypto::KeyPair::generate();
    utxo_provider.add_utxo(nft_input_hash, 0, fractionalized_nft, *keypair.public_key());

    let fee_hash = Hash::from_bytes([0xBB; 32]);
    let fee_output = Output::normal(100_000, owner_hash);
    utxo_provider.add_utxo(fee_hash, 0, fee_output, *keypair.public_key());

    let tx = Transaction {
        version: 1,
        tx_type: TxType::FractionalizeNft,
        inputs: vec![Input::new(nft_input_hash, 0), Input::new(fee_hash, 0)],
        outputs: vec![
            make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares),
            make_fraction_token_output(total_shares, owner_hash, frac_asset_id, total_shares),
        ],
        extra_data: Vec::new(),
    };

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(result.is_err(), "should reject already-fractionalized NFT");
    let err = result.unwrap_err();
    // The spending prevention check fires first: fractionalized NFTs can only
    // be spent by RedeemNft, so FractionalizeNft is also rejected.
    assert!(
        matches!(err, ValidationError::InvalidTransaction(_)),
        "expected InvalidTransaction (spending prevention), got: {:?}",
        err
    );
}

#[test]
fn test_redeem_nft_happy_path() {
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let owner_hash = Hash::from_bytes([0x01; 32]);
    let content = b"test-content";

    let nft_input_hash = Hash::from_bytes([0xAA; 32]);
    let frac_input_hash = Hash::from_bytes([0xCC; 32]);

    let tx = Transaction {
        version: 1,
        tx_type: TxType::RedeemNft,
        inputs: vec![
            Input::new(nft_input_hash, 0),
            Input::new(frac_input_hash, 0),
        ],
        outputs: vec![make_nft_output(1, owner_hash, token_id, content)],
        extra_data: Vec::new(),
    };

    let result = transaction::validate_transaction(&tx, &ctx);
    assert!(
        result.is_ok(),
        "RedeemNft structural validation failed: {:?}",
        result
    );
}

#[test]
fn test_fractionalized_nft_spending_prevention() {
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let owner_hash = Hash::from_bytes([0x01; 32]);
    let content = b"test-content";
    let total_shares = 1000u64;
    let nft_input_hash = Hash::from_bytes([0xAA; 32]);

    let mut utxo_provider = MockUtxoProvider::new();
    let fractionalized_nft =
        make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares);
    let keypair = crypto::KeyPair::generate();
    utxo_provider.add_utxo(nft_input_hash, 0, fractionalized_nft, *keypair.public_key());

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(nft_input_hash, 0)],
        outputs: vec![make_nft_output(1, owner_hash, token_id, content)],
        extra_data: Vec::new(),
    };

    let result = utxo::validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
    assert!(
        result.is_err(),
        "should reject Transfer spending a fractionalized NFT"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, ValidationError::InvalidTransaction(_)),
        "expected InvalidTransaction for spending prevention, got: {:?}",
        err
    );
}

#[test]
fn test_fractionalize_nft_wrong_asset_id_rejected() {
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let owner_hash = Hash::from_bytes([0x01; 32]);
    let content = b"test-content";
    let total_shares = 500u64;
    let wrong_asset_id = Hash::from_bytes([0xFF; 32]);

    let tx = Transaction {
        version: 1,
        tx_type: TxType::FractionalizeNft,
        inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
        outputs: vec![
            make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares),
            make_fraction_token_output(total_shares, owner_hash, wrong_asset_id, total_shares),
        ],
        extra_data: Vec::new(),
    };

    let result = transaction::validate_transaction(&tx, &ctx);
    assert!(result.is_err(), "should reject wrong asset_id");
    let err = result.unwrap_err();
    assert!(
        matches!(err, ValidationError::InvalidFractionalization(_)),
        "expected InvalidFractionalization, got: {:?}",
        err
    );
}

#[test]
fn test_fractionalize_nft_zero_shares_rejected() {
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let owner_hash = Hash::from_bytes([0x01; 32]);
    let content = b"test-content";
    let frac_asset_id = Output::fraction_asset_id(&token_id);

    let base_nft = make_nft_output(1, owner_hash, token_id, content);
    let frac_nft_extra =
        Output::build_fractionalized_extra_data(&base_nft.extra_data, &frac_asset_id, 0);
    let frac_nft = Output {
        extra_data: frac_nft_extra,
        ..base_nft
    };

    let tx = Transaction {
        version: 1,
        tx_type: TxType::FractionalizeNft,
        inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
        outputs: vec![
            frac_nft,
            make_fraction_token_output(1, owner_hash, frac_asset_id, 1),
        ],
        extra_data: Vec::new(),
    };

    let result = transaction::validate_transaction(&tx, &ctx);
    assert!(result.is_err(), "should reject zero shares");
}

#[test]
fn test_is_fractionalized_detection() {
    let token_id = Hash::from_bytes([0x42; 32]);
    let owner_hash = Hash::from_bytes([0x01; 32]);
    let content = b"test-content";

    let normal_nft = make_nft_output(1, owner_hash, token_id, content);
    assert!(!normal_nft.is_fractionalized());
    assert!(normal_nft.fractionalization_metadata().is_none());

    let frac_nft = make_fractionalized_nft_output(1, owner_hash, token_id, content, 1000);
    assert!(frac_nft.is_fractionalized());
    let (asset_id, shares) = frac_nft.fractionalization_metadata().unwrap();
    assert_eq!(shares, 1000);
    assert_eq!(asset_id, Output::fraction_asset_id(&token_id));

    let stripped = frac_nft.strip_fractionalization_metadata().unwrap();
    assert_eq!(stripped, normal_nft.extra_data);
}

#[test]
fn test_fraction_asset_id_deterministic() {
    let token_id = Hash::from_bytes([0x42; 32]);
    let id1 = Output::fraction_asset_id(&token_id);
    let id2 = Output::fraction_asset_id(&token_id);
    assert_eq!(id1, id2, "fraction_asset_id must be deterministic");

    let other_token = Hash::from_bytes([0x43; 32]);
    let id3 = Output::fraction_asset_id(&other_token);
    assert_ne!(
        id1, id3,
        "different token_ids must produce different asset_ids"
    );
}

/// Helper: derive pubkey_hash (as Hash) from a keypair, matching the condition evaluation.
fn pubkey_hash_from_keypair(keypair: &crypto::KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, keypair.public_key().as_bytes())
}

/// Helper: sign a transaction for all inputs (both conditioned and normal).
/// Sets covenant witnesses for conditioned outputs and input signatures for normal outputs.
fn sign_tx_for_test(tx: &mut Transaction, keypair: &crypto::KeyPair) {
    // Build covenant witnesses for each input
    let input_count = tx.inputs.len();
    let mut witnesses: Vec<Vec<u8>> = Vec::with_capacity(input_count);
    for i in 0..input_count {
        let signing_hash = tx.signing_message_for_input(i);
        let mut w = crate::conditions::Witness::default();
        w.signatures.push(crate::conditions::WitnessSignature {
            pubkey: *keypair.public_key(),
            signature: crypto::signature::sign_hash(&signing_hash, keypair.private_key()),
        });
        witnesses.push(w.encode());
    }
    tx.set_covenant_witnesses(&witnesses);

    // Sign all inputs with per-input signing hash
    for i in 0..input_count {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].public_key = Some(*keypair.public_key());
    }
}

#[test]
fn test_fractionalize_nft_full_flow() {
    // Full flow: create NFT → fractionalize → verify outputs → reject Transfer → redeem → verify
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let keypair = crypto::KeyPair::generate();
    let owner_hash = pubkey_hash_from_keypair(&keypair);
    let content = b"full-flow-content";
    let total_shares = 500u64;
    let frac_asset_id = Output::fraction_asset_id(&token_id);

    // 1. Create a mock NFT output (UniqueAsset)
    let nft_output = make_nft_output(1, owner_hash, token_id, content);
    assert!(
        !nft_output.is_fractionalized(),
        "fresh NFT must not be fractionalized"
    );

    // 2. Build FractionalizeNft tx consuming it
    let nft_input_hash = Hash::from_bytes([0xAA; 32]);
    let fee_input_hash = Hash::from_bytes([0xBB; 32]);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(nft_input_hash, 0, nft_output, *keypair.public_key());
    utxo_provider.add_utxo(
        fee_input_hash,
        0,
        Output::normal(100_000, owner_hash),
        *keypair.public_key(),
    );

    let frac_nft_output =
        make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares);
    let frac_token_output =
        make_fraction_token_output(total_shares, owner_hash, frac_asset_id, total_shares);

    let mut frac_tx = Transaction {
        version: 1,
        tx_type: TxType::FractionalizeNft,
        inputs: vec![Input::new(nft_input_hash, 0), Input::new(fee_input_hash, 0)],
        outputs: vec![frac_nft_output.clone(), frac_token_output.clone()],
        extra_data: Vec::new(),
    };
    sign_tx_for_test(&mut frac_tx, &keypair);

    // 3. Validate passes (structural + UTXO)
    let structural_result = transaction::validate_transaction(&frac_tx, &ctx);
    assert!(
        structural_result.is_ok(),
        "FractionalizeNft structural validation failed: {:?}",
        structural_result
    );

    let utxo_result = utxo::validate_transaction_with_utxos(&frac_tx, &ctx, &utxo_provider);
    assert!(
        utxo_result.is_ok(),
        "FractionalizeNft UTXO validation failed: {:?}",
        utxo_result
    );

    // 4. Check output 0 has FRAC_MARKER
    assert!(
        frac_tx.outputs[0].is_fractionalized(),
        "output 0 must be fractionalized"
    );
    let (out_asset_id, out_shares) = frac_tx.outputs[0].fractionalization_metadata().unwrap();
    assert_eq!(out_asset_id, frac_asset_id);
    assert_eq!(out_shares, total_shares);

    // 5. Check output 1 is FungibleAsset with correct asset_id
    assert_eq!(frac_tx.outputs[1].output_type, OutputType::FungibleAsset);
    let (fa_asset_id, fa_supply, _ticker) = frac_tx.outputs[1].fungible_asset_metadata().unwrap();
    assert_eq!(fa_asset_id, frac_asset_id);
    assert_eq!(fa_supply, total_shares);

    // 6. Try spending fractionalized NFT with Transfer → REJECTED
    let frac_nft_hash = Hash::from_bytes([0xDD; 32]);
    let mut utxo_provider2 = MockUtxoProvider::new();
    utxo_provider2.add_utxo(
        frac_nft_hash,
        0,
        frac_nft_output.clone(),
        *keypair.public_key(),
    );

    let bad_transfer = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(frac_nft_hash, 0)],
        outputs: vec![make_nft_output(1, owner_hash, token_id, content)],
        extra_data: Vec::new(),
    };

    let transfer_result =
        utxo::validate_transaction_with_utxos(&bad_transfer, &ctx, &utxo_provider2);
    assert!(
        transfer_result.is_err(),
        "Transfer spending a fractionalized NFT must be rejected"
    );

    // 7. Build RedeemNft tx with NFT + fractions
    let frac_token_hash = Hash::from_bytes([0xEE; 32]);
    let fee_hash2 = Hash::from_bytes([0xFF; 32]);
    let mut utxo_provider3 = MockUtxoProvider::new();
    utxo_provider3.add_utxo(frac_nft_hash, 0, frac_nft_output, *keypair.public_key());
    utxo_provider3.add_utxo(frac_token_hash, 0, frac_token_output, *keypair.public_key());
    utxo_provider3.add_utxo(
        fee_hash2,
        0,
        Output::normal(100_000, owner_hash),
        *keypair.public_key(),
    );

    let redeemed_nft = make_nft_output(1, owner_hash, token_id, content);

    let mut redeem_tx = Transaction {
        version: 1,
        tx_type: TxType::RedeemNft,
        inputs: vec![
            Input::new(frac_nft_hash, 0),
            Input::new(frac_token_hash, 0),
            Input::new(fee_hash2, 0),
        ],
        outputs: vec![redeemed_nft],
        extra_data: Vec::new(),
    };
    sign_tx_for_test(&mut redeem_tx, &keypair);

    // 8. Validate passes
    let redeem_structural = transaction::validate_transaction(&redeem_tx, &ctx);
    assert!(
        redeem_structural.is_ok(),
        "RedeemNft structural validation failed: {:?}",
        redeem_structural
    );

    let redeem_utxo = utxo::validate_transaction_with_utxos(&redeem_tx, &ctx, &utxo_provider3);
    assert!(
        redeem_utxo.is_ok(),
        "RedeemNft UTXO validation failed: {:?}",
        redeem_utxo
    );

    // 9. Check output has original token_id WITHOUT frac metadata
    assert!(!redeem_tx.outputs[0].is_fractionalized());
    let (redeemed_token_id, _) = redeem_tx.outputs[0].nft_metadata().unwrap();
    assert_eq!(redeemed_token_id, token_id);
}

#[test]
fn test_redeem_nft_partial_fractions_rejected() {
    // Build RedeemNft with only half the fraction tokens → must fail
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let keypair = crypto::KeyPair::generate();
    let owner_hash = pubkey_hash_from_keypair(&keypair);
    let content = b"partial-test";
    let total_shares = 1000u64;
    let frac_asset_id = Output::fraction_asset_id(&token_id);

    let frac_nft_hash = Hash::from_bytes([0xAA; 32]);
    let partial_token_hash = Hash::from_bytes([0xBB; 32]);
    let fee_hash = Hash::from_bytes([0xCC; 32]);

    let frac_nft_output =
        make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares);
    // Only 500 of 1000 shares
    let partial_token_output =
        make_fraction_token_output(500, owner_hash, frac_asset_id, total_shares);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(frac_nft_hash, 0, frac_nft_output, *keypair.public_key());
    utxo_provider.add_utxo(
        partial_token_hash,
        0,
        partial_token_output,
        *keypair.public_key(),
    );
    utxo_provider.add_utxo(
        fee_hash,
        0,
        Output::normal(100_000, owner_hash),
        *keypair.public_key(),
    );

    let mut redeem_tx = Transaction {
        version: 1,
        tx_type: TxType::RedeemNft,
        inputs: vec![
            Input::new(frac_nft_hash, 0),
            Input::new(partial_token_hash, 0),
            Input::new(fee_hash, 0),
        ],
        outputs: vec![make_nft_output(1, owner_hash, token_id, content)],
        extra_data: Vec::new(),
    };
    sign_tx_for_test(&mut redeem_tx, &keypair);

    // Structural validation should pass (it doesn't check UTXO amounts)
    let structural = transaction::validate_transaction(&redeem_tx, &ctx);
    assert!(structural.is_ok(), "structural validation should pass");

    // UTXO validation must reject: only 500 of 1000 shares provided
    let utxo_result = utxo::validate_transaction_with_utxos(&redeem_tx, &ctx, &utxo_provider);
    assert!(
        utxo_result.is_err(),
        "should reject RedeemNft with partial fractions"
    );
    let err = utxo_result.unwrap_err();
    assert!(
        matches!(err, ValidationError::InvalidRedemption(_)),
        "expected InvalidRedemption, got: {:?}",
        err
    );
}

#[test]
fn test_redeem_nft_fraction_in_outputs_rejected() {
    // Build RedeemNft that keeps some fractions in outputs → must fail
    let ctx = test_context_with_covenants();
    let token_id = Hash::from_bytes([0x42; 32]);
    let keypair = crypto::KeyPair::generate();
    let owner_hash = pubkey_hash_from_keypair(&keypair);
    let content = b"output-leak-test";
    let total_shares = 1000u64;
    let frac_asset_id = Output::fraction_asset_id(&token_id);

    let frac_nft_hash = Hash::from_bytes([0xAA; 32]);
    let frac_token_hash = Hash::from_bytes([0xBB; 32]);
    let fee_hash = Hash::from_bytes([0xCC; 32]);

    let frac_nft_output =
        make_fractionalized_nft_output(1, owner_hash, token_id, content, total_shares);
    let full_token_output =
        make_fraction_token_output(total_shares, owner_hash, frac_asset_id, total_shares);

    let mut utxo_provider = MockUtxoProvider::new();
    utxo_provider.add_utxo(frac_nft_hash, 0, frac_nft_output, *keypair.public_key());
    utxo_provider.add_utxo(frac_token_hash, 0, full_token_output, *keypair.public_key());
    utxo_provider.add_utxo(
        fee_hash,
        0,
        Output::normal(100_000, owner_hash),
        *keypair.public_key(),
    );

    // Create a RedeemNft that sneaks fraction tokens into the outputs
    let leaked_fraction = make_fraction_token_output(500, owner_hash, frac_asset_id, total_shares);

    let mut redeem_tx = Transaction {
        version: 1,
        tx_type: TxType::RedeemNft,
        inputs: vec![
            Input::new(frac_nft_hash, 0),
            Input::new(frac_token_hash, 0),
            Input::new(fee_hash, 0),
        ],
        outputs: vec![
            make_nft_output(1, owner_hash, token_id, content),
            leaked_fraction,
        ],
        extra_data: Vec::new(),
    };
    sign_tx_for_test(&mut redeem_tx, &keypair);

    // UTXO validation must reject: fraction tokens in outputs
    let utxo_result = utxo::validate_transaction_with_utxos(&redeem_tx, &ctx, &utxo_provider);
    assert!(
        utxo_result.is_err(),
        "should reject RedeemNft with fraction tokens in outputs"
    );
    let err = utxo_result.unwrap_err();
    assert!(
        matches!(err, ValidationError::InvalidRedemption(_)),
        "expected InvalidRedemption, got: {:?}",
        err
    );
}
