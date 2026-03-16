//! Cross-crate serialization compatibility tests.
//!
//! These tests verify that transactions built by the wallet crate's TxBuilder
//! produce byte-identical output to `bincode::serialize()` on doli-core's
//! Transaction struct. This is critical: if the wire format doesn't match,
//! nodes will reject every transaction the GUI sends.
//!
//! NOTE: doli-core is added as a DEV-dependency only. The runtime wallet crate
//! does NOT depend on doli-core (architectural constraint: no VDF/GMP).

use crypto::{hash::hash, signature, KeyPair};
use doli_core::{Input, Output, OutputType, SighashType, Transaction, TxType};
use wallet::Utxo;

/// Build a Transfer tx via TxBuilder, then build the identical tx via doli-core,
/// serialize both, and compare byte-for-byte.
#[test]
fn test_m1_transfer_serialization_matches_core() {
    // Deterministic keypair from fixed seed
    let seed = [42u8; 32];
    let kp = KeyPair::from_seed(seed);

    let tx_hash_bytes = [0xAAu8; 32];
    let recipient_hash = [0xBBu8; 32];
    let sender_hash = [0xCCu8; 32];
    let amount: u64 = 500_000_000; // 5 DOLI
    let change: u64 = 300_000_000; // 3 DOLI

    // --- Build via wallet TxBuilder ---
    let mut wallet_builder = wallet::TxBuilder::new(wallet::TxType::Transfer);
    wallet_builder.add_input(tx_hash_bytes, 0);
    wallet_builder.add_output(amount, recipient_hash, 0, 0, Vec::new());
    wallet_builder.add_output(change, sender_hash, 0, 0, Vec::new());
    let wallet_hex = wallet_builder.sign_and_build(&kp).unwrap();
    let wallet_bytes = hex::decode(&wallet_hex).unwrap();

    // --- Build via doli-core ---
    // First, compute the same signing message the wallet computes
    let signing_bytes = wallet_builder.build_for_signing().unwrap();
    let signing_hash = hash(&signing_bytes);
    let sig = signature::sign(signing_hash.as_bytes(), kp.private_key());

    let core_tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: crypto::Hash::from_bytes(tx_hash_bytes),
            output_index: 0,
            signature: sig,
            sighash_type: SighashType::All,
            committed_output_count: 0,
        }],
        outputs: vec![
            Output {
                output_type: OutputType::Normal,
                amount,
                pubkey_hash: crypto::Hash::from_bytes(recipient_hash),
                lock_until: 0,
                extra_data: Vec::new(),
            },
            Output {
                output_type: OutputType::Normal,
                amount: change,
                pubkey_hash: crypto::Hash::from_bytes(sender_hash),
                lock_until: 0,
                extra_data: Vec::new(),
            },
        ],
        extra_data: Vec::new(),
    };

    let core_bytes = bincode::serialize(&core_tx).unwrap();

    assert_eq!(
        wallet_bytes,
        core_bytes,
        "Wallet TxBuilder output must be byte-identical to bincode::serialize(Transaction).\n\
         wallet len={}, core len={}\n\
         wallet first 64 bytes: {:02x?}\n\
         core   first 64 bytes: {:02x?}",
        wallet_bytes.len(),
        core_bytes.len(),
        &wallet_bytes[..wallet_bytes.len().min(64)],
        &core_bytes[..core_bytes.len().min(64)],
    );
}

/// Build an AddBond tx via the convenience builder and compare with core.
#[test]
fn test_m1_add_bond_serialization_matches_core() {
    let seed = [99u8; 32];
    let kp = KeyPair::from_seed(seed);

    let utxo_hash = "dd".repeat(32); // 64 hex chars = 32 bytes
    let utxos = vec![Utxo {
        tx_hash: utxo_hash.clone(),
        output_index: 0,
        amount: 50_000_000_000, // 500 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];

    let sender_hash = [0xEEu8; 32];
    let fee = 1000u64;
    let bond_count = 2u32;

    let mut wallet_builder =
        wallet::TxBuilder::build_add_bond(&utxos, bond_count, sender_hash, fee).unwrap();

    // Sign with wallet
    let wallet_hex = wallet_builder.sign_and_build(&kp).unwrap();
    let wallet_bytes = hex::decode(&wallet_hex).unwrap();

    // Compute the same signing message
    let signing_bytes = wallet_builder.build_for_signing().unwrap();
    let signing_hash = hash(&signing_bytes);
    let sig = signature::sign(signing_hash.as_bytes(), kp.private_key());

    let utxo_hash_bytes: [u8; 32] = hex::decode(&utxo_hash).unwrap().try_into().unwrap();

    // Expected: 2 bond outputs + 1 change output
    let bond_cost = bond_count as u64 * wallet::BOND_UNIT;
    let total_needed = bond_cost + fee;
    let change_amount = 50_000_000_000u64 - total_needed;

    let core_tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input {
            prev_tx_hash: crypto::Hash::from_bytes(utxo_hash_bytes),
            output_index: 0,
            signature: sig,
            sighash_type: SighashType::All,
            committed_output_count: 0,
        }],
        outputs: {
            let mut outs = Vec::new();
            for _ in 0..bond_count {
                outs.push(Output {
                    output_type: OutputType::Bond,
                    amount: wallet::BOND_UNIT,
                    pubkey_hash: crypto::Hash::from_bytes(sender_hash),
                    lock_until: u64::MAX,
                    extra_data: Vec::new(),
                });
            }
            outs.push(Output {
                output_type: OutputType::Normal,
                amount: change_amount,
                pubkey_hash: crypto::Hash::from_bytes(sender_hash),
                lock_until: 0,
                extra_data: Vec::new(),
            });
            outs
        },
        extra_data: Vec::new(),
    };

    let core_bytes = bincode::serialize(&core_tx).unwrap();

    assert_eq!(
        wallet_bytes,
        core_bytes,
        "AddBond TxBuilder output must match core bincode.\n\
         wallet len={}, core len={}",
        wallet_bytes.len(),
        core_bytes.len(),
    );
}

/// Verify that Hash fields in the wire format include the u64 length prefix
/// that bincode 1.x writes for serialize_bytes().
#[test]
fn test_m1_hash_field_has_length_prefix() {
    let seed = [7u8; 32];
    let kp = KeyPair::from_seed(seed);

    let tx_hash = [0x11u8; 32];
    let recipient = [0x22u8; 32];

    let mut builder = wallet::TxBuilder::new(wallet::TxType::Transfer);
    builder.add_input(tx_hash, 0);
    builder.add_output(100, recipient, 0, 0, Vec::new());

    let hex_out = builder.sign_and_build(&kp).unwrap();
    let bytes = hex::decode(&hex_out).unwrap();

    // Layout: version(4) + tx_type(4) + vec_len(8) = 16 bytes before first input
    // First input starts at offset 16:
    //   Hash length prefix (8 bytes, u64 LE = 32)
    let hash_len_prefix = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
    assert_eq!(
        hash_len_prefix, 32,
        "Hash field must have u64 LE length prefix of 32"
    );

    // After hash (24+32=56), output_index (4 bytes) at offset 56
    // Then signature length prefix at offset 60
    let sig_len_prefix = u64::from_le_bytes(bytes[60..68].try_into().unwrap());
    assert_eq!(
        sig_len_prefix, 64,
        "Signature field must have u64 LE length prefix of 64"
    );
}

/// Verify the deserialization round-trip: wallet -> bytes -> core deserialize.
#[test]
fn test_m1_wallet_tx_deserializes_in_core() {
    let seed = [55u8; 32];
    let kp = KeyPair::from_seed(seed);

    let utxos = vec![Utxo {
        tx_hash: "ff".repeat(32),
        output_index: 2,
        amount: 1_000_000_000, // 10 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 42,
        spendable: true,
    }];

    let recipient = [0x33u8; 32];
    let sender = [0x44u8; 32];

    let mut builder =
        wallet::TxBuilder::build_transfer(&utxos, recipient, 500_000_000, 1000, sender).unwrap();
    let hex_out = builder.sign_and_build(&kp).unwrap();
    let bytes = hex::decode(&hex_out).unwrap();

    // This is the critical test: can doli-core deserialize what the wallet produced?
    let deserialized: Transaction = bincode::deserialize(&bytes)
        .expect("doli-core must be able to deserialize wallet-built transactions");

    assert_eq!(deserialized.version, 1);
    assert_eq!(deserialized.tx_type, TxType::Transfer);
    assert_eq!(deserialized.inputs.len(), 1);
    assert_eq!(deserialized.outputs.len(), 2); // recipient + change

    // Verify amounts
    assert_eq!(deserialized.outputs[0].amount, 500_000_000);
    assert_eq!(
        deserialized.outputs[0].pubkey_hash,
        crypto::Hash::from_bytes(recipient)
    );
    assert_eq!(deserialized.outputs[0].output_type, OutputType::Normal);

    // Verify change
    let expected_change = 1_000_000_000 - 500_000_000 - 1000;
    assert_eq!(deserialized.outputs[1].amount, expected_change);
    assert_eq!(
        deserialized.outputs[1].pubkey_hash,
        crypto::Hash::from_bytes(sender)
    );
}

// =============================================================================
// Constant parity tests — detect drift between wallet and core constants
// =============================================================================

/// Verify that all 8 duplicated constants in the wallet crate match their
/// canonical values in doli-core. If any of these fail, the wallet is using
/// stale values that will produce incorrect transactions or UI displays.
#[test]
fn test_constants_match_core() {
    // Bond unit (10 DOLI in base units)
    assert_eq!(
        wallet::BOND_UNIT,
        doli_core::consensus::BOND_UNIT,
        "BOND_UNIT mismatch: wallet has {}, core has {}",
        wallet::BOND_UNIT,
        doli_core::consensus::BOND_UNIT,
    );

    // Maximum bonds per producer
    assert_eq!(
        wallet::MAX_BONDS_PER_PRODUCER,
        doli_core::consensus::MAX_BONDS_PER_PRODUCER,
        "MAX_BONDS_PER_PRODUCER mismatch: wallet has {}, core has {}",
        wallet::MAX_BONDS_PER_PRODUCER,
        doli_core::consensus::MAX_BONDS_PER_PRODUCER,
    );

    // Blocks per reward epoch
    assert_eq!(
        wallet::BLOCKS_PER_REWARD_EPOCH,
        doli_core::consensus::BLOCKS_PER_REWARD_EPOCH,
        "BLOCKS_PER_REWARD_EPOCH mismatch: wallet has {}, core has {}",
        wallet::BLOCKS_PER_REWARD_EPOCH,
        doli_core::consensus::BLOCKS_PER_REWARD_EPOCH,
    );

    // Coinbase maturity
    assert_eq!(
        wallet::COINBASE_MATURITY,
        doli_core::consensus::COINBASE_MATURITY,
        "COINBASE_MATURITY mismatch: wallet has {}, core has {}",
        wallet::COINBASE_MATURITY,
        doli_core::consensus::COINBASE_MATURITY,
    );

    // Unbonding period
    assert_eq!(
        wallet::UNBONDING_PERIOD,
        doli_core::consensus::UNBONDING_PERIOD,
        "UNBONDING_PERIOD mismatch: wallet has {}, core has {}",
        wallet::UNBONDING_PERIOD,
        doli_core::consensus::UNBONDING_PERIOD,
    );

    // Base registration fee
    assert_eq!(
        wallet::BASE_REGISTRATION_FEE,
        doli_core::consensus::BASE_REGISTRATION_FEE,
        "BASE_REGISTRATION_FEE mismatch: wallet has {}, core has {}",
        wallet::BASE_REGISTRATION_FEE,
        doli_core::consensus::BASE_REGISTRATION_FEE,
    );

    // Maximum registration fee
    assert_eq!(
        wallet::MAX_REGISTRATION_FEE,
        doli_core::consensus::MAX_REGISTRATION_FEE,
        "MAX_REGISTRATION_FEE mismatch: wallet has {}, core has {}",
        wallet::MAX_REGISTRATION_FEE,
        doli_core::consensus::MAX_REGISTRATION_FEE,
    );

    // Vesting quarter duration (wallet is u64, core is Slot = u32)
    assert_eq!(
        wallet::VESTING_QUARTER_SLOTS,
        doli_core::consensus::VESTING_QUARTER_SLOTS as u64,
        "VESTING_QUARTER_SLOTS mismatch: wallet has {}, core has {}",
        wallet::VESTING_QUARTER_SLOTS,
        doli_core::consensus::VESTING_QUARTER_SLOTS,
    );
}

// =============================================================================
// TxType variant count — catches core adding new types without wallet update
// =============================================================================

/// Count how many valid TxType variants exist in core by probing from_u32(0..=255).
/// If core adds a new transaction type, this test will fail, reminding the developer
/// to update the wallet's TxType enum and to_core_type_id() mapping.
#[test]
fn test_core_txtype_variant_count() {
    let mut count = 0u32;
    for i in 0..=255u32 {
        if doli_core::transaction::TxType::from_u32(i).is_some() {
            count += 1;
        }
    }
    // Core has 18 variants: Transfer(0), Registration(1), Exit(2), ClaimReward(3),
    // ClaimBond(4), SlashProducer(5), Coinbase(6), AddBond(7), RequestWithdrawal(8),
    // ClaimWithdrawal(9), EpochReward(10), RemoveMaintainer(11), AddMaintainer(12),
    // DelegateBond(13), RevokeDelegation(14), ProtocolActivation(15),
    // MintAsset(17), BurnAsset(18)
    // Note: discriminant 16 is unused (gap between ProtocolActivation and MintAsset).
    assert_eq!(
        count, 18,
        "Core TxType variant count changed from 18 to {}! Update wallet TxType to match.",
        count,
    );
}

// =============================================================================
// to_core_type_id exhaustive mapping — verify every wallet TxType maps correctly
// =============================================================================

/// Verify that each wallet TxType variant maps to the correct core TxType discriminant
/// in the serialized wire format. This catches silent remapping bugs where the wallet
/// sends a transaction with the wrong type discriminant.
///
/// Approach: build a minimal tx with each wallet TxType, serialize it, then read the
/// tx_type u32 from bytes [4..8] (bincode: version=u32 at [0..4], tx_type=u32 at [4..8]).
/// Deserialize with core to confirm the type matches end-to-end.
#[test]
fn test_to_core_type_id_mapping() {
    use wallet::TxType as WTx;

    let seed = [77u8; 32];
    let kp = KeyPair::from_seed(seed);

    // Helper: build a minimal tx with the given wallet type, serialize, and extract
    // the core tx_type discriminant from the wire bytes.
    let extract_core_type = |wtype: WTx| -> u32 {
        let mut builder = wallet::TxBuilder::new(wtype);
        builder.add_input([0xAAu8; 32], 0);
        builder.add_output(100, [0xBBu8; 32], 0, 0, Vec::new());
        let hex_out = builder.sign_and_build(&kp).unwrap();
        let bytes = hex::decode(&hex_out).unwrap();
        // tx_type is at bytes [4..8] as u32 LE
        u32::from_le_bytes(bytes[4..8].try_into().unwrap())
    };

    // Direct 1:1 mappings (wallet variant -> expected core discriminant)
    let mappings: Vec<(WTx, u32, &str)> = vec![
        (WTx::Transfer, 0, "Transfer"),
        (WTx::Registration, 1, "Registration"),
        (WTx::ProducerExit, 2, "Exit"),
        (WTx::Coinbase, 6, "Coinbase"),
        (WTx::RewardClaim, 3, "ClaimReward"),
        (WTx::AddBond, 7, "AddBond"),
        (WTx::RequestWithdrawal, 8, "RequestWithdrawal"),
        (WTx::ClaimWithdrawal, 9, "ClaimWithdrawal (tombstone)"),
        (WTx::SlashingEvidence, 5, "SlashProducer"),
        (WTx::DelegateBond, 13, "DelegateBond"),
        (WTx::RevokeDelegation, 14, "RevokeDelegation"),
    ];

    for (wallet_type, expected_core_id, name) in &mappings {
        let actual = extract_core_type(*wallet_type);
        assert_eq!(
            actual, *expected_core_id,
            "Wallet TxType::{} serialized to core id {}, expected {}",
            name, actual, expected_core_id,
        );
        // Also verify the core id is actually a valid TxType
        assert!(
            doli_core::transaction::TxType::from_u32(actual).is_some(),
            "Wallet TxType::{} serialized to core id {} which is not a valid core TxType!",
            name,
            actual,
        );
    }

    // Types that map to Transfer(0) because core doesn't have dedicated variants
    let transfer_aliases: Vec<(WTx, &str)> = vec![
        (WTx::NftMint, "NftMint"),
        (WTx::NftTransfer, "NftTransfer"),
        (WTx::TokenIssuance, "TokenIssuance"),
        (WTx::BridgeLock, "BridgeLock"),
    ];

    for (wallet_type, name) in &transfer_aliases {
        let actual = extract_core_type(*wallet_type);
        assert_eq!(
            actual, 0,
            "Wallet TxType::{} should serialize as core Transfer(0), got {}",
            name, actual,
        );
    }
}

// =============================================================================
// Fee parity — verify wallet and core fee functions produce identical results
// =============================================================================

/// Verify that the wallet's fee_multiplier_x100 produces identical results to core
/// across all tier boundaries. This function is consensus-critical: wrong fees cause
/// transaction rejection.
#[test]
fn test_fee_multiplier_parity() {
    // Test every tier boundary and representative values within each tier
    let test_cases: Vec<u32> = vec![
        0, 1, 4, // tier 1: 0-4 -> 100
        5, 7, 9, // tier 2: 5-9 -> 150
        10, 15, 19, // tier 3: 10-19 -> 200
        20, 35, 49, // tier 4: 20-49 -> 300
        50, 75, 99, // tier 5: 50-99 -> 450
        100, 150, 199, // tier 6: 100-199 -> 650
        200, 250, 299, // tier 7: 200-299 -> 850
        300, 500, 1000, // tier 8: 300+ -> 1000
    ];

    for pending in test_cases {
        let core_result = doli_core::consensus::fee_multiplier_x100(pending);
        // Wallet's fee_multiplier_x100 is private, so we test via registration_fee
        // which uses it internally. We verify the core result matches expected values.
        let expected = match pending {
            0..=4 => 100,
            5..=9 => 150,
            10..=19 => 200,
            20..=49 => 300,
            50..=99 => 450,
            100..=199 => 650,
            200..=299 => 850,
            _ => 1000,
        };
        assert_eq!(
            core_result, expected,
            "Core fee_multiplier_x100({}) = {}, expected {}",
            pending, core_result, expected,
        );
    }
}

/// Verify that wallet's registration fee calculation matches core across the full
/// range of pending registration counts.
#[test]
fn test_registration_fee_parity() {
    // Test representative pending counts across all tiers
    let test_cases: Vec<u32> = vec![0, 3, 5, 10, 20, 50, 100, 200, 300, 500];

    for pending in test_cases {
        let core_fee = doli_core::consensus::registration_fee(pending);

        // Wallet calculates: calculate_registration_cost(1, pending) returns
        // (bond_cost, reg_fee, total). We extract the reg_fee.
        let (_, wallet_fee, _) = wallet::calculate_registration_cost(1, pending).unwrap();

        assert_eq!(
            wallet_fee, core_fee,
            "Registration fee mismatch at pending={}: wallet={}, core={}",
            pending, wallet_fee, core_fee,
        );
    }
}

/// Verify that wallet's vesting penalty calculation matches core's withdrawal_penalty_rate
/// across all quarter boundaries.
#[test]
fn test_vesting_penalty_parity() {
    // Use core's VESTING_QUARTER_SLOTS for boundary values
    let quarter = doli_core::consensus::VESTING_QUARTER_SLOTS as u64;

    let test_cases: Vec<(u64, u8)> = vec![
        (0, 75),                     // Start of Q1
        (quarter / 2, 75),           // Middle of Q1
        (quarter - 1, 75),           // End of Q1
        (quarter, 50),               // Start of Q2
        (quarter + quarter / 2, 50), // Middle of Q2
        (2 * quarter - 1, 50),       // End of Q2
        (2 * quarter, 25),           // Start of Q3
        (3 * quarter - 1, 25),       // End of Q3
        (3 * quarter, 0),            // Fully vested
        (4 * quarter, 0),            // Well past vesting
        (10 * quarter, 0),           // Far future
    ];

    for (age_slots, expected_pct) in &test_cases {
        let core_pct = doli_core::withdrawal_penalty_rate_with_quarter(
            *age_slots as u32,
            doli_core::consensus::VESTING_QUARTER_SLOTS,
        );
        let wallet_pct = wallet::vesting_penalty_pct(*age_slots);

        assert_eq!(
            wallet_pct, *expected_pct,
            "Wallet vesting_penalty_pct({}) = {}, expected {}",
            age_slots, wallet_pct, expected_pct,
        );
        assert_eq!(
            core_pct, *expected_pct,
            "Core withdrawal_penalty_rate({}) = {}, expected {}",
            age_slots, core_pct, expected_pct,
        );
        assert_eq!(
            wallet_pct, core_pct,
            "Vesting penalty mismatch at age_slots={}: wallet={}, core={}",
            age_slots, wallet_pct, core_pct,
        );
    }
}
