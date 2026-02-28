//! Bond Stacking Integration Tests
//!
//! Tests the bond stacking system:
//! - Add bonds (1,000 DOLI = 1 bond)
//! - Maximum 100 bonds per producer
//! - Withdrawal with vesting penalties (75%/50%/25%/0%)
//! - FIFO withdrawal (oldest bonds first)
//! - 7-day withdrawal delay
//! - 100% burn of penalties

#[path = "../common/mod.rs"]
mod common;

use crypto::{hash::hash, Hash, KeyPair};
use doli_core::{
    withdrawal_penalty_rate,
    // Transaction types
    AddBondData,
    // Bond types
    BondEntry,
    BondError,
    ClaimWithdrawalData,
    PendingWithdrawal,
    ProducerBonds,
    Transaction,
    TxType,
    WithdrawalRequestData,
    // Bond constants
    BOND_UNIT,
    MAX_BONDS_PER_PRODUCER,
    WITHDRAWAL_DELAY_SLOTS,
    YEAR_IN_SLOTS,
};

/// Test bond unit constant
#[test]
fn test_bond_unit_constant() {
    // 1 bond = 10 DOLI = 1_000_000_000 base units
    assert_eq!(BOND_UNIT, 1_000_000_000);
}

/// Test max bonds constant
#[test]
fn test_max_bonds_constant() {
    // Maximum 10,000 bonds per producer
    assert_eq!(MAX_BONDS_PER_PRODUCER, 10_000);
}

/// Test withdrawal delay constant
#[test]
fn test_withdrawal_delay_constant() {
    // 7 days at 10s slots = 7 * 24 * 360 = 60,480 slots
    assert_eq!(WITHDRAWAL_DELAY_SLOTS, 60_480);
}

/// Test year in slots constant
#[test]
fn test_year_in_slots_constant() {
    // 1 year at 10s slots = 365 * 24 * 360 = 3,153,600 slots
    assert_eq!(YEAR_IN_SLOTS, 3_153_600);
}

/// Test vesting penalty schedule
#[test]
fn test_withdrawal_penalty_schedule() {
    // Year 1 (0-365 days): 75% penalty
    assert_eq!(withdrawal_penalty_rate(0), 75);
    assert_eq!(withdrawal_penalty_rate(YEAR_IN_SLOTS - 1), 75);

    // Year 2 (365-730 days): 50% penalty
    assert_eq!(withdrawal_penalty_rate(YEAR_IN_SLOTS), 50);
    assert_eq!(withdrawal_penalty_rate(2 * YEAR_IN_SLOTS - 1), 50);

    // Year 3 (730-1095 days): 25% penalty
    assert_eq!(withdrawal_penalty_rate(2 * YEAR_IN_SLOTS), 25);
    assert_eq!(withdrawal_penalty_rate(3 * YEAR_IN_SLOTS - 1), 25);

    // Year 4+ (1095+ days): 0% penalty (fully vested)
    assert_eq!(withdrawal_penalty_rate(3 * YEAR_IN_SLOTS), 0);
    assert_eq!(withdrawal_penalty_rate(4 * YEAR_IN_SLOTS), 0);
    assert_eq!(withdrawal_penalty_rate(10 * YEAR_IN_SLOTS), 0);
}

/// Test BondEntry creation and age calculation
#[test]
fn test_bond_entry() {
    let creation_slot = 1000;
    let bond = BondEntry::new(creation_slot);

    assert_eq!(bond.creation_slot, creation_slot);
    assert_eq!(bond.amount, BOND_UNIT);

    // Age at same slot should be 0
    assert_eq!(bond.age(1000), 0);

    // Age after 1 year
    let one_year_later = creation_slot + YEAR_IN_SLOTS;
    assert_eq!(bond.age(one_year_later), YEAR_IN_SLOTS);

    // Penalty rate at different ages
    assert_eq!(bond.penalty_rate(1000), 75); // Same slot = year 1
    assert_eq!(bond.penalty_rate(one_year_later), 50); // 1 year = year 2
}

/// Test BondEntry withdrawal calculations
#[test]
fn test_bond_withdrawal_amounts() {
    let creation_slot = 0;
    let bond = BondEntry::new(creation_slot);

    // Year 1: 75% penalty
    let (net, penalty) = bond.withdrawal_amount(100);
    assert_eq!(penalty, BOND_UNIT * 75 / 100);
    assert_eq!(net, BOND_UNIT * 25 / 100);
    assert_eq!(net + penalty, BOND_UNIT);

    // Year 2: 50% penalty
    let (net, penalty) = bond.withdrawal_amount(YEAR_IN_SLOTS + 100);
    assert_eq!(penalty, BOND_UNIT * 50 / 100);
    assert_eq!(net, BOND_UNIT * 50 / 100);

    // Year 3: 25% penalty
    let (net, penalty) = bond.withdrawal_amount(2 * YEAR_IN_SLOTS + 100);
    assert_eq!(penalty, BOND_UNIT * 25 / 100);
    assert_eq!(net, BOND_UNIT * 75 / 100);

    // Year 4+: 0% penalty (fully vested)
    let (net, penalty) = bond.withdrawal_amount(4 * YEAR_IN_SLOTS);
    assert_eq!(penalty, 0);
    assert_eq!(net, BOND_UNIT);
}

/// Test BondEntry vested check
#[test]
fn test_bond_is_vested() {
    let bond = BondEntry::new(0);

    // Not vested in years 1-3
    assert!(!bond.is_vested(0));
    assert!(!bond.is_vested(YEAR_IN_SLOTS));
    assert!(!bond.is_vested(2 * YEAR_IN_SLOTS));
    assert!(!bond.is_vested(3 * YEAR_IN_SLOTS));
    assert!(!bond.is_vested(4 * YEAR_IN_SLOTS - 1));

    // Vested at year 4+
    assert!(bond.is_vested(4 * YEAR_IN_SLOTS));
    assert!(bond.is_vested(5 * YEAR_IN_SLOTS));
}

/// Test ProducerBonds add bonds
#[test]
fn test_producer_bonds_add() {
    let mut bonds = ProducerBonds::new();

    assert_eq!(bonds.bond_count(), 0);
    assert_eq!(bonds.total_staked(), 0);

    // Add 5 bonds
    bonds.add_bonds(5, 100).unwrap();
    assert_eq!(bonds.bond_count(), 5);
    assert_eq!(bonds.total_staked(), 5 * BOND_UNIT);
    assert_eq!(bonds.selection_weight(), 5);

    // Add 10 more bonds
    bonds.add_bonds(10, 200).unwrap();
    assert_eq!(bonds.bond_count(), 15);
    assert_eq!(bonds.total_staked(), 15 * BOND_UNIT);
}

/// Test ProducerBonds max bonds limit
#[test]
fn test_producer_bonds_max_limit() {
    let mut bonds = ProducerBonds::new();

    // Add exactly 10,000 bonds (max)
    bonds.add_bonds(10_000, 0).unwrap();
    assert_eq!(bonds.bond_count(), 10_000);

    // Cannot add more
    let err = bonds.add_bonds(1, 10_000).unwrap_err();
    assert!(matches!(err, BondError::MaxBondsExceeded { .. }));
}

/// Test ProducerBonds FIFO withdrawal
#[test]
fn test_producer_bonds_fifo_withdrawal() {
    let mut bonds = ProducerBonds::new();
    let destination = Hash::ZERO;

    // Add bonds at different times:
    // 5 bonds at slot 0 (will be oldest = fully vested after 3 years)
    // 3 bonds at slot 2 * YEAR_IN_SLOTS (newer = higher penalty)
    bonds.add_bonds(5, 0).unwrap();
    bonds.add_bonds(3, 2 * YEAR_IN_SLOTS).unwrap();

    assert_eq!(bonds.bond_count(), 8);

    // Withdraw at slot 3 * YEAR_IN_SLOTS
    // Oldest bonds (slot 0) are now 3 years old = 0% penalty (fully vested!)
    // Newest bonds (slot 2Y) are 1 year old = 50% penalty
    let current_slot = 3 * YEAR_IN_SLOTS;

    // Request withdrawal of 3 bonds (should take 3 oldest)
    let withdrawal = bonds
        .request_withdrawal(3, current_slot, destination)
        .unwrap();

    // 3 oldest bonds have 0% penalty (3+ years = fully vested)
    let expected_penalty = 0;
    let expected_net = 3 * BOND_UNIT;

    assert_eq!(withdrawal.bond_count, 3);
    assert_eq!(withdrawal.penalty_amount, expected_penalty);
    assert_eq!(withdrawal.net_amount, expected_net);

    // 5 bonds remain (2 old + 3 new)
    assert_eq!(bonds.bond_count(), 5);
}

/// Test PendingWithdrawal claimability
#[test]
fn test_pending_withdrawal_claimability() {
    let withdrawal = PendingWithdrawal {
        bond_count: 5,
        request_slot: 1000,
        net_amount: 5 * BOND_UNIT / 2, // 50% after penalty
        penalty_amount: 5 * BOND_UNIT / 2,
        destination: Hash::ZERO,
    };

    // Not claimable immediately
    assert!(!withdrawal.is_claimable(1000));
    assert!(!withdrawal.is_claimable(1000 + WITHDRAWAL_DELAY_SLOTS - 1));

    // Claimable after 7 days
    assert!(withdrawal.is_claimable(1000 + WITHDRAWAL_DELAY_SLOTS));
    assert!(withdrawal.is_claimable(1000 + WITHDRAWAL_DELAY_SLOTS + 1000));
}

/// Test ProducerBonds claim withdrawal
#[test]
fn test_producer_bonds_claim_withdrawal() {
    let mut bonds = ProducerBonds::new();
    let destination = Hash::ZERO;

    bonds.add_bonds(10, 0).unwrap();

    // Request withdrawal
    let current_slot = YEAR_IN_SLOTS; // Year 2 = 50% penalty
    let withdrawal = bonds
        .request_withdrawal(5, current_slot, destination)
        .unwrap();

    assert!(bonds.has_pending_withdrawals());

    // Try to claim before delay - should fail
    let err = bonds.claim_withdrawal(current_slot + 1000).unwrap_err();
    assert!(matches!(err, BondError::NoClaimableWithdrawal));

    // Claim after delay
    let claimed = bonds
        .claim_withdrawal(current_slot + WITHDRAWAL_DELAY_SLOTS)
        .unwrap();
    assert_eq!(claimed.net_amount, withdrawal.net_amount);

    // No more pending withdrawals
    assert!(!bonds.has_pending_withdrawals());
}

/// Test ProducerBonds maturity summary
#[test]
fn test_bonds_maturity_summary() {
    let mut bonds = ProducerBonds::new();

    // Add bonds at different creation times
    bonds.add_bonds(2, 0).unwrap(); // age 4Y at check → year_4_plus
    bonds.add_bonds(3, YEAR_IN_SLOTS).unwrap(); // age 3Y at check → year_4_plus (3+ years = vested)
    bonds.add_bonds(4, 2 * YEAR_IN_SLOTS).unwrap(); // age 2Y at check → year_3 (25% penalty)
    bonds.add_bonds(1, 3 * YEAR_IN_SLOTS).unwrap(); // age 1Y at check → year_2 (50% penalty)

    // Check at 4 years from epoch 0
    let current_slot = 4 * YEAR_IN_SLOTS;
    let summary = bonds.maturity_summary(current_slot);

    // year_1 = bonds < 1Y old (none)
    // year_2 = bonds 1Y-2Y old (1 bond from slot 3Y)
    // year_3 = bonds 2Y-3Y old (4 bonds from slot 2Y)
    // year_4_plus = bonds 3Y+ old (2 from slot 0 + 3 from slot 1Y = 5)
    assert_eq!(summary.year_1, 0);
    assert_eq!(summary.year_2, 1);
    assert_eq!(summary.year_3, 4);
    assert_eq!(summary.year_4_plus, 5);
}

/// Test ProducerBonds total withdrawal penalty calculation
#[test]
fn test_total_withdrawal_penalty() {
    let mut bonds = ProducerBonds::new();

    // All bonds at slot 0
    bonds.add_bonds(10, 0).unwrap();

    // Year 1: 75% penalty each
    let penalty = bonds.total_withdrawal_penalty(100);
    assert_eq!(penalty, 10 * BOND_UNIT * 75 / 100);

    // Year 4+: 0% penalty
    let penalty = bonds.total_withdrawal_penalty(4 * YEAR_IN_SLOTS);
    assert_eq!(penalty, 0);
}

/// Test AddBondData serialization
#[test]
#[allow(deprecated)]
fn test_add_bond_data_serialization() {
    let keypair = KeyPair::generate();
    let data = AddBondData::new(*keypair.public_key(), 5);

    let bytes = data.to_bytes();
    let decoded = AddBondData::from_bytes(&bytes).unwrap();

    assert_eq!(decoded.producer_pubkey, *keypair.public_key());
    assert_eq!(decoded.bond_count, 5);

    assert_eq!(decoded.total_amount(), 5 * BOND_UNIT);
}

/// Test WithdrawalRequestData serialization
#[test]
fn test_withdrawal_request_data_serialization() {
    let keypair = KeyPair::generate();
    let destination = hash(b"destination");
    let data = WithdrawalRequestData::new(*keypair.public_key(), 3, destination);

    let bytes = data.to_bytes();
    let decoded = WithdrawalRequestData::from_bytes(&bytes).unwrap();

    assert_eq!(decoded.producer_pubkey, *keypair.public_key());
    assert_eq!(decoded.bond_count, 3);
    assert_eq!(decoded.destination, destination);
}

/// Test ClaimWithdrawalData serialization
#[test]
fn test_claim_withdrawal_data_serialization() {
    let keypair = KeyPair::generate();
    let data = ClaimWithdrawalData::new(*keypair.public_key(), 0);

    let bytes = data.to_bytes();
    let decoded = ClaimWithdrawalData::from_bytes(&bytes).unwrap();

    assert_eq!(decoded.producer_pubkey, *keypair.public_key());
    assert_eq!(decoded.withdrawal_index, 0);
}

/// Test AddBond transaction creation
#[test]
fn test_add_bond_transaction() {
    let keypair = KeyPair::generate();

    // Create mock input (normally from UTXO)
    let input = doli_core::Input::new(Hash::ZERO, 0);

    let tx = Transaction::new_add_bond(vec![input], *keypair.public_key(), 5);

    assert_eq!(tx.tx_type, TxType::AddBond);
    assert!(tx.is_add_bond());
    assert_eq!(tx.inputs.len(), 1);
    assert_eq!(tx.outputs.len(), 0); // No outputs - funds become bonds

    let data = tx.add_bond_data().unwrap();
    assert_eq!(data.bond_count, 5);
}

/// Test RequestWithdrawal transaction creation
#[test]
fn test_request_withdrawal_transaction() {
    let keypair = KeyPair::generate();
    let destination = hash(b"destination");

    let tx = Transaction::new_request_withdrawal(*keypair.public_key(), 3, destination);

    assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
    assert!(tx.is_request_withdrawal());
    assert_eq!(tx.inputs.len(), 0);
    assert_eq!(tx.outputs.len(), 0);

    let data = tx.withdrawal_request_data().unwrap();
    assert_eq!(data.bond_count, 3);
    assert_eq!(data.destination, destination);
}

/// Test ClaimWithdrawal transaction creation
#[test]
fn test_claim_withdrawal_transaction() {
    let keypair = KeyPair::generate();
    let destination = hash(b"destination");
    let net_amount = 5 * BOND_UNIT / 2; // After 50% penalty

    let tx = Transaction::new_claim_withdrawal(*keypair.public_key(), 0, net_amount, destination);

    assert_eq!(tx.tx_type, TxType::ClaimWithdrawal);
    assert!(tx.is_claim_withdrawal());
    assert_eq!(tx.inputs.len(), 0);
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, net_amount);
    assert_eq!(tx.outputs[0].pubkey_hash, destination);

    let data = tx.claim_withdrawal_data().unwrap();
    assert_eq!(data.withdrawal_index, 0);
}

/// Test BondError display
#[test]
fn test_bond_error_display() {
    let err = BondError::MaxBondsExceeded {
        current: 95,
        requested: 10,
        max: 100,
    };
    assert!(err.to_string().contains("max bonds exceeded"));

    let err = BondError::InsufficientBonds {
        requested: 10,
        available: 5,
    };
    assert!(err.to_string().contains("insufficient bonds"));

    let err = BondError::ZeroWithdrawal;
    assert!(err.to_string().contains("zero bonds"));

    let err = BondError::NoClaimableWithdrawal;
    assert!(err.to_string().contains("no withdrawal"));
}

/// Test realistic bond scenario
#[test]
fn test_realistic_bond_scenario() {
    let mut bonds = ProducerBonds::new();
    let destination = Hash::ZERO;

    // Producer starts with 10 bonds (10,000 DOLI)
    bonds.add_bonds(10, 0).unwrap();
    assert_eq!(bonds.selection_weight(), 10);

    // After 6 months, add 5 more bonds
    let six_months = YEAR_IN_SLOTS / 2;
    bonds.add_bonds(5, six_months).unwrap();
    assert_eq!(bonds.bond_count(), 15);
    assert_eq!(bonds.selection_weight(), 15);

    // After 2 years, producer wants to partially withdraw
    let two_years = 2 * YEAR_IN_SLOTS;

    // First 10 bonds are now 2 years old → year_3 (25% penalty)
    // Next 5 bonds are 1.5 years old → year_2 (50% penalty)
    let summary = bonds.maturity_summary(two_years);
    assert_eq!(summary.year_1, 0); // no bonds < 1Y old
    assert_eq!(summary.year_2, 5); // 5 bonds from 6 months (1.5Y old)
    assert_eq!(summary.year_3, 10); // 10 bonds from slot 0 (2Y old)

    // Withdraw 5 oldest bonds (25% penalty - they're 2Y old, in year_3)
    let withdrawal = bonds.request_withdrawal(5, two_years, destination).unwrap();

    // Net = 5 * 1000 DOLI * 75% = 3,750 DOLI
    // Penalty = 5 * 1000 DOLI * 25% = 1,250 DOLI (BURNED)
    assert_eq!(withdrawal.net_amount, 5 * BOND_UNIT * 75 / 100);
    assert_eq!(withdrawal.penalty_amount, 5 * BOND_UNIT * 25 / 100);

    // 10 bonds remain
    assert_eq!(bonds.bond_count(), 10);

    // Wait 7 days and claim
    let claim_time = two_years + WITHDRAWAL_DELAY_SLOTS;
    let claimed = bonds.claim_withdrawal(claim_time).unwrap();
    assert_eq!(claimed.net_amount, 5 * BOND_UNIT * 75 / 100);

    // After 4 years total, withdraw remaining with no penalty
    let four_years = 4 * YEAR_IN_SLOTS;
    let final_withdrawal = bonds
        .request_withdrawal(10, four_years, destination)
        .unwrap();

    // After the first withdrawal of 5 bonds, we have:
    // - 5 bonds from slot 0 (remaining original bonds)
    // - 5 bonds from slot 6_months
    //
    // At 4 years:
    // - 5 bonds from slot 0: age = 4Y, years = 4 → 0% penalty
    // - 5 bonds from slot 6_months: age = 3.5Y, years = 3 → 0% penalty (3+ = vested)
    //
    // FIFO takes oldest first, all are fully vested
    let expected_net = 10 * BOND_UNIT;
    let expected_penalty = 0;

    assert_eq!(final_withdrawal.net_amount, expected_net);
    assert_eq!(final_withdrawal.penalty_amount, expected_penalty);
}
