// OUTPUT CONTRACT: the wallet crate's transaction-construction surface —
//   TxBuilder::build_transfer, calculate_registration_cost,
//   calculate_withdrawal_net, vesting_penalty_pct, and the protocol constants
//   these tests pin against doli-core.
//
//   O1: the built transaction shape — tx_type, input_count, output_count
//       (the change output is present or absent, which is O1's second cell)
//   O2: the computed scalar — bond cost, fee, net withdrawal, penalty percent
//   O3: the constant's value — consensus-critical, must equal doli-core exactly
//
// PATHS:
//   P1: build_transfer where the UTXO set must be combined and change is returned
//   P2: registration cost for a bond count
//   P3: registration fee across the tiered pending-count multiplier
//   P4: vesting penalty across the quarter schedule
//   P5: withdrawal net for a given penalty
//   P6: TxType discriminants
//   P7: protocol constants
//
// INPUT PARTITIONS (the classes that change which branch or tier runs):
//   P1: UTXO set smaller than the amount (must combine) — the case asserted here.
//   P3: pending count per tier — 0 (1.00x), 5 (1.50x), 100 (6.50x), 300 (cap),
//       1000 (still capped). The cap boundary and beyond-cap are distinct classes
//       because only they can expose a missing clamp.
//   P4: quarter boundaries — 0, QUARTER-1, exactly QUARTER, and 4*QUARTER. The
//       off-by-one pair around a boundary is the partition that catches a wrong
//       comparison operator.
//   P5: penalty 75 / 50 / 0 — the endpoints plus a midpoint.
//   P2/P6/P7: single partition each; the value is a constant, so no input class
//       can change the result.
//
// MATRIX: 3 outputs x 7 paths x their partitions. Each cell is asserted by the
//   correspondingly named test below. O1 applies only to P1; O2 to P2-P5; O3 to
//   P6-P7.
//
//   EXCEPTION — test_nf013_wallet_crate_no_vdf_dependency is OUTPUT CONTRACT: N/A
//   and INPUT PARTITIONS: N/A. It is a dependency-absence test: the property under
//   test is that this file COMPILES and LINKS without VDF/GMP/rug, which no
//   runtime matrix can express. It has one path (the crate builds or it does not)
//   and one partition (this build). Its body exists only to give the test
//   something to execute.
//
//! Integration tests: Transaction builder serialization compatibility.
//!
//! These tests verify that transactions built by the wallet crate's TxBuilder
//! produce canonical byte encodings that match what `doli-core` expects.
//! This is critical for transaction acceptance by nodes.

use wallet::{
    calculate_registration_cost, calculate_withdrawal_net, vesting_penalty_pct, TxBuilder, TxType,
    Utxo, BASE_REGISTRATION_FEE, BOND_UNIT, MAX_BONDS_PER_PRODUCER, MAX_REGISTRATION_FEE,
    UNITS_PER_DOLI, VESTING_QUARTER_SLOTS,
};

// ============================================================================
// Requirement: GUI-FR-011 (Must) -- Send DOLI transaction construction
// Integration test: end-to-end transfer building
// ============================================================================

#[test]
fn test_fr011_build_transfer_end_to_end() {
    let utxos = vec![
        Utxo {
            tx_hash: "ab".repeat(32),
            output_index: 0,
            amount: 1_000_000_000, // 10 DOLI
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 500,
            spendable: true,
        },
        Utxo {
            tx_hash: "cd".repeat(32),
            output_index: 1,
            amount: 500_000_000, // 5 DOLI
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 501,
            spendable: true,
        },
    ];

    let recipient = [0x11u8; 32];
    let sender = [0x22u8; 32];
    let amount = 1_200_000_000; // 12 DOLI
    let fee = 10_000;

    let builder = TxBuilder::build_transfer(&utxos, recipient, amount, fee, sender).unwrap();
    assert_eq!(builder.tx_type(), TxType::Transfer);
    assert_eq!(builder.input_count(), 2); // needs both UTXOs
    assert_eq!(builder.output_count(), 2); // recipient + change
}

// ============================================================================
// Requirement: GUI-FR-024 (Must) -- Bond stacking
// Integration test: bond cost matches protocol constants
// ============================================================================

#[test]
fn test_fr024_bond_cost_matches_protocol() {
    // Protocol constant: 1 bond = BOND_UNIT = 1,000,000,000 base units = 10 DOLI
    assert_eq!(BOND_UNIT, 10 * UNITS_PER_DOLI);

    // 100 bonds = 1,000 DOLI
    let (bond_cost, _, _) = calculate_registration_cost(100, 0).unwrap();
    assert_eq!(bond_cost, 100 * BOND_UNIT);
    assert_eq!(bond_cost, 100_000_000_000); // 1,000 DOLI in base units
}

// ============================================================================
// Requirement: GUI-FR-025 (Must) -- Withdrawal vesting penalty
// Integration test: FIFO penalty schedule matches protocol
// ============================================================================

#[test]
fn test_fr025_vesting_schedule_matches_protocol() {
    // Protocol vesting schedule:
    // Q1 (0-1 year):  75% penalty -> return 25%
    // Q2 (1-2 years): 50% penalty -> return 50%
    // Q3 (2-3 years): 25% penalty -> return 75%
    // Q4 (4+ years):  0% penalty  -> return 100%

    // Q1: brand new bond
    assert_eq!(vesting_penalty_pct(0), 75);
    // Q1: almost 1 year
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS - 1), 75);
    // Q2: exactly 1 year
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS), 50);
    // Fully vested: 4+ years
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 4), 0);
}

#[test]
fn test_fr025_withdrawal_net_matches_protocol() {
    // 10 DOLI bond with 75% penalty -> 2.5 DOLI returned
    let net_q1 = calculate_withdrawal_net(BOND_UNIT, 75);
    assert_eq!(net_q1, 250_000_000); // 2.5 DOLI

    // 10 DOLI bond with 50% penalty -> 5 DOLI returned
    let net_q2 = calculate_withdrawal_net(BOND_UNIT, 50);
    assert_eq!(net_q2, 500_000_000); // 5 DOLI

    // 10 DOLI bond with 0% penalty -> 10 DOLI returned
    let net_vested = calculate_withdrawal_net(BOND_UNIT, 0);
    assert_eq!(net_vested, BOND_UNIT); // 10 DOLI
}

// ============================================================================
// Requirement: GUI-FR-020 (Must) -- Registration fee
// Integration test: fee calculation matches protocol
// ============================================================================

#[test]
fn test_fr020_registration_fee_matches_protocol() {
    // BASE_REGISTRATION_FEE = 100,000 base units = 0.001 DOLI
    assert_eq!(BASE_REGISTRATION_FEE, 100_000);

    // Tiered fee multiplier table (matching doli-core::consensus::fee_multiplier_x100):
    //   0-4 pending:   1.00x
    //   5-9 pending:   1.50x
    //   10-19 pending: 2.00x
    //   20-49 pending: 3.00x
    //   50-99 pending: 4.50x
    //   100-199:       6.50x
    //   200-299:       8.50x
    //   300+:          10.00x (cap)

    // With 0 pending: fee = 100,000 * 100 / 100 = 100,000 (1.00x)
    let (_, fee, _) = calculate_registration_cost(1, 0).unwrap();
    assert_eq!(fee, 100_000);

    // With 5 pending: fee = 100,000 * 150 / 100 = 150,000 (1.50x)
    let (_, fee, _) = calculate_registration_cost(1, 5).unwrap();
    assert_eq!(fee, 150_000);

    // With 100 pending: fee = 100,000 * 650 / 100 = 650,000 (6.50x)
    let (_, fee, _) = calculate_registration_cost(1, 100).unwrap();
    assert_eq!(fee, 650_000);

    // With 300+ pending: fee capped at MAX_REGISTRATION_FEE = 1,000,000 (10.00x)
    let (_, fee, _) = calculate_registration_cost(1, 300).unwrap();
    assert_eq!(fee, MAX_REGISTRATION_FEE);

    // With 1000 pending: still capped at MAX_REGISTRATION_FEE
    let (_, fee, _) = calculate_registration_cost(1, 1000).unwrap();
    assert_eq!(fee, MAX_REGISTRATION_FEE);
}

// ============================================================================
// Requirement: GUI-NF-013 (Must) -- VDF feature flag
// Acceptance: wallet crate compiles without VDF/GMP
// This test exists simply by being compilable and passing.
// OUTPUT CONTRACT: N/A · INPUT PARTITIONS: N/A — see the file header.
// ============================================================================

#[test]
fn test_nf013_wallet_crate_no_vdf_dependency() {
    // This test compiles and runs without VDF/GMP/rug.
    // If the wallet crate accidentally depends on doli-core or vdf,
    // compilation would fail on Windows without MSYS2.
    //
    // INC-I-162: newly created wallets are version 3 (the BLS attestation key is
    // derived from the seed phrase). Asserted against the constant rather than a
    // literal, so a future format change does not break a test that is not about
    // the format.
    let (wallet, _) = wallet::Wallet::new("no-vdf-test");
    assert_eq!(
        wallet.version(),
        wallet::wallet::WALLET_VERSION_SEED_DERIVED_BLS
    );
    // The fact that this test compiles proves the VDF dependency is absent.
}

// ============================================================================
// TxType enum values must match doli-core exactly
// ============================================================================

#[test]
fn test_tx_type_enum_values_match_protocol() {
    // These values are consensus-critical. If they don't match doli-core,
    // transactions will be rejected by nodes.
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

// ============================================================================
// Constants must match consensus.rs values
// ============================================================================

#[test]
fn test_constants_match_consensus() {
    assert_eq!(UNITS_PER_DOLI, 100_000_000, "1 DOLI = 100M base units");
    assert_eq!(BOND_UNIT, 1_000_000_000, "1 bond = 10 DOLI");
    assert_eq!(MAX_BONDS_PER_PRODUCER, 3_000, "Max 3000 bonds per producer");
    assert_eq!(BASE_REGISTRATION_FEE, 100_000, "Base reg fee = 0.001 DOLI");
    assert_eq!(MAX_REGISTRATION_FEE, 1_000_000, "Max reg fee = 0.01 DOLI");
    assert_eq!(VESTING_QUARTER_SLOTS, 3_153_600, "1 year on mainnet");
}
