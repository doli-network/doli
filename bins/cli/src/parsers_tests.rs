// OUTPUT CONTRACT: All outputs, all paths, all input partitions per REQ-SDK-001..005, REQ-SDK-009.
// INPUT PARTITIONS: Documented per test function below. Each test covers one cell from
// the Output Contract tables in specs/sdk-templates-requirements.md.

use super::*;

// =============================================================================
// TEST-SDK-001 — Threshold parser arm (REQ-SDK-001)
// =============================================================================

#[test]
fn threshold_valid_n2_three_simple_subconditions() {
    // INPUT PARTITION: Valid n=2, 3 simple sub-conditions → happy path
    let hex = "aa".repeat(32);
    let input = format!(
        "threshold(2, hashlock({}), timelock(100), timelock_expiry(200))",
        hex
    );
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::Threshold { n, conditions } => {
            assert_eq!(n, 2);
            assert_eq!(conditions.len(), 3);
            assert!(matches!(conditions[0], doli_core::Condition::Hashlock(_)));
            assert!(matches!(conditions[1], doli_core::Condition::Timelock(100)));
            assert!(matches!(
                conditions[2],
                doli_core::Condition::TimelockExpiry(200)
            ));
        }
        _ => panic!("Expected Threshold, got {:?}", cond),
    }
}

#[test]
fn threshold_valid_n1_two_subconditions_minimum() {
    // INPUT PARTITION: Valid n=1, 2 sub-conditions (minimum) → happy path
    let input = "threshold(1, timelock(50), timelock_expiry(100))";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::Threshold { n, conditions } => {
            assert_eq!(n, 1);
            assert_eq!(conditions.len(), 2);
        }
        _ => panic!("Expected Threshold, got {:?}", cond),
    }
}

#[test]
fn threshold_valid_nested_and_or() {
    // INPUT PARTITION: Valid nested — threshold containing and/or → recursive parse
    let hex = "bb".repeat(32);
    let input = format!(
        "threshold(2, and(timelock(10), timelock_expiry(20)), or(timelock(30), hashlock({})))",
        hex
    );
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::Threshold { n, conditions } => {
            assert_eq!(n, 2);
            assert_eq!(conditions.len(), 2);
            assert!(matches!(conditions[0], doli_core::Condition::And(_, _)));
            assert!(matches!(conditions[1], doli_core::Condition::Or(_, _)));
        }
        _ => panic!("Expected Threshold, got {:?}", cond),
    }
}

#[test]
fn threshold_invalid_n_zero() {
    // INPUT PARTITION: Invalid n=0 → validation error
    let input = "threshold(0, timelock(10), timelock(20))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("threshold n must be >= 1"), "got: {}", msg);
}

#[test]
fn threshold_invalid_n_exceeds_count() {
    // INPUT PARTITION: Invalid n > len(conditions) → validation error
    let input = "threshold(3, timelock(10), timelock(20))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("exceeds condition count"), "got: {}", msg);
}

#[test]
fn threshold_invalid_only_one_subcondition() {
    // INPUT PARTITION: Invalid — only 1 sub-condition → validation error
    let input = "threshold(1, timelock(10))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("at least 2 conditions"), "got: {}", msg);
}

#[test]
fn threshold_invalid_too_many_subconditions() {
    // INPUT PARTITION: Invalid — 6+ sub-conditions → validation error (MAX_THRESHOLD_CONDITIONS=5)
    let input = "threshold(2, timelock(1), timelock(2), timelock(3), timelock(4), timelock(5), timelock(6))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("MAX_THRESHOLD_CONDITIONS"), "got: {}", msg);
}

#[test]
fn threshold_invalid_n_not_u8() {
    // INPUT PARTITION: Invalid — n not a u8 → parse error
    let input = "threshold(abc, timelock(10), timelock(20))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid threshold"), "got: {}", msg);
}

#[test]
fn threshold_invalid_malformed_subcondition() {
    // INPUT PARTITION: Invalid — malformed sub-condition → recursive error propagation
    let input = "threshold(1, garbage_condition(foo), timelock(20))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Unknown condition"), "got: {}", msg);
}

#[test]
fn threshold_n_overflow_u8() {
    // INPUT PARTITION: n=256 overflows u8
    let input = "threshold(256, timelock(10), timelock(20))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid threshold"), "got: {}", msg);
}

// =============================================================================
// TEST-SDK-002 — AmountGuard parser arm (REQ-SDK-002)
// =============================================================================

#[test]
fn amount_guard_valid_500_index0() {
    // INPUT PARTITION: Valid "500.0, 0" → happy path
    let input = "amount_guard(500.0, 0)";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::AmountGuard {
            min_amount,
            output_index,
        } => {
            assert_eq!(min_amount, 50_000_000_000); // 500.0 DOLI = 50B units
            assert_eq!(output_index, 0);
        }
        _ => panic!("Expected AmountGuard, got {:?}", cond),
    }
}

#[test]
fn amount_guard_valid_smallest_unit_max_index() {
    // INPUT PARTITION: Valid "0.00000001, 255" (1 unit, max index) → boundary
    let input = "amount_guard(0.00000001, 255)";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::AmountGuard {
            min_amount,
            output_index,
        } => {
            assert_eq!(min_amount, 1);
            assert_eq!(output_index, 255);
        }
        _ => panic!("Expected AmountGuard, got {:?}", cond),
    }
}

#[test]
fn amount_guard_invalid_zero_amount() {
    // INPUT PARTITION: Invalid "0, 0" (zero amount) → validation error
    let input = "amount_guard(0, 0)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("must be greater than zero"), "got: {}", msg);
}

#[test]
fn amount_guard_invalid_non_numeric_amount() {
    // INPUT PARTITION: Invalid "abc, 0" (non-numeric amount) → parse error
    let input = "amount_guard(abc, 0)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid amount"), "got: {}", msg);
}

#[test]
fn amount_guard_invalid_missing_output_index() {
    // INPUT PARTITION: Invalid "500.0" (missing output_index) → arity error
    let input = "amount_guard(500.0)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("amount_guard requires 2 args"), "got: {}", msg);
}

#[test]
fn amount_guard_invalid_too_many_args() {
    // INPUT PARTITION: Invalid "500.0, 0, 1" (too many args) → arity error
    let input = "amount_guard(500.0, 0, 1)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("amount_guard requires 2 args"), "got: {}", msg);
}

#[test]
fn amount_guard_invalid_output_index_overflow() {
    // INPUT PARTITION: Invalid "500.0, 256" (output_index overflow) → parse error
    let input = "amount_guard(500.0, 256)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid output_index"), "got: {}", msg);
}

// =============================================================================
// TEST-SDK-003 — OutputTypeGuard parser arm (REQ-SDK-003)
// =============================================================================

#[test]
fn output_type_guard_valid_normal_index0() {
    // INPUT PARTITION: Valid "normal, 0" → happy path
    let input = "output_type_guard(normal, 0)";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::OutputTypeGuard {
            expected_type,
            output_index,
        } => {
            assert_eq!(expected_type, doli_core::OutputType::Normal);
            assert_eq!(output_index, 0);
        }
        _ => panic!("Expected OutputTypeGuard, got {:?}", cond),
    }
}

#[test]
fn output_type_guard_valid_case_insensitive() {
    // INPUT PARTITION: Valid "HTLC, 1" (case insensitive) → happy path
    let input = "output_type_guard(HTLC, 1)";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::OutputTypeGuard {
            expected_type,
            output_index,
        } => {
            assert_eq!(expected_type, doli_core::OutputType::HTLC);
            assert_eq!(output_index, 1);
        }
        _ => panic!("Expected OutputTypeGuard, got {:?}", cond),
    }
}

#[test]
fn output_type_guard_valid_vesting_max_index() {
    // INPUT PARTITION: Valid "vesting, 255" (max index) → boundary
    let input = "output_type_guard(vesting, 255)";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::OutputTypeGuard {
            expected_type,
            output_index,
        } => {
            assert_eq!(expected_type, doli_core::OutputType::Vesting);
            assert_eq!(output_index, 255);
        }
        _ => panic!("Expected OutputTypeGuard, got {:?}", cond),
    }
}

#[test]
fn output_type_guard_invalid_unknown_type() {
    // INPUT PARTITION: Invalid "unknown_type, 0" → type parse error
    let input = "output_type_guard(unknown_type, 0)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Unknown output type"), "got: {}", msg);
    assert!(
        msg.contains("normal"),
        "error should list valid types, got: {}",
        msg
    );
}

#[test]
fn output_type_guard_invalid_missing_index() {
    // INPUT PARTITION: Invalid "normal" (missing index) → arity error
    let input = "output_type_guard(normal)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("output_type_guard requires 2 args"),
        "got: {}",
        msg
    );
}

#[test]
fn output_type_guard_invalid_non_numeric_index() {
    // INPUT PARTITION: Invalid "normal, abc" (non-numeric index) → parse error
    let input = "output_type_guard(normal, abc)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid output_index"), "got: {}", msg);
}

#[test]
fn output_type_guard_all_15_variants() {
    // INPUT PARTITION: All 15 OutputType variants accepted by name, case-insensitive
    let variants = [
        ("normal", doli_core::OutputType::Normal),
        ("bond", doli_core::OutputType::Bond),
        ("multisig", doli_core::OutputType::Multisig),
        ("hashlock", doli_core::OutputType::Hashlock),
        ("htlc", doli_core::OutputType::HTLC),
        ("vesting", doli_core::OutputType::Vesting),
        ("nft", doli_core::OutputType::NFT),
        ("fungibleasset", doli_core::OutputType::FungibleAsset),
        ("bridgehtlc", doli_core::OutputType::BridgeHTLC),
        ("pool", doli_core::OutputType::Pool),
        ("lpshare", doli_core::OutputType::LPShare),
        ("collateral", doli_core::OutputType::Collateral),
        ("lendingdeposit", doli_core::OutputType::LendingDeposit),
        ("zkrollup", doli_core::OutputType::ZKRollup),
        ("encryptedcontent", doli_core::OutputType::EncryptedContent),
    ];
    for (name, expected_type) in &variants {
        let input = format!("output_type_guard({}, 0)", name);
        let cond = parse_condition(&input)
            .unwrap_or_else(|e| panic!("Failed to parse output_type_guard({}, 0): {}", name, e));
        match cond {
            doli_core::Condition::OutputTypeGuard {
                expected_type: t, ..
            } => {
                assert_eq!(t, *expected_type, "Mismatch for type name '{}'", name);
            }
            _ => panic!("Expected OutputTypeGuard for '{}', got {:?}", name, cond),
        }
    }
}

// =============================================================================
// TEST-SDK-004 — RecipientGuard parser arm (REQ-SDK-004)
// =============================================================================

#[test]
fn recipient_guard_valid_hex_hash_index0() {
    // INPUT PARTITION: Valid hex hash + index 0 → happy path (hex)
    let hex = "cc".repeat(32);
    let input = format!("recipient_guard({}, 0)", hex);
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::RecipientGuard {
            expected_pubkey_hash,
            output_index,
        } => {
            assert_eq!(expected_pubkey_hash, crypto::Hash::from_hex(&hex).unwrap());
            assert_eq!(output_index, 0);
        }
        _ => panic!("Expected RecipientGuard, got {:?}", cond),
    }
}

#[test]
fn recipient_guard_valid_hex_hash_index1() {
    // INPUT PARTITION: Valid hex hash + index 1 → happy path
    let hex = "dd".repeat(32);
    let input = format!("recipient_guard({}, 1)", hex);
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::RecipientGuard {
            expected_pubkey_hash,
            output_index,
        } => {
            assert_eq!(expected_pubkey_hash, crypto::Hash::from_hex(&hex).unwrap());
            assert_eq!(output_index, 1);
        }
        _ => panic!("Expected RecipientGuard, got {:?}", cond),
    }
}

#[test]
fn recipient_guard_invalid_missing_index() {
    // INPUT PARTITION: Invalid — missing index → arity error
    let hex = "ee".repeat(32);
    let input = format!("recipient_guard({})", hex);
    let err = parse_condition(&input).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("recipient_guard requires 2 args"),
        "got: {}",
        msg
    );
}

#[test]
fn recipient_guard_invalid_bad_address() {
    // INPUT PARTITION: Invalid "not_an_address, 0" → address resolution error
    let input = "recipient_guard(not_an_address, 0)";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid address"), "got: {}", msg);
}

#[test]
fn recipient_guard_invalid_too_many_args() {
    // INPUT PARTITION: Invalid — too many args → arity error
    let hex = "ff".repeat(32);
    let input = format!("recipient_guard({}, 0, extra)", hex);
    let err = parse_condition(&input).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("recipient_guard requires 2 args"),
        "got: {}",
        msg
    );
}

// =============================================================================
// TEST-SDK-005 — Guard composition with and/or (REQ-SDK-005)
// =============================================================================

#[test]
fn composition_and_two_guards() {
    // INPUT PARTITION: And(guard, guard) → recursive parse → And
    let hex = "aa".repeat(32);
    let input = format!("and(amount_guard(500.0, 0), recipient_guard({}, 0))", hex);
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::And(left, right) => {
            assert!(matches!(*left, doli_core::Condition::AmountGuard { .. }));
            assert!(matches!(
                *right,
                doli_core::Condition::RecipientGuard { .. }
            ));
        }
        _ => panic!("Expected And, got {:?}", cond),
    }
}

#[test]
fn composition_or_guard_and_timelock() {
    // INPUT PARTITION: Or(guard, timelock) → recursive parse → Or
    let input = "or(amount_guard(100.0, 0), timelock(1000))";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::Or(left, right) => {
            assert!(matches!(*left, doli_core::Condition::AmountGuard { .. }));
            assert!(matches!(*right, doli_core::Condition::Timelock(1000)));
        }
        _ => panic!("Expected Or, got {:?}", cond),
    }
}

#[test]
fn composition_and_threshold_with_guard() {
    // INPUT PARTITION: And(threshold(...), guard) → deep recursion
    let hex = "bb".repeat(32);
    let hash2 = "cc".repeat(32);
    let input = format!(
        "and(threshold(2, hashlock({}), timelock(100)), recipient_guard({}, 0))",
        hex, hash2
    );
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::And(left, right) => {
            assert!(matches!(*left, doli_core::Condition::Threshold { .. }));
            assert!(matches!(
                *right,
                doli_core::Condition::RecipientGuard { .. }
            ));
        }
        _ => panic!("Expected And, got {:?}", cond),
    }
}

#[test]
fn composition_and_three_args_error() {
    // INPUT PARTITION: And with 3 args → and arity check
    let input = "and(timelock(1), timelock(2), timelock(3))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("and requires exactly 2 args"), "got: {}", msg);
}

#[test]
fn composition_or_one_arg_error() {
    // INPUT PARTITION: Or with 1 arg → or arity check
    let input = "or(timelock(1))";
    let err = parse_condition(input).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("or requires exactly 2 args"), "got: {}", msg);
}

// =============================================================================
// REQ-SDK-009 — Existing arm regression tests
// =============================================================================

#[test]
fn existing_multisig_positive() {
    let hex1 = "aa".repeat(32);
    let hex2 = "bb".repeat(32);
    let input = format!("multisig(2, {}, {})", hex1, hex2);
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::Multisig { threshold, keys } => {
            assert_eq!(threshold, 2);
            assert_eq!(keys.len(), 2);
        }
        _ => panic!("Expected Multisig, got {:?}", cond),
    }
}

#[test]
fn existing_multisig_negative_too_few_args() {
    let hex1 = "aa".repeat(32);
    let input = format!("multisig(2, {})", hex1);
    let err = parse_condition(&input).unwrap_err();
    assert!(err
        .to_string()
        .contains("multisig requires at least 3 args"));
}

#[test]
fn existing_hashlock_positive() {
    let hex = "dd".repeat(32);
    let input = format!("hashlock({})", hex);
    let cond = parse_condition(&input).unwrap();
    assert!(matches!(cond, doli_core::Condition::Hashlock(_)));
}

#[test]
fn existing_hashlock_negative() {
    let input = "hashlock(not_hex)";
    let err = parse_condition(input).unwrap_err();
    assert!(err.to_string().contains("Invalid hex hash"));
}

#[test]
fn existing_timelock_positive() {
    let input = "timelock(12345)";
    let cond = parse_condition(input).unwrap();
    assert!(matches!(cond, doli_core::Condition::Timelock(12345)));
}

#[test]
fn existing_timelock_negative() {
    let input = "timelock(abc)";
    let err = parse_condition(input).unwrap_err();
    assert!(err.to_string().contains("Invalid height"));
}

#[test]
fn existing_timelock_expiry_positive() {
    let input = "timelock_expiry(99999)";
    let cond = parse_condition(input).unwrap();
    assert!(matches!(cond, doli_core::Condition::TimelockExpiry(99999)));
}

#[test]
fn existing_vesting_positive() {
    let hex = "ee".repeat(32);
    let input = format!("vesting({}, 500)", hex);
    let cond = parse_condition(&input).unwrap();
    match cond {
        doli_core::Condition::And(left, right) => {
            assert!(matches!(*left, doli_core::Condition::Signature(_)));
            assert!(matches!(*right, doli_core::Condition::Timelock(500)));
        }
        _ => panic!(
            "Expected And(Signature, Timelock) for vesting, got {:?}",
            cond
        ),
    }
}

#[test]
fn existing_htlc_positive() {
    let hash = "aa".repeat(32);
    let refund = "bb".repeat(32);
    let input = format!("htlc({}, 100, 200, {})", hash, refund);
    let cond = parse_condition(&input).unwrap();
    // htlc produces Or(And(Hashlock, Timelock), And(Signature, TimelockExpiry))
    assert!(matches!(cond, doli_core::Condition::Or(_, _)));
}

#[test]
fn existing_unknown_condition_error() {
    let input = "totally_unknown(1, 2, 3)";
    let err = parse_condition(input).unwrap_err();
    assert!(err.to_string().contains("Unknown condition"));
}

// =============================================================================
// condition_to_output_type tests
// =============================================================================

#[test]
fn output_type_mapping_multisig() {
    let cond = doli_core::Condition::multisig(2, vec![crypto::Hash::default(); 2]);
    assert_eq!(
        condition_to_output_type(&cond),
        doli_core::OutputType::Multisig
    );
}

#[test]
fn output_type_mapping_hashlock() {
    let cond = doli_core::Condition::hashlock(crypto::Hash::default());
    assert_eq!(
        condition_to_output_type(&cond),
        doli_core::OutputType::Hashlock
    );
}

#[test]
fn output_type_mapping_or() {
    let cond = doli_core::Condition::Or(
        Box::new(doli_core::Condition::Timelock(1)),
        Box::new(doli_core::Condition::TimelockExpiry(2)),
    );
    assert_eq!(condition_to_output_type(&cond), doli_core::OutputType::HTLC);
}

#[test]
fn output_type_mapping_and() {
    let cond = doli_core::Condition::And(
        Box::new(doli_core::Condition::Signature(crypto::Hash::default())),
        Box::new(doli_core::Condition::Timelock(100)),
    );
    assert_eq!(
        condition_to_output_type(&cond),
        doli_core::OutputType::Vesting
    );
}

#[test]
fn output_type_mapping_timelocks() {
    assert_eq!(
        condition_to_output_type(&doli_core::Condition::Timelock(1)),
        doli_core::OutputType::Vesting
    );
    assert_eq!(
        condition_to_output_type(&doli_core::Condition::TimelockExpiry(1)),
        doli_core::OutputType::Vesting
    );
}

#[test]
fn output_type_mapping_signature() {
    let cond = doli_core::Condition::Signature(crypto::Hash::default());
    assert_eq!(
        condition_to_output_type(&cond),
        doli_core::OutputType::Normal
    );
}

#[test]
fn output_type_mapping_threshold() {
    let cond = doli_core::Condition::Threshold {
        n: 1,
        conditions: vec![
            doli_core::Condition::Timelock(1),
            doli_core::Condition::Timelock(2),
        ],
    };
    assert_eq!(
        condition_to_output_type(&cond),
        doli_core::OutputType::Multisig
    );
}

#[test]
fn output_type_mapping_guards() {
    // All guard variants map to Multisig (known limitation)
    let ag = doli_core::Condition::amount_guard(100, 0);
    let otg = doli_core::Condition::output_type_guard(doli_core::OutputType::Normal, 0);
    let rg = doli_core::Condition::recipient_guard(crypto::Hash::default(), 0);
    assert_eq!(
        condition_to_output_type(&ag),
        doli_core::OutputType::Multisig
    );
    assert_eq!(
        condition_to_output_type(&otg),
        doli_core::OutputType::Multisig
    );
    assert_eq!(
        condition_to_output_type(&rg),
        doli_core::OutputType::Multisig
    );
}

// =============================================================================
// split_top_level tests
// =============================================================================

#[test]
fn split_top_level_no_nesting() {
    let parts = split_top_level("a, b, c");
    assert_eq!(parts, vec!["a", "b", "c"]);
}

#[test]
fn split_top_level_single_nesting() {
    let parts = split_top_level("foo(a, b), bar");
    assert_eq!(parts, vec!["foo(a, b)", "bar"]);
}

#[test]
fn split_top_level_double_nesting() {
    let parts = split_top_level("foo(a, bar(x, y)), baz");
    assert_eq!(parts, vec!["foo(a, bar(x, y))", "baz"]);
}

#[test]
fn split_top_level_empty_string() {
    let parts = split_top_level("");
    assert!(parts.is_empty());
}

#[test]
fn split_top_level_trailing_comma() {
    let parts = split_top_level("a, b, ");
    assert_eq!(parts, vec!["a", "b"]);
}

// =============================================================================
// Additional edge cases for completeness (REQ-SDK-009)
// =============================================================================

#[test]
fn parse_condition_missing_open_paren() {
    let err = parse_condition("timelock100)").unwrap_err();
    assert!(err.to_string().contains("Expected condition format"));
}

#[test]
fn parse_condition_missing_close_paren() {
    let err = parse_condition("timelock(100").unwrap_err();
    assert!(err.to_string().contains("Missing closing parenthesis"));
}

#[test]
fn threshold_with_nested_threshold() {
    // Threshold containing another threshold
    let input = "threshold(1, threshold(1, timelock(10), timelock(20)), timelock(30))";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::Threshold { n, conditions } => {
            assert_eq!(n, 1);
            assert_eq!(conditions.len(), 2);
            assert!(matches!(
                conditions[0],
                doli_core::Condition::Threshold { .. }
            ));
        }
        _ => panic!("Expected Threshold, got {:?}", cond),
    }
}

#[test]
fn amount_guard_whole_number_without_decimal() {
    // "500" without decimal point — coins_to_units should handle this
    let input = "amount_guard(500, 0)";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::AmountGuard {
            min_amount,
            output_index,
        } => {
            assert_eq!(min_amount, 50_000_000_000);
            assert_eq!(output_index, 0);
        }
        _ => panic!("Expected AmountGuard, got {:?}", cond),
    }
}

#[test]
fn threshold_n_equals_count() {
    // n equals exactly the number of conditions (valid: n-of-n)
    let input = "threshold(3, timelock(1), timelock(2), timelock(3))";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::Threshold { n, conditions } => {
            assert_eq!(n, 3);
            assert_eq!(conditions.len(), 3);
        }
        _ => panic!("Expected Threshold, got {:?}", cond),
    }
}

#[test]
fn threshold_max_5_subconditions() {
    // Exactly 5 sub-conditions (at limit, should succeed)
    let input = "threshold(2, timelock(1), timelock(2), timelock(3), timelock(4), timelock(5))";
    let cond = parse_condition(input).unwrap();
    match cond {
        doli_core::Condition::Threshold { n, conditions } => {
            assert_eq!(n, 2);
            assert_eq!(conditions.len(), 5);
        }
        _ => panic!("Expected Threshold, got {:?}", cond),
    }
}
