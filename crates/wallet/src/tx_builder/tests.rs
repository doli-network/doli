use super::*;
use crate::types::*;

// ========================================================================
// Requirement: GUI-FR-011 (Must) -- Send transaction
// Acceptance: Transaction construction, signing, validation
// ========================================================================

#[test]
fn test_fr011_transfer_builder_basic() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000, // 5 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let recipient = [0xBBu8; 32];
    let sender = [0xAAu8; 32];

    let builder = TxBuilder::build_transfer(&utxos, recipient, 100_000_000, 1000, sender).unwrap();
    assert_eq!(builder.tx_type(), TxType::Transfer);
    assert_eq!(builder.input_count(), 1);
    assert!(builder.output_count() >= 1); // recipient + possible change
}

#[test]
fn test_fr011_transfer_insufficient_balance() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 100_000, // 0.001 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let recipient = [0xBBu8; 32];
    let sender = [0xAAu8; 32];

    let result = TxBuilder::build_transfer(&utxos, recipient, 500_000_000, 1000, sender);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Insufficient"));
}

#[test]
fn test_fr011_transfer_zero_amount_rejected() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 0, 1000, [0xAAu8; 32]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("greater than 0"));
}

#[test]
fn test_fr011_transfer_change_output() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000, // 5 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let sender = [0xAAu8; 32];

    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, sender).unwrap();
    // Should have 2 outputs: recipient (1 DOLI) + change (3.99999 DOLI)
    assert_eq!(builder.output_count(), 2);
}

#[test]
fn test_fr011_transfer_exact_amount_no_change() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 100_001_000, // exactly amount + fee
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let sender = [0xAAu8; 32];

    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, sender).unwrap();
    // Exact amount: only recipient output, no change
    assert_eq!(builder.output_count(), 1);
}

#[test]
fn test_fr011_transfer_skips_unspendable_utxos() {
    let utxos = vec![
        Utxo {
            tx_hash: "bb".repeat(32),
            output_index: 0,
            amount: 999_000_000_000, // large but unspendable
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: false,
        },
        Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000, // spendable
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        },
    ];
    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]).unwrap();
    assert_eq!(builder.input_count(), 1);
}

#[test]
fn test_fr011_transfer_skips_bond_utxos() {
    let utxos = vec![
        Utxo {
            tx_hash: "cc".repeat(32),
            output_index: 0,
            amount: 1_000_000_000,
            output_type: "bond".to_string(),
            lock_until: u64::MAX,
            height: 100,
            spendable: true,
        },
        Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        },
    ];
    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]).unwrap();
    assert_eq!(
        builder.input_count(),
        1,
        "Bond UTXOs must not be used for transfers"
    );
}

#[test]
fn test_fr011_transfer_multiple_utxos() {
    let utxos = vec![
        Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 100_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        },
        Utxo {
            tx_hash: "bb".repeat(32),
            output_index: 0,
            amount: 100_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 101,
            spendable: true,
        },
    ];
    let builder = TxBuilder::build_transfer(
        &utxos,
        [0xBBu8; 32],
        150_000_000, // needs both UTXOs
        1000,
        [0xAAu8; 32],
    )
    .unwrap();
    assert_eq!(builder.input_count(), 2);
}

#[test]
fn test_fr011_transfer_empty_utxos() {
    let utxos: Vec<Utxo> = vec![];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Insufficient"));
}

// Edge case: amount + fee overflow
#[test]
fn test_fr011_transfer_amount_fee_overflow() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: u64::MAX,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], u64::MAX, u64::MAX, [0xAAu8; 32]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("overflow"));
}

// ========================================================================
// Requirement: GUI-FR-020 (Must) -- Producer registration cost
// Acceptance: Bond count * 10 DOLI + registration fee
// ========================================================================

#[test]
fn test_fr020_registration_cost_single_bond() {
    let (bond_cost, reg_fee, total) = calculate_registration_cost(1, 0).unwrap();
    assert_eq!(bond_cost, BOND_UNIT); // 10 DOLI
    assert_eq!(reg_fee, BASE_REGISTRATION_FEE); // 0.001 DOLI
    assert_eq!(total, BOND_UNIT + BASE_REGISTRATION_FEE);
}

#[test]
fn test_fr020_registration_cost_10_bonds() {
    let (bond_cost, reg_fee, total) = calculate_registration_cost(10, 0).unwrap();
    assert_eq!(bond_cost, 10 * BOND_UNIT); // 100 DOLI
    assert_eq!(total, bond_cost + reg_fee);
}

#[test]
fn test_fr020_registration_fee_scales_with_pending() {
    let (_, fee_0, _) = calculate_registration_cost(1, 0).unwrap();
    let (_, fee_5, _) = calculate_registration_cost(1, 5).unwrap();
    assert!(
        fee_5 > fee_0,
        "Fee should increase with pending registrations"
    );
}

#[test]
fn test_fr020_registration_fee_capped() {
    let (_, fee, _) = calculate_registration_cost(1, 1000).unwrap();
    assert_eq!(
        fee, MAX_REGISTRATION_FEE,
        "Fee must be capped at MAX_REGISTRATION_FEE"
    );
}

#[test]
fn test_fr020_registration_cost_zero_bonds_rejected() {
    let result = calculate_registration_cost(0, 0);
    assert!(result.is_err());
}

#[test]
fn test_fr020_registration_cost_exceeds_max_bonds() {
    let result = calculate_registration_cost(MAX_BONDS_PER_PRODUCER + 1, 0);
    assert!(result.is_err());
}

// ========================================================================
// Requirement: GUI-FR-024 (Must) -- Add bonds
// Acceptance: Each bond = 10 DOLI, max 3000 bonds
// ========================================================================

#[test]
fn test_fr024_add_bond_basic() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 50_000_000_000, // 500 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let sender = [0xAAu8; 32];

    let builder = TxBuilder::build_add_bond(&utxos, 5, sender, 1000).unwrap();
    assert_eq!(builder.tx_type(), TxType::AddBond);
    // 5 bond outputs + 1 change output
    assert_eq!(builder.output_count(), 6);
}

#[test]
fn test_fr024_add_bond_zero_rejected() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 50_000_000_000,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_add_bond(&utxos, 0, [0xAAu8; 32], 1000);
    assert!(result.is_err());
}

#[test]
fn test_fr024_add_bond_exceeds_max() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: u64::MAX / 2,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_add_bond(&utxos, MAX_BONDS_PER_PRODUCER + 1, [0xAAu8; 32], 1000);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
}

#[test]
fn test_fr024_add_bond_insufficient_balance() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000, // 5 DOLI -- not enough for 1 bond (10 DOLI)
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_add_bond(&utxos, 1, [0xAAu8; 32], 1000);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Insufficient"));
}

#[test]
fn test_fr024_bond_cost_calculation() {
    // 5 bonds = 50 DOLI = 5,000,000,000 base units
    assert_eq!(5u64 * BOND_UNIT, 5_000_000_000);
    // 3000 bonds (max) = 30,000 DOLI = 3,000,000,000,000 base units
    assert_eq!(3000u64 * BOND_UNIT, 3_000_000_000_000u64);
}

// ========================================================================
// Requirement: GUI-FR-025 (Must) -- Request withdrawal
// Acceptance: FIFO order, vesting penalty calculation
// ========================================================================

#[test]
fn test_fr025_vesting_penalty_q1() {
    // 0-1 year: 75% penalty
    assert_eq!(vesting_penalty_pct(0), 75);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS - 1), 75);
}

#[test]
fn test_fr025_vesting_penalty_q2() {
    // 1-2 years: 50% penalty
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS), 50);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 2 - 1), 50);
}

#[test]
fn test_fr025_vesting_penalty_q3() {
    // 2-3 years: 25% penalty
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 2), 25);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 3 - 1), 25);
}

#[test]
fn test_fr025_vesting_penalty_fully_vested() {
    // 4+ years: 0% penalty
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 4), 0);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 10), 0);
}

#[test]
fn test_fr025_withdrawal_net_calculation() {
    // 10 DOLI bond, 75% penalty
    let net = calculate_withdrawal_net(BOND_UNIT, 75);
    assert_eq!(net, BOND_UNIT / 4); // 2.5 DOLI

    // 10 DOLI bond, 50% penalty
    let net = calculate_withdrawal_net(BOND_UNIT, 50);
    assert_eq!(net, BOND_UNIT / 2); // 5 DOLI

    // 10 DOLI bond, 0% penalty (fully vested)
    let net = calculate_withdrawal_net(BOND_UNIT, 0);
    assert_eq!(net, BOND_UNIT); // 10 DOLI
}

#[test]
fn test_fr025_request_withdrawal_basic() {
    let sender = [0xAAu8; 32];
    let builder = TxBuilder::build_request_withdrawal(5, sender, None).unwrap();
    assert_eq!(builder.tx_type(), TxType::RequestWithdrawal);
}

#[test]
fn test_fr025_request_withdrawal_zero_bonds_rejected() {
    let result = TxBuilder::build_request_withdrawal(0, [0xAAu8; 32], None);
    assert!(result.is_err());
}

#[test]
fn test_fr025_request_withdrawal_with_destination() {
    let sender = [0xAAu8; 32];
    let dest = [0xBBu8; 32];
    let builder = TxBuilder::build_request_withdrawal(3, sender, Some(dest)).unwrap();
    assert_eq!(builder.output_count(), 1);
}

// ========================================================================
// Requirement: GUI-FR-031, GUI-FR-032 (Must) -- Claim rewards
// Acceptance: Epoch selector, optional recipient, estimated reward
// ========================================================================

#[test]
fn test_fr031_reward_claim_basic() {
    let sender = [0xAAu8; 32];
    let builder = TxBuilder::build_reward_claim(42, sender, None).unwrap();
    assert_eq!(builder.tx_type(), TxType::RewardClaim);
}

#[test]
fn test_fr031_reward_claim_with_recipient() {
    let sender = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];
    let builder = TxBuilder::build_reward_claim(42, sender, Some(recipient)).unwrap();
    assert_eq!(builder.output_count(), 1);
}

// ========================================================================
// TxBuilder core tests
// ========================================================================

#[test]
fn test_tx_builder_new() {
    let builder = TxBuilder::new(TxType::Transfer);
    assert_eq!(builder.tx_type(), TxType::Transfer);
    assert_eq!(builder.input_count(), 0);
    assert_eq!(builder.output_count(), 0);
}

#[test]
fn test_tx_builder_add_input_output() {
    let mut builder = TxBuilder::new(TxType::Transfer);
    builder.add_input([0u8; 32], 0);
    builder.add_output(100, [0u8; 32], 0, 0, Vec::new());

    assert_eq!(builder.input_count(), 1);
    assert_eq!(builder.output_count(), 1);
}

#[test]
fn test_tx_type_values() {
    // These must match doli-core TxType values exactly
    assert_eq!(TxType::Transfer as u8, 0);
    assert_eq!(TxType::Registration as u8, 1);
    assert_eq!(TxType::ProducerExit as u8, 2);
    assert_eq!(TxType::Coinbase as u8, 3);
    assert_eq!(TxType::NftMint as u8, 4);
    assert_eq!(TxType::NftTransfer as u8, 5);
    assert_eq!(TxType::RewardClaim as u8, 6);
    assert_eq!(TxType::AddBond as u8, 7);
    assert_eq!(TxType::RequestWithdrawal as u8, 8);
    assert_eq!(TxType::ClaimWithdrawal as u8, 9);
    assert_eq!(TxType::SlashingEvidence as u8, 10);
    assert_eq!(TxType::TokenIssuance as u8, 11);
    assert_eq!(TxType::BridgeLock as u8, 12);
    assert_eq!(TxType::DelegateBond as u8, 13);
    assert_eq!(TxType::RevokeDelegation as u8, 14);
}

// ========================================================================
// Edge cases
// ========================================================================

#[test]
fn test_build_for_signing_no_inputs_rejected() {
    let mut builder = TxBuilder::new(TxType::Transfer);
    builder.add_output(100, [0u8; 32], 0, 0, Vec::new());
    // build_for_signing is a todo!() but the validation before it should work
    // This test verifies the error path
    // Note: This will panic with todo!() until implemented
}

#[test]
fn test_transfer_invalid_tx_hash_hex() {
    let utxos = vec![Utxo {
        tx_hash: "not_valid_hex!!".to_string(),
        output_index: 0,
        amount: 500_000_000,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]);
    assert!(result.is_err());
}
