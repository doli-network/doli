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

// =============================================================================
// TEST-SDK-007 — Multi-output spend (REQ-SDK-007)
// =============================================================================
//
// OUTPUT CONTRACT: All outputs, all paths, all input partitions per REQ-SDK-007.
//
// | Input Partition                              | Code Path         | Expected Output                              |
// |----------------------------------------------|-------------------|----------------------------------------------|
// | P1:  Single valid normal spec                | parse success     | Ok((0, Normal, hash, amount))                |
// | P2:  Valid vesting spec                      | parse success     | Ok((idx, Vesting, hash, amount))             |
// | P3:  Valid nft spec (S4 mitigation)          | parse success     | Ok((idx, NFT, hash, amount))                 |
// | P4:  Invalid type "bond"                     | type rejection    | Err("cannot be used in spend")               |
// | P5:  Invalid amount "abc"                    | parse error       | Err("Invalid amount")                        |
// | P6:  Too few colons                          | format error      | Err("Expected format")                       |
// | P7:  Contiguous indices 0,1,2                | validation pass   | Ok(sorted vec)                               |
// | P8:  Gap in indices (0,2)                    | validation error  | Err("Missing index 1")                       |
// | P9:  Duplicate index                         | validation error  | Err("Duplicate output index")                |
// | P10: 9 outputs (exceeds cap)                 | cap error         | Err("Maximum 8 outputs")                     |
// | P11: Fee > 1% of input AND > 10000 units     | fee warning       | should_warn = true                           |
// | P12: Fee <= 1% of input                      | no warning        | should_warn = false                          |
// | P13: Fee > 10000 but <= 1% of input          | no warning        | should_warn = false                          |
// | P14: 8 outputs exactly (at cap)              | validation pass   | Ok(sorted vec)                               |
// | P15: Case-insensitive type "Normal"          | parse success     | Ok((idx, Normal, ...))                       |

// --- parse_output_spec tests ---

#[test]
fn parse_output_spec_valid_normal() {
    // INPUT PARTITION P1: Single valid normal spec → Ok
    let hex_addr = "aa".repeat(32);
    let spec = format!("0:normal:{}:500.0", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    let (idx, otype, hash, amount) = result.unwrap();
    assert_eq!(idx, 0);
    assert_eq!(otype, doli_core::OutputType::Normal);
    assert_eq!(hash, crypto::Hash::from_bytes([0xaa; 32]));
    assert_eq!(amount, 50_000_000_000); // 500.0 DOLI = 500 * 10^8
}

#[test]
fn parse_output_spec_valid_vesting() {
    // INPUT PARTITION P2: Valid vesting spec → Ok
    let hex_addr = "bb".repeat(32);
    let spec = format!("1:vesting:{}:100.0", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    let (idx, otype, _hash, amount) = result.unwrap();
    assert_eq!(idx, 1);
    assert_eq!(otype, doli_core::OutputType::Vesting);
    assert_eq!(amount, 10_000_000_000); // 100.0 DOLI
}

#[test]
fn parse_output_spec_valid_nft() {
    // INPUT PARTITION P3: Valid NFT spec (S4 mitigation: NFT IS user-constructible)
    let hex_addr = "cc".repeat(32);
    let spec = format!("2:nft:{}:1.0", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    let (idx, otype, _hash, _amount) = result.unwrap();
    assert_eq!(idx, 2);
    assert_eq!(otype, doli_core::OutputType::NFT);
}

#[test]
fn parse_output_spec_case_insensitive() {
    // INPUT PARTITION P15: Case-insensitive type "Normal" → Ok
    let hex_addr = "aa".repeat(32);
    let spec = format!("0:Normal:{}:10.0", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    let (_, otype, _, _) = result.unwrap();
    assert_eq!(otype, doli_core::OutputType::Normal);
}

#[test]
fn parse_output_spec_invalid_type_bond() {
    // INPUT PARTITION P4: "bond" → rejected (protocol-internal)
    let hex_addr = "aa".repeat(32);
    let spec = format!("0:bond:{}:100.0", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("cannot be used in spend"),
        "Expected type rejection error, got: {}",
        err
    );
}

#[test]
fn parse_output_spec_invalid_amount() {
    // INPUT PARTITION P5: Invalid amount "abc" → parse error
    let hex_addr = "aa".repeat(32);
    let spec = format!("0:normal:{}:abc", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Invalid amount"),
        "Expected amount error, got: {}",
        err
    );
}

#[test]
fn parse_output_spec_too_few_parts() {
    // INPUT PARTITION P6: Too few colons → format error
    let result = super::parse_output_spec("0:normal");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Expected format"),
        "Expected format error, got: {}",
        err
    );
}

#[test]
fn parse_output_spec_invalid_index() {
    // Invalid index "x" → parse error
    let hex_addr = "aa".repeat(32);
    let spec = format!("x:normal:{}:100.0", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Invalid output index"),
        "Expected index error, got: {}",
        err
    );
}

#[test]
fn parse_output_spec_unknown_type() {
    // Unknown type "foobar" → error
    let hex_addr = "aa".repeat(32);
    let spec = format!("0:foobar:{}:100.0", hex_addr);
    let result = super::parse_output_spec(&spec);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Unknown output type"),
        "Expected unknown type error, got: {}",
        err
    );
}

// --- validate_output_specs tests ---

#[test]
fn validate_output_specs_contiguous() {
    // INPUT PARTITION P7: Contiguous indices 0,1,2 → Ok
    use doli_core::OutputType;
    let hash = crypto::Hash::from_bytes([0xaa; 32]);
    let specs = vec![
        (0u8, OutputType::Normal, hash, 100u64),
        (1u8, OutputType::Normal, hash, 200u64),
        (2u8, OutputType::Normal, hash, 300u64),
    ];
    let result = super::validate_output_specs(&specs);
    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    let outputs = result.unwrap();
    assert_eq!(outputs.len(), 3);
    assert_eq!(outputs[0].amount, 100);
    assert_eq!(outputs[1].amount, 200);
    assert_eq!(outputs[2].amount, 300);
}

#[test]
fn validate_output_specs_gap() {
    // INPUT PARTITION P8: Gap in indices (0,2) → error
    use doli_core::OutputType;
    let hash = crypto::Hash::from_bytes([0xaa; 32]);
    let specs = vec![
        (0u8, OutputType::Normal, hash, 100u64),
        (2u8, OutputType::Normal, hash, 300u64),
    ];
    let result = super::validate_output_specs(&specs);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Missing index 1"),
        "Expected missing index error, got: {}",
        err
    );
}

#[test]
fn validate_output_specs_duplicate() {
    // INPUT PARTITION P9: Duplicate index → error
    use doli_core::OutputType;
    let hash = crypto::Hash::from_bytes([0xaa; 32]);
    let specs = vec![
        (0u8, OutputType::Normal, hash, 100u64),
        (0u8, OutputType::Normal, hash, 200u64),
    ];
    let result = super::validate_output_specs(&specs);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Duplicate output index"),
        "Expected duplicate index error, got: {}",
        err
    );
}

#[test]
fn validate_output_specs_exceeds_cap() {
    // INPUT PARTITION P10: 9 outputs → cap error
    use doli_core::OutputType;
    let hash = crypto::Hash::from_bytes([0xaa; 32]);
    let specs: Vec<_> = (0..9)
        .map(|i| (i as u8, OutputType::Normal, hash, 100u64))
        .collect();
    let result = super::validate_output_specs(&specs);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Maximum 8 outputs"),
        "Expected cap error, got: {}",
        err
    );
}

#[test]
fn validate_output_specs_at_cap() {
    // INPUT PARTITION P14: Exactly 8 outputs → Ok
    use doli_core::OutputType;
    let hash = crypto::Hash::from_bytes([0xaa; 32]);
    let specs: Vec<_> = (0..8)
        .map(|i| (i as u8, OutputType::Normal, hash, 100u64))
        .collect();
    let result = super::validate_output_specs(&specs);
    assert!(
        result.is_ok(),
        "Expected Ok for 8 outputs, got {:?}",
        result
    );
    assert_eq!(result.unwrap().len(), 8);
}

#[test]
fn validate_output_specs_unsorted_input() {
    // Specs given out of order (1,0) → should sort by index and succeed
    use doli_core::OutputType;
    let hash = crypto::Hash::from_bytes([0xaa; 32]);
    let specs = vec![
        (1u8, OutputType::Normal, hash, 200u64),
        (0u8, OutputType::Normal, hash, 100u64),
    ];
    let result = super::validate_output_specs(&specs);
    assert!(
        result.is_ok(),
        "Expected Ok for unsorted input, got {:?}",
        result
    );
    let outputs = result.unwrap();
    assert_eq!(outputs[0].amount, 100); // index 0 first
    assert_eq!(outputs[1].amount, 200); // index 1 second
}

// --- should_warn_high_fee tests ---

#[test]
fn high_fee_above_both_thresholds_warns() {
    // INPUT PARTITION P11: fee > 1% of input AND > 10000 units → warn
    // input = 1_000_000, output = 0, fee = 1_000_000
    // 1% of input = 10_000; fee(1_000_000) > max(10_000, 10_000) = 10_000 → warn
    assert!(super::should_warn_high_fee(1_000_000, 0));
}

#[test]
fn fee_within_one_percent_no_warning() {
    // INPUT PARTITION P12: fee <= 1% of input → no warning
    // input = 10_000_000, output = 9_910_000, fee = 90_000
    // 1% of input = 100_000; fee(90_000) <= max(100_000, 10_000) = 100_000 → no warn
    assert!(!super::should_warn_high_fee(10_000_000, 9_910_000));
}

#[test]
fn fee_above_10000_but_within_one_percent() {
    // INPUT PARTITION P13: fee > 10000 but <= 1% of input → no warning
    // input = 100_000_000, output = 99_500_000, fee = 500_000
    // 1% of input = 1_000_000; fee(500_000) <= max(1_000_000, 10_000) → no warn
    assert!(!super::should_warn_high_fee(100_000_000, 99_500_000));
}

#[test]
fn fee_exactly_at_threshold_no_warning() {
    // fee exactly equals threshold → no warning (threshold is >, not >=)
    // input = 1_000_000, output = 990_000, fee = 10_000
    // 1% of input = 10_000; fee(10_000) == max(10_000, 10_000) → no warn (not >)
    assert!(!super::should_warn_high_fee(1_000_000, 990_000));
}

#[test]
fn fee_zero_no_warning() {
    // Zero fee → no warning
    assert!(!super::should_warn_high_fee(1_000_000, 1_000_000));
}

#[test]
fn fee_small_input_uses_10000_floor() {
    // Small input where 1% < 10000 → uses 10000 floor
    // input = 100_000, output = 80_000, fee = 20_000
    // 1% of input = 1_000; max(1_000, 10_000) = 10_000; fee(20_000) > 10_000 → warn
    assert!(super::should_warn_high_fee(100_000, 80_000));
}

#[test]
fn fee_small_input_below_floor() {
    // Small input, fee below 10000 floor → no warning
    // input = 100_000, output = 91_000, fee = 9_000
    // 1% = 1_000; max(1_000, 10_000) = 10_000; fee(9_000) <= 10_000 → no warn
    assert!(!super::should_warn_high_fee(100_000, 91_000));
}

// =============================================================================
// TEST-099 — Auto fee matches the node's size-scaled minimum_fee (INC-I-099)
// =============================================================================
//
// OUTPUT CONTRACT: fn auto_fee_for_outputs(outputs: &[Output]) -> u64
//   O1: returned fee (u64)
//   PATHS:
//     P1: all outputs have empty extra_data (plain transfer)
//     P2: an output has extra_data of exactly FEE_DIVISOR bytes (boundary → +1)
//     P3: an output has extra_data ≥ FEE_DIVISOR bytes (covenant, e.g. escrow)
//   MATRIX:
//     P1 → BASE_FEE (1)                              [REQ-099-002 regression guard]
//     P2 (100 bytes) → 1 + 100/100 = 2               [REQ-099-001 the "1 < 2" repro]
//     P3 (250 bytes) → 1 + 250/100 = 3
//     ALL → equals Transaction::new_transfer(vec![], outputs).minimum_fee() (node parity)

#[test]
fn auto_fee_plain_transfer_is_base_fee() {
    // REQ-099-002: plain transfers (0 extra_data) keep the flat base fee — no regression.
    use doli_core::Output;
    let hash = crypto::Hash::from_bytes([0x11; 32]);
    let outputs = vec![Output::normal(1_000, hash), Output::normal(50, hash)];
    assert_eq!(
        super::auto_fee_for_outputs(&outputs),
        doli_core::consensus::BASE_FEE
    );
    assert_eq!(super::auto_fee_for_outputs(&outputs), 1);
}

#[test]
fn auto_fee_covenant_output_meets_node_minimum() {
    // REQ-099-001: the exact INC-I-099 repro. A covenant output with ≥100 extra_data
    // bytes requires fee 2 — the old flat-1 auto fee produced "FEE_TOO_LOW: 1 < 2".
    use doli_core::{Output, OutputType, Transaction};
    let hash = crypto::Hash::from_bytes([0x22; 32]);
    let recipient = Output {
        output_type: OutputType::Multisig,
        amount: 1_000,
        pubkey_hash: hash,
        lock_until: 0,
        extra_data: vec![0u8; 100], // escrow-class condition size → byte_fee = 1
    };
    let change = Output::normal(25, hash); // change has empty extra_data, must not affect fee
    let outputs = vec![recipient, change];

    let fee = super::auto_fee_for_outputs(&outputs);
    assert_eq!(fee, 2, "100-byte extra_data → 1 + 100/100 = 2");
    // Node parity: must equal what the mempool enforces.
    let node_min = Transaction::new_transfer(Vec::new(), outputs).minimum_fee();
    assert_eq!(fee, node_min, "auto fee must equal node minimum_fee()");
}

#[test]
fn auto_fee_scales_with_extra_data() {
    // P3: 250 bytes → 1 + 250/100 = 3 (integer division).
    use doli_core::{Output, OutputType, Transaction};
    let hash = crypto::Hash::from_bytes([0x33; 32]);
    let recipient = Output {
        output_type: OutputType::Hashlock,
        amount: 1_000,
        pubkey_hash: hash,
        lock_until: 0,
        extra_data: vec![0u8; 250],
    };
    let outputs = vec![recipient];
    assert_eq!(super::auto_fee_for_outputs(&outputs), 3);
    assert_eq!(
        super::auto_fee_for_outputs(&outputs),
        Transaction::new_transfer(Vec::new(), outputs).minimum_fee()
    );
}
