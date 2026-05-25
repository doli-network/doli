// OUTPUT CONTRACT: fn Output::lp_share(), fn Output::lp_share_with_condition(),
//   fn lp_share_metadata(), fn OutputType::is_conditioned()
//
//   Outputs:
//     O1: Output with condition-prefixed extra_data layout
//     O2: is_conditioned() returns true for LPShare
//     O3: lp_share_metadata() correctly skips condition prefix and returns pool_id
//     O4: Condition-bearing LPShare spending honors guards (reject/accept)
//
//   PATHS:
//     P1: Default constructor -> Condition::Signature(owner) prefix
//     P2: Custom condition constructor -> arbitrary condition prefix
//     P3: Spending with AmountGuard below threshold -> rejected
//     P4: Spending with AmountGuard above threshold -> accepted
//     P5: is_conditioned() type-level check
//
//   INPUT PARTITIONS:
//     IP1: lp_share(amount, pool_id, owner) -- default Signature condition
//     IP2: lp_share_with_condition(amount, pool_id, owner, And(Sig, AmountGuard))
//     IP3: Transfer spending conditioned LPShare, output amount < min_amount
//     IP4: Transfer spending conditioned LPShare, output amount >= min_amount
//
//   MATRIX (5 tests):
//     O1 x P1 x IP1 -> lpshare_default_layout_is_condition_prefixed
//     O4 x P3 x IP3 -> lpshare_with_amount_guard_blocks_spend_below_threshold
//     O4 x P4 x IP4 -> lpshare_with_amount_guard_allows_spend_above_threshold
//     O2 x P5 x IP1 -> is_conditioned_returns_true_for_lpshare
//     O3 x P2 x IP2 -> lpshare_metadata_round_trip_with_condition

use crypto::Hash;
use doli_core::conditions::Condition;
use doli_core::transaction::{Output, OutputType};

// ---------------------------------------------------------------------------
// Test 1: Default constructor produces condition-prefixed layout
// ---------------------------------------------------------------------------
#[test]
fn lpshare_default_layout_is_condition_prefixed() {
    let pool_id = Hash::from_bytes([0xAA; 32]);
    let owner = Hash::from_bytes([0xBB; 32]);

    let output = Output::lp_share(1000, pool_id, owner);

    // 1. Condition::decode_prefix must succeed and return Signature(owner)
    let (cond, cond_len) = Condition::decode_prefix(&output.extra_data)
        .expect("extra_data must start with a valid condition prefix");
    assert_eq!(
        cond,
        Condition::Signature(owner),
        "default condition must be Signature(owner)"
    );
    // Signature condition encodes as: 1B version + 1B tag + 32B hash = 34 bytes
    assert_eq!(cond_len, 34, "Signature condition prefix must be 34 bytes");

    // 2. lp_share_metadata() must return the pool_id
    let recovered = output
        .lp_share_metadata()
        .expect("lp_share_metadata must decode successfully");
    assert_eq!(
        recovered, pool_id,
        "pool_id must round-trip through metadata"
    );

    // 3. is_conditioned() must return true
    assert!(
        output.output_type.is_conditioned(),
        "LPShare must be conditioned"
    );
}

// ---------------------------------------------------------------------------
// Helper: build a spending tx for a conditioned LPShare UTXO
//
// The tx spends two inputs:
//   [0] = the conditioned LPShare UTXO (non-native amount)
//   [1] = a Normal DOLI UTXO that covers the output value (native amount)
//
// This mirrors real-world usage: LP shares are non-native, so a DOLI input
// is needed whenever the spending tx produces Normal outputs.
//
// The witness signature uses signing_message_for_input() (BIP-143 style),
// which is what the validator verifies against.
// ---------------------------------------------------------------------------
fn build_lpshare_spend_tx(
    kp: &crypto::KeyPair,
    owner_pkh: Hash,
    prev_hash_lp: Hash,
    prev_hash_doli: Hash,
    output_amount: u64,
    _doli_amount: u64,
) -> doli_core::transaction::Transaction {
    use doli_core::transaction::{Input, SighashType, Transaction, TxType};

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![
            // Input 0: LPShare (conditioned)
            Input {
                prev_tx_hash: prev_hash_lp,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            // Input 1: Normal DOLI (covers output value)
            Input {
                prev_tx_hash: prev_hash_doli,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![Output::normal(output_amount, owner_pkh)],
        extra_data: vec![],
    };

    // Sign input 0 (LPShare) -- per-input signing hash
    let signing_hash_0 = tx.signing_message_for_input(0);
    let sig_0 = crypto::signature::sign_hash(&signing_hash_0, kp.private_key());
    tx.inputs[0].signature = sig_0;

    // Sign input 1 (Normal DOLI) -- per-input signing hash
    let signing_hash_1 = tx.signing_message_for_input(1);
    let sig_1 = crypto::signature::sign_hash(&signing_hash_1, kp.private_key());
    tx.inputs[1].signature = sig_1;

    // Set covenant witnesses: input 0 gets the condition witness, input 1
    // gets an empty witness (Normal outputs use the input.signature field).
    let witness_0 = doli_core::conditions::Witness {
        signatures: vec![doli_core::conditions::WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig_0,
        }],
        preimage: None,
        or_branches: vec![],
    };
    tx.set_covenant_witnesses(&[witness_0.encode(), vec![]]);

    tx
}

// ---------------------------------------------------------------------------
// Test 2: AmountGuard blocks spend below threshold
// ---------------------------------------------------------------------------
#[test]
fn lpshare_with_amount_guard_blocks_spend_below_threshold() {
    use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
    use doli_core::network::Network;
    use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext};
    use std::collections::HashMap;

    let kp = crypto::KeyPair::from_seed([0x42; 32]);
    let owner_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());
    let pool_id = Hash::from_bytes([0xAA; 32]);

    // Build LPShare with And(Signature(owner), AmountGuard{min=1000, idx=0})
    let condition = Condition::And(
        Box::new(Condition::Signature(owner_pkh)),
        Box::new(Condition::AmountGuard {
            min_amount: 1000,
            output_index: 0,
        }),
    );
    let lp_output = Output::lp_share_with_condition(1000, pool_id, owner_pkh, &condition)
        .expect("condition encoding must succeed");

    let prev_hash_lp = Hash::from_bytes([0x55; 32]);
    let prev_hash_doli = Hash::from_bytes([0x66; 32]);

    struct MockUtxos {
        utxos: HashMap<(Hash, u32), UtxoInfo>,
    }
    impl UtxoProvider for MockUtxos {
        fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
            self.utxos.get(&(*tx_hash, index)).cloned()
        }
    }

    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (prev_hash_lp, 0),
        UtxoInfo {
            output: lp_output,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    // Normal DOLI input to cover the output
    utxos.utxos.insert(
        (prev_hash_doli, 0),
        UtxoInfo {
            output: Output::normal(500, owner_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    // Output amount = 500 < 1000 threshold -> must reject (AmountGuard)
    let tx = build_lpshare_spend_tx(&kp, owner_pkh, prev_hash_lp, prev_hash_doli, 500, 500);

    let ctx = ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 120,
        1,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0);

    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_err(),
        "spending conditioned LPShare with output amount below AmountGuard \
         must be rejected"
    );
}

// ---------------------------------------------------------------------------
// Test 3: AmountGuard allows spend above threshold
// ---------------------------------------------------------------------------
#[test]
fn lpshare_with_amount_guard_allows_spend_above_threshold() {
    use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
    use doli_core::network::Network;
    use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext};
    use std::collections::HashMap;

    let kp = crypto::KeyPair::from_seed([0x42; 32]);
    let owner_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());
    let pool_id = Hash::from_bytes([0xAA; 32]);

    // Build LPShare with And(Signature(owner), AmountGuard{min=1000, idx=0})
    let condition = Condition::And(
        Box::new(Condition::Signature(owner_pkh)),
        Box::new(Condition::AmountGuard {
            min_amount: 1000,
            output_index: 0,
        }),
    );
    let lp_output = Output::lp_share_with_condition(2000, pool_id, owner_pkh, &condition)
        .expect("condition encoding must succeed");

    let prev_hash_lp = Hash::from_bytes([0x55; 32]);
    let prev_hash_doli = Hash::from_bytes([0x66; 32]);

    struct MockUtxos {
        utxos: HashMap<(Hash, u32), UtxoInfo>,
    }
    impl UtxoProvider for MockUtxos {
        fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
            self.utxos.get(&(*tx_hash, index)).cloned()
        }
    }

    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (prev_hash_lp, 0),
        UtxoInfo {
            output: lp_output,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    // Normal DOLI input with enough to cover output + fee (2001 DOLI)
    utxos.utxos.insert(
        (prev_hash_doli, 0),
        UtxoInfo {
            output: Output::normal(2001, owner_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    // Output amount = 2000 >= 1000 threshold -> should accept
    // DOLI input = 2001, output = 2000, fee = 1 (minimum)
    let tx = build_lpshare_spend_tx(&kp, owner_pkh, prev_hash_lp, prev_hash_doli, 2000, 2001);

    let ctx = ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 120,
        1,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0);

    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "spending conditioned LPShare with output amount above AmountGuard \
         must be accepted, got: {:?}",
        res
    );
}

// ---------------------------------------------------------------------------
// Test 4: is_conditioned() returns true for LPShare
// ---------------------------------------------------------------------------
#[test]
fn is_conditioned_returns_true_for_lpshare() {
    assert!(
        OutputType::LPShare.is_conditioned(),
        "OutputType::LPShare must return true from is_conditioned()"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Round-trip with custom condition preserves pool_id
// ---------------------------------------------------------------------------
#[test]
fn lpshare_metadata_round_trip_with_condition() {
    let pool_id = Hash::from_bytes([0xCC; 32]);
    let owner = Hash::from_bytes([0xDD; 32]);

    // Nested condition: And(Signature, AmountGuard)
    let condition = Condition::And(
        Box::new(Condition::Signature(owner)),
        Box::new(Condition::AmountGuard {
            min_amount: 5000,
            output_index: 0,
        }),
    );

    let output = Output::lp_share_with_condition(999, pool_id, owner, &condition)
        .expect("encoding must succeed");

    // Verify condition decodes correctly
    let (decoded_cond, _cond_len) =
        Condition::decode_prefix(&output.extra_data).expect("condition prefix must decode");
    assert_eq!(decoded_cond, condition, "condition must round-trip");

    // Verify metadata decodes correctly
    let recovered = output
        .lp_share_metadata()
        .expect("metadata must decode after condition prefix");
    assert_eq!(
        recovered, pool_id,
        "pool_id must survive round-trip with nested condition"
    );

    // Verify type
    assert_eq!(output.output_type, OutputType::LPShare);
    assert!(output.output_type.is_conditioned());
}
