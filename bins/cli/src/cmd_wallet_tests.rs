// OUTPUT CONTRACT: All outputs, all paths, all input partitions per REQ-SDK-006.
//
// | Input Partition                        | Code Path    | Expected Output     |
// |----------------------------------------|--------------|---------------------|
// | Mainnet + condition with AmountGuard   | warning path | returns true        |
// | Mainnet + condition without guards     | no-warning   | returns false       |
// | Devnet  + condition with guard         | no-warning   | returns false       |
// | Testnet + condition with guard         | no-warning   | returns false       |
//
// INPUT PARTITIONS:
//   Path 1 (warning): network is mainnet (explicit or unset) AND cond.contains_guard() == true
//     P1a: Explicit "mainnet" + simple guard (AmountGuard)
//     P1b: None/unset network + simple guard (OutputTypeGuard) — default = mainnet
//     P1c: Explicit "mainnet" + threshold containing guard (RecipientGuard)
//   Path 2 (no-warning): network is NOT mainnet OR cond.contains_guard() == false
//     P2a: Explicit "mainnet" + non-guard condition (multisig)
//     P2b: "devnet" + guard condition
//     P2c: "testnet" + guard condition

use doli_core::Condition;

// =============================================================================
// TEST-SDK-006 — Mainnet guard warning (REQ-SDK-006)
// =============================================================================

// --- Path 1: warning (returns true) ---

#[test]
fn mainnet_with_guard_returns_true() {
    // INPUT PARTITION P1a: Explicit "mainnet" + AmountGuard → warning
    let cond = Condition::AmountGuard {
        min_amount: 100,
        output_index: 0,
    };
    assert!(super::should_warn_mainnet_guards(Some("mainnet"), &cond));
}

#[test]
fn mainnet_unset_with_guard_returns_true() {
    // INPUT PARTITION P1b: None/unset network + OutputTypeGuard → warning (default = mainnet)
    let cond = Condition::OutputTypeGuard {
        expected_type: doli_core::OutputType::Normal,
        output_index: 0,
    };
    assert!(super::should_warn_mainnet_guards(None, &cond));
}

#[test]
fn mainnet_threshold_containing_guard_returns_true() {
    // INPUT PARTITION P1c: Explicit "mainnet" + threshold containing RecipientGuard → warning
    let inner_guard = Condition::RecipientGuard {
        expected_pubkey_hash: crypto::Hash::from_bytes([0xbb; 32]),
        output_index: 0,
    };
    let inner_hash = Condition::Hashlock(crypto::Hash::from_bytes([0xcc; 32]));
    let cond = Condition::Threshold {
        n: 1,
        conditions: vec![inner_guard, inner_hash],
    };
    assert!(super::should_warn_mainnet_guards(Some("mainnet"), &cond));
}

// --- Path 2: no-warning (returns false) ---

#[test]
fn mainnet_without_guard_returns_false() {
    // INPUT PARTITION P2a: Explicit "mainnet" + multisig (no guard) → no warning
    let key_hash = crypto::Hash::from_bytes([0xaa; 32]);
    let cond = Condition::multisig(2, vec![key_hash, key_hash]);
    assert!(!super::should_warn_mainnet_guards(Some("mainnet"), &cond));
}

#[test]
fn devnet_with_guard_returns_false() {
    // INPUT PARTITION P2b: "devnet" + AmountGuard → no warning
    let cond = Condition::AmountGuard {
        min_amount: 100,
        output_index: 0,
    };
    assert!(!super::should_warn_mainnet_guards(Some("devnet"), &cond));
}

#[test]
fn testnet_with_guard_returns_false() {
    // INPUT PARTITION P2c: "testnet" + AmountGuard → no warning
    let cond = Condition::AmountGuard {
        min_amount: 100,
        output_index: 0,
    };
    assert!(!super::should_warn_mainnet_guards(Some("testnet"), &cond));
}
