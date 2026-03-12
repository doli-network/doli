//! Bond Stacking Integration Tests
//!
//! Tests the bond stacking system:
//! - Add bonds (10 DOLI = 1 bond)
//! - Maximum 3,000 bonds per producer
//! - Withdrawal with vesting penalties (75%/50%/25%/0%)
//! - FIFO withdrawal (oldest bonds first)
//! - Instant withdrawal (no delay)
//! - 100% burn of penalties
//!
//! Vesting schedule: 4-year, quarter-based (1-year quarters on mainnet)

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
    ProducerBonds,
    Transaction,
    TxType,
    WithdrawalRequestData,
    // Bond constants
    BOND_UNIT,
    MAX_BONDS_PER_PRODUCER,
    VESTING_PERIOD_SLOTS,
    VESTING_QUARTER_SLOTS,
    YEAR_IN_SLOTS,
};
use storage::{ProducerInfo, StoredBondEntry};

/// Test bond unit constant
#[test]
fn test_bond_unit_constant() {
    // 1 bond = 10 DOLI = 1_000_000_000 base units
    assert_eq!(BOND_UNIT, 1_000_000_000);
}

/// Test max bonds constant
#[test]
fn test_max_bonds_constant() {
    // Maximum 3,000 bonds per producer
    assert_eq!(MAX_BONDS_PER_PRODUCER, 3_000);
}

/// Test year in slots constant (used for seniority, not vesting)
#[test]
fn test_year_in_slots_constant() {
    // 1 year at 10s slots = 365 * 24 * 360 = 3,153,600 slots
    assert_eq!(YEAR_IN_SLOTS, 3_153_600);
}

/// Test vesting constants (mainnet: 4-year, 1-year quarters)
#[test]
fn test_vesting_constants() {
    // 1 quarter = 1 year = 3,153,600 slots at 10s/slot (mainnet)
    assert_eq!(VESTING_QUARTER_SLOTS, YEAR_IN_SLOTS);
    assert_eq!(VESTING_QUARTER_SLOTS, 3_153_600);
    // Full vesting = 4 quarters = 4 years = 12,614,400 slots
    assert_eq!(VESTING_PERIOD_SLOTS, 4 * VESTING_QUARTER_SLOTS);
    assert_eq!(VESTING_PERIOD_SLOTS, 12_614_400);
}

/// Test vesting penalty schedule (quarter-based, mainnet: 1-year quarters)
#[test]
fn test_withdrawal_penalty_schedule() {
    // Q1 (0-1yr): 75% penalty
    assert_eq!(withdrawal_penalty_rate(0), 75);
    assert_eq!(withdrawal_penalty_rate(VESTING_QUARTER_SLOTS - 1), 75);

    // Q2 (1-2yr): 50% penalty
    assert_eq!(withdrawal_penalty_rate(VESTING_QUARTER_SLOTS), 50);
    assert_eq!(withdrawal_penalty_rate(2 * VESTING_QUARTER_SLOTS - 1), 50);

    // Q3 (2-3yr): 25% penalty
    assert_eq!(withdrawal_penalty_rate(2 * VESTING_QUARTER_SLOTS), 25);
    assert_eq!(withdrawal_penalty_rate(3 * VESTING_QUARTER_SLOTS - 1), 25);

    // Q4+ (3yr+): 0% penalty (fully vested)
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

    // Add exactly 3,000 bonds (max)
    bonds.add_bonds(3_000, 0).unwrap();
    assert_eq!(bonds.bond_count(), 3_000);

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

/// Test AddBond transaction creation
#[test]
fn test_add_bond_transaction() {
    let keypair = KeyPair::generate();

    // Create mock input (normally from UTXO)
    let input = doli_core::Input::new(Hash::ZERO, 0);

    let tx = Transaction::new_add_bond(
        vec![input],
        *keypair.public_key(),
        5,
        5 * BOND_UNIT,
        10_000_000,
    );

    assert_eq!(tx.tx_type, TxType::AddBond);
    assert!(tx.is_add_bond());
    assert_eq!(tx.inputs.len(), 1);
    assert_eq!(tx.outputs.len(), 5); // One Bond UTXO per bond (per-bond FIFO)

    let data = tx.add_bond_data().unwrap();
    assert_eq!(data.bond_count, 5);
}

/// Test RequestWithdrawal transaction creation
#[test]
fn test_request_withdrawal_transaction() {
    let keypair = KeyPair::generate();
    let destination = hash(b"destination");
    let net_amount = 3 * BOND_UNIT; // 3 bonds, fully vested

    let bond_inputs = vec![doli_core::Input::new(hash(b"bond_utxo"), 0)];
    let tx = Transaction::new_request_withdrawal(
        bond_inputs,
        *keypair.public_key(),
        3,
        destination,
        net_amount,
    );

    assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
    assert!(tx.is_request_withdrawal());
    assert_eq!(tx.inputs.len(), 1); // Bond UTXO input (lock/unlock model)
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, net_amount);
    assert_eq!(tx.outputs[0].pubkey_hash, destination);

    let data = tx.withdrawal_request_data().unwrap();
    assert_eq!(data.bond_count, 3);
    assert_eq!(data.destination, destination);
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

    // 10 bonds remain (withdrawal is instant, no delay)
    assert_eq!(bonds.bond_count(), 10);

    // After full vesting period, withdraw remaining with no penalty
    let fully_vested = VESTING_PERIOD_SLOTS + one_and_half_q; // Ensure even newest bonds are vested
    let final_withdrawal = bonds
        .request_withdrawal(10, fully_vested, destination)
        .unwrap();

    // All bonds fully vested → 0% penalty
    assert_eq!(final_withdrawal.net_amount, 10 * BOND_UNIT);
    assert_eq!(final_withdrawal.penalty_amount, 0);
}

// ==================== StoredBondEntry & Per-Bond Tracking Tests ====================

/// Test StoredBondEntry struct creation
#[test]
fn test_stored_bond_entry_creation() {
    let entry = StoredBondEntry {
        creation_slot: 1000,
        amount: BOND_UNIT,
    };
    assert_eq!(entry.creation_slot, 1000);
    assert_eq!(entry.amount, BOND_UNIT);
}

/// Test ProducerInfo initializes bond_entries on creation
#[test]
fn test_producer_info_initializes_bond_entries() {
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        500, // registered_at
        3 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    assert_eq!(info.bond_count, 3);
    assert_eq!(info.bond_entries.len(), 3);
    for entry in &info.bond_entries {
        assert_eq!(entry.creation_slot, 500);
        assert_eq!(entry.amount, BOND_UNIT);
    }
    assert_eq!(info.withdrawal_pending_count, 0);
}

/// Test bond_entries migration from empty to populated
#[test]
fn test_bond_entries_migration() {
    let keypair = KeyPair::generate();
    #[allow(deprecated)]
    let mut info = ProducerInfo {
        public_key: *keypair.public_key(),
        registered_at: 200,
        bond_amount: 5 * BOND_UNIT,
        bond_outpoint: (Hash::ZERO, 0),
        status: storage::ProducerStatus::Active,
        blocks_produced: 0,
        slots_missed: 0,
        registration_era: 0,
        pending_rewards: 0,
        has_prior_exit: false,
        last_activity: 0,
        activity_gaps: 0,
        bond_count: 5,
        additional_bonds: Vec::new(),
        delegated_to: None,
        delegated_bonds: 0,
        received_delegations: Vec::new(),
        bond_entries: Vec::new(), // Empty — needs migration
        withdrawal_pending_count: 0,
        bls_pubkey: Vec::new(),
    };

    // Should be empty before migration
    assert!(info.bond_entries.is_empty());

    // Migrate
    info.migrate_bond_entries(BOND_UNIT);

    // Now populated from bond_count + registered_at
    assert_eq!(info.bond_entries.len(), 5);
    for entry in &info.bond_entries {
        assert_eq!(entry.creation_slot, 200);
        assert_eq!(entry.amount, BOND_UNIT);
    }
}

/// Test add_bonds creates entries with correct creation_slot
#[test]
fn test_add_bonds_with_creation_slot() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        2 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    assert_eq!(info.bond_entries.len(), 2);
    assert_eq!(info.bond_entries[0].creation_slot, 0);

    // Add 3 bonds at slot 5000
    let outpoints = vec![(Hash::ZERO, 1), (Hash::ZERO, 2), (Hash::ZERO, 3)];
    let added = info.add_bonds(outpoints, BOND_UNIT, 5000);
    assert_eq!(added, 3);
    assert_eq!(info.bond_entries.len(), 5);
    assert_eq!(info.bond_entries[2].creation_slot, 5000);
    assert_eq!(info.bond_entries[3].creation_slot, 5000);
    assert_eq!(info.bond_entries[4].creation_slot, 5000);
}

/// Test calculate_withdrawal FIFO order (oldest first)
#[test]
fn test_calculate_withdrawal_fifo_order() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        2 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // Add 3 more bonds at a later slot
    let later_slot = VESTING_QUARTER_SLOTS + 1000;
    let outpoints = vec![(Hash::ZERO, 1), (Hash::ZERO, 2), (Hash::ZERO, 3)];
    info.add_bonds(outpoints, BOND_UNIT, later_slot);

    // Calculate withdrawal of 2 bonds well past full vesting (4 years)
    let fully_vested_slot = 4 * VESTING_QUARTER_SLOTS + 1;
    let (net, penalty) = info.calculate_withdrawal(2, fully_vested_slot).unwrap();
    assert_eq!(net, 2 * BOND_UNIT);
    assert_eq!(penalty, 0);
}

/// Test withdrawal with mixed-age penalties
#[test]
fn test_withdrawal_mixed_age_penalties() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        5 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // Add 2 bonds at 2*Q slots (will be in Q3 at check time)
    let outpoints = vec![(Hash::ZERO, 1), (Hash::ZERO, 2)];
    info.add_bonds(outpoints, BOND_UNIT, 2 * VESTING_QUARTER_SLOTS);

    // Check at slot = 3*Q
    let check_slot = 3 * VESTING_QUARTER_SLOTS;

    // Withdraw 6 bonds: 5 at slot 0 (age 3Q = vested) + 1 at slot 2Q (age 1Q = 50% penalty)
    let (net, penalty) = info.calculate_withdrawal(6, check_slot).unwrap();
    // 5 vested bonds: 5 * BOND_UNIT * 100% = 5 * BOND_UNIT
    // 1 Q2 bond:      1 * BOND_UNIT * 50% = 0.5 * BOND_UNIT net, 0.5 * BOND_UNIT penalty
    assert_eq!(net, 5 * BOND_UNIT + BOND_UNIT / 2);
    assert_eq!(penalty, BOND_UNIT / 2);
}

/// Test apply_withdrawal reduces bond_count
#[test]
fn test_apply_withdrawal_reduces_bond_count() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        10 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    assert_eq!(info.bond_count, 10);
    assert_eq!(info.bond_entries.len(), 10);

    info.apply_withdrawal(3, BOND_UNIT);

    assert_eq!(info.bond_count, 7);
    assert_eq!(info.bond_entries.len(), 7);
    assert_eq!(info.bond_amount, 7 * BOND_UNIT);
}

/// Test withdrawal insufficient bonds
#[test]
fn test_withdrawal_insufficient_bonds() {
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        3 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // Requesting more than available
    assert!(info.calculate_withdrawal(5, 1000).is_none());
}

/// Test withdrawal_pending_count prevents double-withdrawal
#[test]
fn test_withdrawal_pending_prevents_double() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        5 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // First withdrawal: 3 bonds
    info.withdrawal_pending_count = 3;

    // Available = 5 - 3 = 2. Requesting 3 should fail.
    assert!(info.calculate_withdrawal(3, 1000).is_none());

    // Requesting 2 should succeed (2 available)
    assert!(info.calculate_withdrawal(2, 1000).is_some());
}

/// Test withdrawal_pending_count resets after apply_withdrawal
#[test]
fn test_withdrawal_pending_resets_at_epoch() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        5 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    info.withdrawal_pending_count = 2;
    info.apply_withdrawal(2, BOND_UNIT);

    // After apply_withdrawal, pending count is decremented
    assert_eq!(info.withdrawal_pending_count, 0);
    assert_eq!(info.bond_count, 3);
}

/// Test all bonds vested → zero penalty
#[test]
fn test_withdrawal_all_vested_zero_penalty() {
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        5 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // All bonds at slot 0, check at > VESTING_PERIOD
    let (net, penalty) = info
        .calculate_withdrawal(5, VESTING_PERIOD_SLOTS + 1)
        .unwrap();
    assert_eq!(net, 5 * BOND_UNIT);
    assert_eq!(penalty, 0);
}

/// Test all bonds in Q1 → max penalty (75%)
#[test]
fn test_withdrawal_all_q1_max_penalty() {
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        1000,
        3 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // Bonds at slot 1000, check at slot 1100 (age 100 = Q1)
    let (net, penalty) = info.calculate_withdrawal(3, 1100).unwrap();
    assert_eq!(penalty, 3 * BOND_UNIT * 75 / 100);
    assert_eq!(net, 3 * BOND_UNIT * 25 / 100);
}

/// Test full withdrawal lifecycle: add at different times, withdraw subset, verify remaining
#[test]
fn test_full_withdrawal_lifecycle() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        3 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // Add 2 bonds at 2 quarters in
    let mid_slot = 2 * VESTING_QUARTER_SLOTS;
    let outpoints = vec![(Hash::ZERO, 1), (Hash::ZERO, 2)];
    info.add_bonds(outpoints, BOND_UNIT, mid_slot);

    // Add 2 bonds at 3 quarters in
    let late_slot = 3 * VESTING_QUARTER_SLOTS;
    let outpoints2 = vec![(Hash::ZERO, 3), (Hash::ZERO, 4)];
    info.add_bonds(outpoints2, BOND_UNIT, late_slot);

    assert_eq!(info.bond_count, 7);
    assert_eq!(info.bond_entries.len(), 7);

    // Withdraw 4 bonds at slot just past full vesting for slot-0 bonds
    let check_slot = VESTING_PERIOD_SLOTS + 1;
    let (net, penalty) = info.calculate_withdrawal(4, check_slot).unwrap();
    // Slot 0 bonds: age = VESTING_PERIOD+1 → 4 quarters → vested (0%) — 3 bonds
    // mid_slot bonds: age = VESTING_PERIOD+1 - 2*Q = 2*Q+1 → 2 quarters → Q3 (25%) — 1 bond
    assert_eq!(net, 3 * BOND_UNIT + BOND_UNIT * 75 / 100);
    assert_eq!(penalty, BOND_UNIT * 25 / 100);

    // Apply the withdrawal
    info.withdrawal_pending_count = 4;
    info.apply_withdrawal(4, BOND_UNIT);

    // Remaining: 3 bonds (1 from mid_slot, 2 from late_slot)
    assert_eq!(info.bond_count, 3);
    assert_eq!(info.bond_entries.len(), 3);
    assert_eq!(info.bond_entries[0].creation_slot, mid_slot);
    assert_eq!(info.bond_entries[1].creation_slot, late_slot);
    assert_eq!(info.bond_entries[2].creation_slot, late_slot);
}

/// Test new_with_bonds initializes bond_entries
#[test]
fn test_new_with_bonds_initializes_entries() {
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new_with_bonds(
        *keypair.public_key(),
        300,
        4 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        4,
    );

    assert_eq!(info.bond_count, 4);
    assert_eq!(info.bond_entries.len(), 4);
    for entry in &info.bond_entries {
        assert_eq!(entry.creation_slot, 300);
    }
}
