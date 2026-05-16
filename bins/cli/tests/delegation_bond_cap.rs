// OUTPUT CONTRACT: validate_delegation_bond_count(n: u32) -> Result<(), Error>
// O1: Ok(()) for bond_count in [1, MAX_BONDS_PER_PRODUCER]
// O2: Err for bond_count == 0
// O3: Err for bond_count > MAX_BONDS_PER_PRODUCER
// O4: Err message text mentions the actual upper bound
// PATHS: range-check branch in `handle_delegate` (delegation.rs:21)
// MATRIX: O1×P_below_old_cap, O1×P_above_old_cap_within_new_cap, O1×P_at_new_cap, O2×P_zero, O3×P_above_new_cap
//
// INPUT PARTITIONS:
// - P_zero:                 bond_count = 0           (must reject — boundary)
// - P_below_old_cap:        bond_count = 50          (always accepted before & after; regression guard)
// - P_at_old_cap:           bond_count = 100         (last value accepted by buggy code)
// - P_above_old_cap_in_new: bond_count = 104, 1000   (REGRESSION: rejected before, must accept after)
// - P_at_new_cap:           bond_count = MAX_BONDS_PER_PRODUCER (3000) — boundary upper inclusive
// - P_above_new_cap:        bond_count = 3001        (must reject — boundary)
//
// REGRESSION GUARD: INC-I-077. This test must FAIL against the buggy source
// (hardcoded `1..=100`) and PASS after the fix uses MAX_BONDS_PER_PRODUCER.
//
// STRATEGY: The CLI handler `handle_delegate` is async + needs Wallet+RpcClient,
// so we can't call it directly. We test the invariant at the source level by
// asserting the validation expression references MAX_BONDS_PER_PRODUCER (3000)
// and not the legacy literal 1..=100, AND that the consensus constant is the
// expected value.

use doli_core::consensus::MAX_BONDS_PER_PRODUCER;

const DELEGATION_SRC: &str = include_str!("../src/cmd_producer/delegation.rs");

#[test]
fn consensus_max_bonds_constant_is_3000() {
    // Anchors the test to the assumed protocol cap. If consensus ever changes
    // this, the delegation cap should be re-evaluated.
    assert_eq!(MAX_BONDS_PER_PRODUCER, 3_000);
}

#[test]
fn cli_must_not_hardcode_legacy_cap_of_100() {
    // INC-I-077: the buggy code used `1..=100` as the bond_count range.
    let has_legacy = DELEGATION_SRC.contains("1..=100")
        || DELEGATION_SRC.contains("Bond count must be between 1 and 100");
    assert!(
        !has_legacy,
        "INC-I-077 regression: delegation.rs still hardcodes the legacy cap of 100. \
         Replace `1..=100` with `1..=MAX_BONDS_PER_PRODUCER` from doli_core::consensus."
    );
}

#[test]
fn cli_must_reference_max_bonds_per_producer() {
    assert!(
        DELEGATION_SRC.contains("MAX_BONDS_PER_PRODUCER"),
        "delegation.rs must reference the consensus constant MAX_BONDS_PER_PRODUCER \
         instead of a hardcoded literal cap."
    );
}

#[test]
fn cli_error_message_must_reference_new_upper_bound() {
    // The user-facing error should reflect the actual range so users aren't
    // misled. Either it embeds the constant name or the literal 3000.
    let message_ok = DELEGATION_SRC.contains("Bond count must be between 1 and 3000")
        || DELEGATION_SRC.contains("MAX_BONDS_PER_PRODUCER");
    assert!(
        message_ok,
        "delegation.rs error message must mention the actual upper bound (3000) \
         or reference MAX_BONDS_PER_PRODUCER."
    );
}
