// OUTPUT CONTRACT: fn TxType::from_u32(v: u32) -> Option<TxType>
//                  fn OutputType::from_u8(v: u8) -> Option<OutputType>
//   Outputs:
//     O1: TxType::from_u32 for tombstoned lending discriminants (24..=28)
//     O2: OutputType::from_u8 for tombstoned lending discriminants (11, 12)
//   PATHS:
//     P1: from_u32(24) → None (was CreateLoan, tombstoned B.1)
//     P2: from_u32(25) → None (was RepayLoan, tombstoned B.1)
//     P3: from_u32(26) → None (was LiquidateLoan, tombstoned B.1)
//     P4: from_u32(27) → None (was LendingDeposit, tombstoned B.1)
//     P5: from_u32(28) → None (was LendingWithdraw, tombstoned B.1)
//     P6: from_u8(11) → None (was Collateral, tombstoned B.1)
//     P7: from_u8(12) → None (was LendingDeposit, tombstoned B.1)
//   INPUT PARTITIONS:
//     Each discriminant is its own partition — a regression that reactivates
//     one discriminant without the others must be caught individually.
//   MATRIX (7 paths x 1 assertion each):
//     P1..P5 → 5 assertions (TxType tombstoned)
//     P6..P7 → 2 assertions (OutputType tombstoned)
//
// B.1 tombstone regression test (DeFi L1 Foundations Architecture).
// These discriminants are permanently retired — they must NEVER be
// reused for new types. If this test fails, someone reactivated a
// tombstoned discriminant. See specs/defi-l1-foundations-architecture.md B.1.

use doli_core::transaction::{OutputType, TxType};

/// TxType discriminants 24-28 are tombstoned (lending subsystem B.1).
/// They must return None from from_u32 permanently.
#[test]
fn tombstoned_lending_tx_types_return_none() {
    // 24 = CreateLoan (tombstoned)
    assert!(
        TxType::from_u32(24).is_none(),
        "TxType 24 (was CreateLoan) must be tombstoned — from_u32 must return None"
    );
    // 25 = RepayLoan (tombstoned)
    assert!(
        TxType::from_u32(25).is_none(),
        "TxType 25 (was RepayLoan) must be tombstoned — from_u32 must return None"
    );
    // 26 = LiquidateLoan (tombstoned)
    assert!(
        TxType::from_u32(26).is_none(),
        "TxType 26 (was LiquidateLoan) must be tombstoned — from_u32 must return None"
    );
    // 27 = LendingDeposit (tombstoned)
    assert!(
        TxType::from_u32(27).is_none(),
        "TxType 27 (was LendingDeposit) must be tombstoned — from_u32 must return None"
    );
    // 28 = LendingWithdraw (tombstoned)
    assert!(
        TxType::from_u32(28).is_none(),
        "TxType 28 (was LendingWithdraw) must be tombstoned — from_u32 must return None"
    );
}

/// OutputType discriminants 11-12 are tombstoned (lending subsystem B.1).
/// They must return None from from_u8 permanently.
#[test]
fn tombstoned_lending_output_types_return_none() {
    // 11 = Collateral (tombstoned)
    assert!(
        OutputType::from_u8(11).is_none(),
        "OutputType 11 (was Collateral) must be tombstoned — from_u8 must return None"
    );
    // 12 = LendingDeposit (tombstoned)
    assert!(
        OutputType::from_u8(12).is_none(),
        "OutputType 12 (was LendingDeposit) must be tombstoned — from_u8 must return None"
    );
}

/// Verify that adjacent discriminants are NOT affected by tombstoning.
/// This catches over-broad removal patterns.
#[test]
fn adjacent_discriminants_unaffected() {
    // TxType: 22 = Swap (active), 29 = FractionalizeNft (active until B.2), 31 = ZKSettle
    assert!(
        TxType::from_u32(22).is_some(),
        "TxType 22 (Swap) must remain active"
    );
    assert!(
        TxType::from_u32(29).is_some(),
        "TxType 29 (FractionalizeNft) must remain active (B.2 scope, not B.1)"
    );
    assert!(
        TxType::from_u32(31).is_some(),
        "TxType 31 (ZKSettle) must remain active"
    );

    // OutputType: 10 = LPShare (active), 13 = ZKRollup (active)
    assert!(
        OutputType::from_u8(10).is_some(),
        "OutputType 10 (LPShare) must remain active"
    );
    assert!(
        OutputType::from_u8(13).is_some(),
        "OutputType 13 (ZKRollup) must remain active"
    );
}
