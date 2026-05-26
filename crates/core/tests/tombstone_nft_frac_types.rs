// OUTPUT CONTRACT: fn TxType::from_u32(v: u32) -> Option<TxType>
//   Outputs:
//     O1: TxType::from_u32 for tombstoned NFT-frac discriminants (29, 30)
//   PATHS:
//     P1: from_u32(29) → None (was FractionalizeNft, tombstoned B.2)
//     P2: from_u32(30) → None (was RedeemNft, tombstoned B.2)
//   INPUT PARTITIONS:
//     Each discriminant is its own partition — a regression that reactivates
//     one discriminant without the other must be caught individually.
//   MATRIX (2 paths x 1 assertion each):
//     P1 → 1 assertion (TxType 29 tombstoned)
//     P2 → 1 assertion (TxType 30 tombstoned)
//
// B.2 tombstone regression test (DeFi L1 Foundations Architecture).
// These discriminants are permanently retired — they must NEVER be
// reused for new types. If this test fails, someone reactivated a
// tombstoned discriminant. See specs/defi-l1-foundations-architecture.md B.2.

use doli_core::transaction::TxType;

/// TxType discriminants 29-30 are tombstoned (NFT fractionalization B.2).
/// They must return None from from_u32 permanently.
#[test]
fn tombstone_nft_frac_types() {
    // 29 = FractionalizeNft (tombstoned)
    assert!(
        TxType::from_u32(29).is_none(),
        "TxType 29 (was FractionalizeNft) must be tombstoned — from_u32 must return None"
    );
    // 30 = RedeemNft (tombstoned)
    assert!(
        TxType::from_u32(30).is_none(),
        "TxType 30 (was RedeemNft) must be tombstoned — from_u32 must return None"
    );
}

/// Verify that adjacent discriminants are NOT affected by B.2 tombstoning.
/// This catches over-broad removal patterns.
#[test]
fn adjacent_discriminants_unaffected_by_b2() {
    // TxType: 22 = Swap (active), 28 = tombstoned (B.1), 31 = ZKSettle (active)
    assert!(
        TxType::from_u32(22).is_some(),
        "TxType 22 (Swap) must remain active"
    );
    assert!(
        TxType::from_u32(28).is_none(),
        "TxType 28 (was LendingWithdraw) must remain tombstoned (B.1)"
    );
    assert!(
        TxType::from_u32(31).is_some(),
        "TxType 31 (ZKSettle) must remain active"
    );
}
