//! Bond Stacking Integration Tests
//!
//! Tests the bond stacking system:
//! - Add bonds (10 DOLI = 1 bond)
//! - Maximum 10,000 bonds per producer
//! - Withdrawal with vesting penalties (75%/50%/25%/0%)
//! - FIFO withdrawal (oldest bonds first)
//! - 7-day withdrawal delay
//! - 100% burn of penalties
//!
//! Vesting schedule: 1-day, quarter-based (6h quarters)

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
    VESTING_PERIOD_SLOTS,
    VESTING_QUARTER_SLOTS,
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

/// Test year in slots constant (used for seniority, not vesting)
#[test]
fn test_year_in_slots_constant() {
    // 1 year at 10s slots = 365 * 24 * 360 = 3,153,600 slots
    assert_eq!(YEAR_IN_SLOTS, 3_153_600);
}

/// Test vesting constants
#[test]
fn test_vesting_constants() {
    // 1 quarter = 6 hours = 2,160 slots at 10s/slot
    assert_eq!(VESTING_QUARTER_SLOTS, 2_160);
    // Full vesting = 4 quarters = 1 day = 8,640 slots
    assert_eq!(VESTING_PERIOD_SLOTS, 4 * VESTING_QUARTER_SLOTS);
    assert_eq!(VESTING_PERIOD_SLOTS, 8_640);
}

/// Test vesting penalty schedule (quarter-based)
#[test]
fn test_withdrawal_penalty_schedule() {
    // Q1 (0-6h): 75% penalty
    assert_eq!(withdrawal_penalty_rate(0), 75);
    assert_eq!(withdrawal_penalty_rate(VESTING_QUARTER_SLOTS - 1), 75);

    // Q2 (6-12h): 50% penalty
    assert_eq!(withdrawal_penalty_rate(VESTING_QUARTER_SLOTS), 50);
    assert_eq!(withdrawal_penalty_rate(2 * VESTING_QUARTER_SLOTS - 1), 50);

    // Q3 (12-18h): 25% penalty
    assert_eq!(withdrawal_penalty_rate(2 * VESTING_QUARTER_SLOTS), 25);
    assert_eq!(withdrawal_penalty_rate(3 * VESTING_QUARTER_SLOTS - 1), 25);

    // Q4+ (18h+): 0% penalty (fully vested)
    assert_eq!(withdrawal_penalty_rate(3 * VESTING_QUARTER_SLOTS), 0);
    assert_eq!(withdrawal_penalty_rate(4 * VESTING_QUARTER_SLOTS), 0);
    assert_eq!(withdrawal_penalty_rate(10 * VESTING_QUARTER_SLOTS), 0);
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

    // Age after 1 quarter (6h)
    let one_quarter_later = creation_slot + VESTING_QUARTER_SLOTS;
    assert_eq!(bond.age(one_quarter_later), VESTING_QUARTER_SLOTS);

    // Penalty rate at different ages
    assert_eq!(bond.penalty_rate(1000), 75); // Same slot = Q1
    assert_eq!(bond.penalty_rate(one_quarter_later), 50); // 1 quarter = Q2
}

/// Test BondEntry withdrawal calculations
#[test]
fn test_bond_withdrawal_amounts() {
    let creation_slot = 0;
    let bond = BondEntry::new(creation_slot);

    // Q1: 75% penalty
    let (net, penalty) = bond.withdrawal_amount(100);
    assert_eq!(penalty, BOND_UNIT * 75 / 100);
    assert_eq!(net, BOND_UNIT * 25 / 100);
    assert_eq!(net + penalty, BOND_UNIT);

    // Q2: 50% penalty
    let (net, penalty) = bond.withdrawal_amount(VESTING_QUARTER_SLOTS + 100);
    assert_eq!(penalty, BOND_UNIT * 50 / 100);
    assert_eq!(net, BOND_UNIT * 50 / 100);

    // Q3: 25% penalty
    let (net, penalty) = bond.withdrawal_amount(2 * VESTING_QUARTER_SLOTS + 100);
    assert_eq!(penalty, BOND_UNIT * 25 / 100);
    assert_eq!(net, BOND_UNIT * 75 / 100);

    // Q4+: 0% penalty (fully vested)
    let (net, penalty) = bond.withdrawal_amount(VESTING_PERIOD_SLOTS);
    assert_eq!(penalty, 0);
    assert_eq!(net, BOND_UNIT);
}

/// Test BondEntry vested check
#[test]
fn test_bond_is_vested() {
    let bond = BondEntry::new(0);

    // Not vested in Q1-Q3
    assert!(!bond.is_vested(0));
    assert!(!bond.is_vested(VESTING_QUARTER_SLOTS));
    assert!(!bond.is_vested(2 * VESTING_QUARTER_SLOTS));
    assert!(!bond.is_vested(VESTING_PERIOD_SLOTS - 1));

    // Vested at Q4+ (full period)
    assert!(bond.is_vested(VESTING_PERIOD_SLOTS));
    assert!(bond.is_vested(2 * VESTING_PERIOD_SLOTS));
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
    // 5 bonds at slot 0 (will be oldest = fully vested after 3 quarters)
    // 3 bonds at slot 2 * VESTING_QUARTER_SLOTS (newer = higher penalty)
    bonds.add_bonds(5, 0).unwrap();
    bonds.add_bonds(3, 2 * VESTING_QUARTER_SLOTS).unwrap();

    assert_eq!(bonds.bond_count(), 8);

    // Withdraw at slot 3 * VESTING_QUARTER_SLOTS (18h)
    // Oldest bonds (slot 0) are now 3 quarters old = 0% penalty (fully vested!)
    // Newest bonds (slot 2Q) are 1 quarter old = 50% penalty
    let current_slot = 3 * VESTING_QUARTER_SLOTS;

    // Request withdrawal of 3 bonds (should take 3 oldest)
    let withdrawal = bonds
        .request_withdrawal(3, current_slot, destination)
        .unwrap();

    // 3 oldest bonds have 0% penalty (3+ quarters = fully vested)
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

    // Request withdrawal at Q2 (6h) = 50% penalty
    let current_slot = VESTING_QUARTER_SLOTS;
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

/// Test ProducerBonds maturity summary (quarter-based)
#[test]
fn test_bonds_maturity_summary() {
    let mut bonds = ProducerBonds::new();

    // Add bonds at different creation times
    bonds.add_bonds(2, 0).unwrap(); // age 4Q at check → vested
    bonds.add_bonds(3, VESTING_QUARTER_SLOTS).unwrap(); // age 3Q at check → vested (3+ quarters)
    bonds.add_bonds(4, 2 * VESTING_QUARTER_SLOTS).unwrap(); // age 2Q at check → q3 (25% penalty)
    bonds.add_bonds(1, 3 * VESTING_QUARTER_SLOTS).unwrap(); // age 1Q at check → q2 (50% penalty)

    // Check at 4 quarters from epoch 0
    let current_slot = 4 * VESTING_QUARTER_SLOTS;
    let summary = bonds.maturity_summary(current_slot);

    // q1 = bonds < 1Q old (none)
    // q2 = bonds 1Q-2Q old (1 bond from slot 3Q)
    // q3 = bonds 2Q-3Q old (4 bonds from slot 2Q)
    // vested = bonds 3Q+ old (2 from slot 0 + 3 from slot 1Q = 5)
    assert_eq!(summary.q1, 0);
    assert_eq!(summary.q2, 1);
    assert_eq!(summary.q3, 4);
    assert_eq!(summary.vested, 5);
}

/// Test ProducerBonds total withdrawal penalty calculation
#[test]
fn test_total_withdrawal_penalty() {
    let mut bonds = ProducerBonds::new();

    // All bonds at slot 0
    bonds.add_bonds(10, 0).unwrap();

    // Q1: 75% penalty each
    let penalty = bonds.total_withdrawal_penalty(100);
    assert_eq!(penalty, 10 * BOND_UNIT * 75 / 100);

    // Q4+: 0% penalty
    let penalty = bonds.total_withdrawal_penalty(VESTING_PERIOD_SLOTS);
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

/// Test realistic bond scenario (1-day vesting)
#[test]
fn test_realistic_bond_scenario() {
    let mut bonds = ProducerBonds::new();
    let destination = Hash::ZERO;

    // Producer starts with 10 bonds (100 DOLI)
    bonds.add_bonds(10, 0).unwrap();
    assert_eq!(bonds.selection_weight(), 10);

    // After 1.5 quarters (9h), add 5 more bonds
    let one_and_half_q = VESTING_QUARTER_SLOTS + VESTING_QUARTER_SLOTS / 2;
    bonds.add_bonds(5, one_and_half_q).unwrap();
    assert_eq!(bonds.bond_count(), 15);
    assert_eq!(bonds.selection_weight(), 15);

    // After 2 quarters (12h), producer wants to partially withdraw
    let two_quarters = 2 * VESTING_QUARTER_SLOTS;

    // First 10 bonds are now 2 quarters old → q3 (25% penalty)
    // Next 5 bonds are 0.5 quarters old → q1 (75% penalty)
    let summary = bonds.maturity_summary(two_quarters);
    assert_eq!(summary.q1, 5); // 5 bonds from 1.5Q (only 0.5Q old)
    assert_eq!(summary.q2, 0); // no bonds 1Q-2Q old
    assert_eq!(summary.q3, 10); // 10 bonds from slot 0 (2Q old)

    // Withdraw 5 oldest bonds (25% penalty - they're 2Q old, in q3)
    let withdrawal = bonds
        .request_withdrawal(5, two_quarters, destination)
        .unwrap();

    // Net = 5 * 10 DOLI * 75% = 37.5 DOLI
    // Penalty = 5 * 10 DOLI * 25% = 12.5 DOLI (BURNED)
    assert_eq!(withdrawal.net_amount, 5 * BOND_UNIT * 75 / 100);
    assert_eq!(withdrawal.penalty_amount, 5 * BOND_UNIT * 25 / 100);

    // 10 bonds remain
    assert_eq!(bonds.bond_count(), 10);

    // Wait 7 days and claim
    let claim_time = two_quarters + WITHDRAWAL_DELAY_SLOTS;
    let claimed = bonds.claim_withdrawal(claim_time).unwrap();
    assert_eq!(claimed.net_amount, 5 * BOND_UNIT * 75 / 100);

    // After full vesting period, withdraw remaining with no penalty
    let fully_vested = VESTING_PERIOD_SLOTS + one_and_half_q; // Ensure even newest bonds are vested
    let final_withdrawal = bonds
        .request_withdrawal(10, fully_vested, destination)
        .unwrap();

    // All bonds fully vested → 0% penalty
    assert_eq!(final_withdrawal.net_amount, 10 * BOND_UNIT);
    assert_eq!(final_withdrawal.penalty_amount, 0);
}
