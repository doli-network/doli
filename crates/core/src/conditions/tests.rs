//! Tests for the conditions module.
//!
//! ## OUTPUT CONTRACT — MaxDeltaGuard
//!
//! **Observable outputs:**
//! - `evaluate()` returns `bool` (true = guard passes, false = guard rejects)
//! - `encode()` returns `Result<Vec<u8>, ConditionError>` (serialized bytes)
//! - `decode()` returns `Result<Condition, ConditionError>` (deserialized condition)
//! - `ops_count()` returns `usize` (0 for all guards — no crypto ops)
//! - `contains_guard()` returns `bool` (true for all guard types)
//! - `validate()` returns `Result<(), ConditionError>`
//!
//! **Code paths (evaluate):**
//! - P1: No transaction context → false
//! - P2: Output index out of bounds → false
//! - P3: reference_amount == 0 (division by zero) → false
//! - P4: delta_bps > max_change_bps → false (reject)
//! - P5: delta_bps <= max_change_bps → true (pass)
//! - P6: delta_bps == max_change_bps (boundary) → true (pass, strictly-greater rejects)
//!
//! ## INPUT PARTITIONS — MaxDeltaGuard evaluate
//!
//! | Partition | Path | Test |
//! |-----------|------|------|
//! | tx=None | P1 | max_delta_guard_no_transaction |
//! | output_index >= outputs.len() | P2 | max_delta_guard_out_of_bounds_index |
//! | reference_amount=0 | P3 | max_delta_guard_zero_reference_amount |
//! | output=10200, ref=10000, bps=100 (2%>1%) | P4 | max_delta_guard_rejects_above_threshold |
//! | output=10050, ref=10000, bps=100 (0.5%<=1%) | P5 | max_delta_guard_allows_within_threshold |
//! | output=10100, ref=10000, bps=100 (1%==1%) | P6 | max_delta_guard_exact_threshold_boundary |
//! | output=MAX/2+1, ref=MAX/2 (overflow edge) | P5 | max_delta_guard_overflow_resistance |
//! | output=MAX, ref=MAX, bps=0 (delta=0) | P5 | max_delta_guard_large_values_no_panic |
//!
//! ## OUTPUT CONTRACT — ReserveRatioGuard
//!
//! **Observable outputs:**
//! - `evaluate()` returns `bool`
//! - `encode()`/`decode()`: round-trip preserves identity
//! - `ops_count()` → 0, `contains_guard()` → true
//!
//! **Code paths (evaluate):**
//! - P1: No transaction context → false
//! - P2: reserve or debt output index out of bounds → false
//! - P3: debt_amount == 0 → false (cannot compute ratio)
//! - P4: ratio_bps < min_ratio_bps → false (reject)
//! - P5: ratio_bps >= min_ratio_bps → true (pass)
//!
//! ## INPUT PARTITIONS — ReserveRatioGuard evaluate
//!
//! | Partition | Path | Test |
//! |-----------|------|------|
//! | tx=None | P1 | reserve_ratio_no_transaction |
//! | debt_output_index OOB | P2 | reserve_ratio_out_of_bounds_index |
//! | debt=0 | P3 | reserve_ratio_zero_debt_rejects |
//! | reserve=100, debt=100, min=15000 (100%<150%) | P4 | reserve_ratio_rejects_below_min |
//! | reserve=200, debt=100, min=15000 (200%>150%) | P5 | reserve_ratio_allows_above_min |
//! | reserve=150, debt=100, min=15000 (150%==150%) | P5 | reserve_ratio_exact_boundary_passes |
//! | reserve=MAX, debt=1 (u128 overflow edge) | P5 | reserve_ratio_u128_internal_no_overflow |

use crypto::hash::hash_with_domain;
use crypto::Hash;

use super::*;

fn dummy_hash(byte: u8) -> Hash {
    Hash::from_bytes([byte; 32])
}

// ---- Encoding / Decoding roundtrips ----

#[test]
fn test_signature_roundtrip() {
    let cond = Condition::signature(dummy_hash(0xAA));
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
    assert_eq!(encoded.len(), 1 + 1 + 32); // version + tag + hash
}

#[test]
fn test_multisig_roundtrip() {
    let cond = Condition::multisig(2, vec![dummy_hash(1), dummy_hash(2), dummy_hash(3)]);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
    assert_eq!(encoded.len(), 1 + 1 + 2 + 3 * 32); // version + tag + params + keys
}

#[test]
fn test_hashlock_roundtrip() {
    let cond = Condition::hashlock(dummy_hash(0xBB));
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

#[test]
fn test_timelock_roundtrip() {
    let cond = Condition::timelock(50_000);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

#[test]
fn test_timelock_expiry_roundtrip() {
    let cond = Condition::timelock_expiry(100_000);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

#[test]
fn test_and_roundtrip() {
    let cond = Condition::And(
        Box::new(Condition::signature(dummy_hash(1))),
        Box::new(Condition::timelock(1000)),
    );
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

#[test]
fn test_or_roundtrip() {
    let cond = Condition::Or(
        Box::new(Condition::hashlock(dummy_hash(0xCC))),
        Box::new(Condition::timelock_expiry(5000)),
    );
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

#[test]
fn test_htlc_roundtrip() {
    let cond = Condition::htlc(dummy_hash(0xDD), 1000, 2000);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

#[test]
fn test_threshold_roundtrip() {
    let cond = Condition::Threshold {
        n: 2,
        conditions: vec![
            Condition::signature(dummy_hash(1)),
            Condition::signature(dummy_hash(2)),
            Condition::signature(dummy_hash(3)),
        ],
    };
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

#[test]
fn test_vesting_roundtrip() {
    let cond = Condition::vesting(dummy_hash(0xEE), 50_000);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
}

// ---- Validation errors ----

#[test]
fn test_multisig_threshold_exceeds_keys() {
    let cond = Condition::Multisig {
        threshold: 3,
        keys: vec![dummy_hash(1), dummy_hash(2)],
    };
    assert!(matches!(
        cond.encode(),
        Err(ConditionError::InvalidThreshold { .. })
    ));
}

#[test]
fn test_multisig_zero_threshold() {
    let cond = Condition::Multisig {
        threshold: 0,
        keys: vec![dummy_hash(1)],
    };
    assert!(matches!(cond.encode(), Err(ConditionError::ZeroThreshold)));
}

#[test]
fn test_too_deep() {
    // Build depth-5 nesting: And(And(And(And(And(sig, sig), sig), sig), sig)
    let mut cond = Condition::signature(dummy_hash(1));
    for _ in 0..5 {
        cond = Condition::And(
            Box::new(cond),
            Box::new(Condition::signature(dummy_hash(2))),
        );
    }
    assert!(matches!(cond.encode(), Err(ConditionError::TooDeep { .. })));
}

#[test]
fn test_max_depth_ok() {
    // depth=4 should be fine
    let mut cond = Condition::signature(dummy_hash(1));
    for _ in 0..4 {
        cond = Condition::And(
            Box::new(cond),
            Box::new(Condition::signature(dummy_hash(2))),
        );
    }
    assert!(cond.encode().is_ok());
}

#[test]
fn test_decode_bad_version() {
    let bytes = vec![0xFF, TAG_SIGNATURE];
    assert!(matches!(
        Condition::decode(&bytes),
        Err(ConditionError::UnsupportedVersion { version: 0xFF })
    ));
}

#[test]
fn test_decode_unknown_tag() {
    let bytes = vec![CONDITION_VERSION, 0xFF];
    assert!(matches!(
        Condition::decode(&bytes),
        Err(ConditionError::UnknownTag { tag: 0xFF })
    ));
}

#[test]
fn test_decode_truncated() {
    let bytes = vec![CONDITION_VERSION, TAG_SIGNATURE, 0x00]; // only 1 byte of 32
    assert!(matches!(
        Condition::decode(&bytes),
        Err(ConditionError::BufferTooShort)
    ));
}

#[test]
fn test_trailing_bytes_rejected() {
    let cond = Condition::timelock(100);
    let mut encoded = cond.encode().unwrap();
    encoded.push(0xFF); // garbage
    assert!(matches!(
        Condition::decode(&encoded),
        Err(ConditionError::TrailingBytes { .. })
    ));
}

// ---- Ops count ----

#[test]
fn test_ops_count() {
    assert_eq!(Condition::signature(dummy_hash(1)).ops_count(), 1);
    assert_eq!(Condition::timelock(100).ops_count(), 0);
    assert_eq!(
        Condition::multisig(2, vec![dummy_hash(1), dummy_hash(2), dummy_hash(3)]).ops_count(),
        3
    );
    let htlc = Condition::htlc(dummy_hash(1), 100, 200);
    assert_eq!(htlc.ops_count(), 1); // hashlock=1, timelocks=0
}

// ---- Witness encoding/decoding ----

#[test]
fn test_witness_empty_roundtrip() {
    let w = Witness::default();
    let encoded = w.encode();
    let decoded = Witness::decode(&encoded).unwrap();
    assert!(decoded.signatures.is_empty());
    assert!(decoded.preimage.is_none());
    assert!(decoded.or_branches.is_empty());
}

#[test]
fn test_witness_preimage_roundtrip() {
    let w = Witness {
        preimage: Some([0xAB; 32]),
        ..Default::default()
    };
    let encoded = w.encode();
    let decoded = Witness::decode(&encoded).unwrap();
    assert_eq!(decoded.preimage, Some([0xAB; 32]));
}

#[test]
fn test_witness_or_branches_roundtrip() {
    let w = Witness {
        or_branches: vec![true, false, true],
        ..Default::default()
    };
    let encoded = w.encode();
    let decoded = Witness::decode(&encoded).unwrap();
    assert_eq!(decoded.or_branches, vec![true, false, true]);
}

// ---- Evaluation ----

#[test]
fn test_eval_timelock_satisfied() {
    let cond = Condition::timelock(100);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_timelock_not_satisfied() {
    let cond = Condition::timelock(100);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 99,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_timelock_expiry_satisfied() {
    let cond = Condition::timelock_expiry(100);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_timelock_expiry_not_satisfied() {
    // TimelockExpiry(100) means spendable at height >= 100
    // At height 99, should NOT be satisfied
    let cond = Condition::timelock_expiry(100);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 99,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_hashlock() {
    let preimage = [0x42u8; 32];
    let expected = hash_with_domain(HASHLOCK_DOMAIN, &preimage);
    let cond = Condition::Hashlock(expected);

    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 0,
        signing_hash: &hash,
        transaction: None,
    };

    // With correct preimage
    let w = Witness {
        preimage: Some(preimage),
        ..Default::default()
    };
    assert!(evaluate(&cond, &w, &ctx, &mut 0));

    // With wrong preimage
    let w_bad = Witness {
        preimage: Some([0x43u8; 32]),
        ..Default::default()
    };
    assert!(!evaluate(&cond, &w_bad, &ctx, &mut 0));

    // With no preimage
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_and() {
    // And(Timelock(50), TimelockExpiry(200)):
    // Timelock(50): height >= 50
    // TimelockExpiry(200): height >= 200
    // Combined: height >= 200 (both must be satisfied)
    let cond = Condition::And(
        Box::new(Condition::timelock(50)),
        Box::new(Condition::timelock_expiry(200)),
    );
    let hash = dummy_hash(0);

    // Height 30: timelock not met
    let ctx = EvalContext {
        current_height: 30,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 100: timelock met but expiry not yet reached
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 200: both satisfied
    let ctx = EvalContext {
        current_height: 200,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 300: both satisfied
    let ctx = EvalContext {
        current_height: 300,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_or_with_branch_hint() {
    let cond = Condition::Or(
        Box::new(Condition::timelock(1000)), // left: not met at h=50
        Box::new(Condition::timelock(10)),   // right: met at h=50
    );
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 50,
        signing_hash: &hash,
        transaction: None,
    };

    // Branch hint: right (true)
    let w = Witness {
        or_branches: vec![true],
        ..Default::default()
    };
    assert!(evaluate(&cond, &w, &ctx, &mut 0));

    // Branch hint: left (false) — will fail
    let w = Witness {
        or_branches: vec![false],
        ..Default::default()
    };
    assert!(!evaluate(&cond, &w, &ctx, &mut 0));
}

#[test]
fn test_eval_or_without_hint_tries_both() {
    let cond = Condition::Or(
        Box::new(Condition::timelock(1000)), // left: not met
        Box::new(Condition::timelock(10)),   // right: met
    );
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 50,
        signing_hash: &hash,
        transaction: None,
    };

    // No branch hints — should try left (fail) then right (succeed)
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_threshold() {
    let cond = Condition::Threshold {
        n: 2,
        conditions: vec![
            Condition::timelock(100), // met at h=150
            Condition::timelock(200), // not met at h=150
            Condition::timelock(50),  // met at h=150
        ],
    };
    let hash = dummy_hash(0);

    // Height 150: conditions[0] and conditions[2] satisfied → 2 >= 2
    let ctx = EvalContext {
        current_height: 150,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 80: only conditions[2] satisfied → 1 < 2
    let ctx = EvalContext {
        current_height: 80,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

// ---- Multisig validation invariants ----

#[test]
fn test_multisig_validate_errors() {
    // Threshold > keys
    let cond = Condition::Multisig {
        threshold: 4,
        keys: vec![dummy_hash(1), dummy_hash(2)],
    };
    assert!(cond.validate().is_err());

    // Zero threshold
    let cond = Condition::Multisig {
        threshold: 0,
        keys: vec![dummy_hash(1)],
    };
    assert!(cond.validate().is_err());
}

// ---- Threshold validation invariants ----

#[test]
fn test_threshold_validate_errors() {
    // n > count
    let cond = Condition::Threshold {
        n: 3,
        conditions: vec![Condition::timelock(100), Condition::timelock(200)],
    };
    assert!(cond.validate().is_err());

    // Zero n
    let cond = Condition::Threshold {
        n: 0,
        conditions: vec![Condition::timelock(100)],
    };
    assert!(cond.validate().is_err());
}

// ---- Encoding size checks ----

#[test]
fn test_max_multisig_fits() {
    // 127 keys = 1 (version) + 1 (tag) + 2 (params) + 127*32 (keys) = 4068 bytes < 4096
    let keys: Vec<Hash> = (0..127).map(|i| dummy_hash(i as u8)).collect();
    let cond = Condition::multisig(64, keys);
    let encoded = cond.encode().unwrap();
    assert!(encoded.len() <= 4096);
}

#[test]
fn test_htlc_encoding_size() {
    let cond = Condition::htlc(dummy_hash(0xDD), 1000, 2000);
    let encoded = cond.encode().unwrap();
    // version(1) + Or(1) + And(1) + Hashlock(1+32) + Timelock(1+8) + TimelockExpiry(1+8) = 54
    assert!(encoded.len() < 64);
}

// ====================================================================
// Integration tests with real crypto signatures
// ====================================================================

fn keypair_pubkey_hash(kp: &crypto::KeyPair) -> Hash {
    hash_with_domain(ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

#[test]
fn integration_signature_condition() {
    let kp = crypto::KeyPair::generate();
    let pkh = keypair_pubkey_hash(&kp);

    let cond = Condition::signature(pkh);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);

    // Build a witness with a real signature
    let tx_hash = Hash::from_bytes([0x42; 32]);
    let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());

    let witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig,
        }],
        ..Default::default()
    };

    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &tx_hash,
        transaction: None,
    };

    let mut branch_idx = 0;
    assert!(evaluate(&cond, &witness, &ctx, &mut branch_idx));

    // Wrong signing hash should fail
    let wrong_hash = Hash::from_bytes([0x99; 32]);
    let ctx_wrong = EvalContext {
        current_height: 100,
        signing_hash: &wrong_hash,
        transaction: None,
    };
    let mut branch_idx = 0;
    assert!(!evaluate(&cond, &witness, &ctx_wrong, &mut branch_idx));
}

#[test]
fn integration_multisig_2_of_3() {
    let kp1 = crypto::KeyPair::generate();
    let kp2 = crypto::KeyPair::generate();
    let kp3 = crypto::KeyPair::generate();

    let pkh1 = keypair_pubkey_hash(&kp1);
    let pkh2 = keypair_pubkey_hash(&kp2);
    let pkh3 = keypair_pubkey_hash(&kp3);

    let cond = Condition::multisig(2, vec![pkh1, pkh2, pkh3]);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);

    let tx_hash = Hash::from_bytes([0xAB; 32]);
    let sig1 = crypto::signature::sign_hash(&tx_hash, kp1.private_key());
    let sig3 = crypto::signature::sign_hash(&tx_hash, kp3.private_key());

    // 2-of-3 with sigs from kp1 and kp3
    let witness = Witness {
        signatures: vec![
            WitnessSignature {
                pubkey: *kp1.public_key(),
                signature: sig1,
            },
            WitnessSignature {
                pubkey: *kp3.public_key(),
                signature: sig3,
            },
        ],
        ..Default::default()
    };

    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &witness, &ctx, &mut idx));

    // Only 1-of-3 should fail
    let witness_1 = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp2.public_key(),
            signature: crypto::signature::sign_hash(&tx_hash, kp2.private_key()),
        }],
        ..Default::default()
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &witness_1, &ctx, &mut idx));
}

#[test]
fn integration_hashlock_preimage() {
    let secret = [0x77u8; 32];
    let hash = hash_with_domain(HASHLOCK_DOMAIN, &secret);

    let cond = Condition::hashlock(hash);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);

    let tx_hash = Hash::from_bytes([0x00; 32]);
    let ctx = EvalContext {
        current_height: 1,
        signing_hash: &tx_hash,
        transaction: None,
    };

    // Correct preimage
    let witness = Witness {
        preimage: Some(secret),
        ..Default::default()
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &witness, &ctx, &mut idx));

    // Wrong preimage
    let bad_witness = Witness {
        preimage: Some([0x88u8; 32]),
        ..Default::default()
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &bad_witness, &ctx, &mut idx));

    // No preimage
    let empty_witness = Witness::default();
    let mut idx = 0;
    assert!(!evaluate(&cond, &empty_witness, &ctx, &mut idx));
}

#[test]
fn integration_htlc_claim_and_refund() {
    let secret = [0xCC; 32];
    let hash = hash_with_domain(HASHLOCK_DOMAIN, &secret);

    // Standard HTLC helper: Or(And(Hashlock, Timelock(100)), TimelockExpiry(200))
    // Left branch: receiver claims with preimage after lock_height
    // Right branch: refund window (anyone before expiry_height)
    let cond = Condition::htlc(hash, 100, 200);
    let encoded = cond.encode().unwrap();
    assert!(encoded.len() <= MAX_WITNESS_SIZE);

    let tx_hash = Hash::from_bytes([0xDD; 32]);

    // Claim path: preimage + height >= 100, branch(left)
    let claim_witness = Witness {
        preimage: Some(secret),
        or_branches: vec![false], // left branch (hashlock path)
        ..Default::default()
    };
    let ctx_after_lock = EvalContext {
        current_height: 150,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &claim_witness, &ctx_after_lock, &mut idx));

    // Claim path fails before timelock
    let ctx_before_lock = EvalContext {
        current_height: 50,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &claim_witness, &ctx_before_lock, &mut idx));

    // Refund path: right branch, TimelockExpiry(200) means height >= 200
    let refund_witness = Witness {
        or_branches: vec![true], // right branch
        ..Default::default()
    };

    // Refund fails before expiry (height 150 < 200)
    let ctx_before_expiry = EvalContext {
        current_height: 150,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(!evaluate(
        &cond,
        &refund_witness,
        &ctx_before_expiry,
        &mut idx
    ));

    // Refund succeeds after expiry (height 250 >= 200)
    let ctx_after_expiry = EvalContext {
        current_height: 250,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(evaluate(
        &cond,
        &refund_witness,
        &ctx_after_expiry,
        &mut idx
    ));

    // Full HTLC with sender signature for refund:
    // Or(And(Hashlock, Timelock(100)), And(Signature(sender), Timelock(200)))
    let kp_sender = crypto::KeyPair::generate();
    let sender_pkh = keypair_pubkey_hash(&kp_sender);
    let cond_with_sig_refund = Condition::Or(
        Box::new(Condition::And(
            Box::new(Condition::Hashlock(hash)),
            Box::new(Condition::Timelock(100)),
        )),
        Box::new(Condition::And(
            Box::new(Condition::Signature(sender_pkh)),
            Box::new(Condition::Timelock(200)),
        )),
    );

    let sig = crypto::signature::sign_hash(&tx_hash, kp_sender.private_key());
    let sig_refund_witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp_sender.public_key(),
            signature: sig,
        }],
        or_branches: vec![true], // right branch (refund)
        ..Default::default()
    };
    let ctx_after_refund_lock = EvalContext {
        current_height: 250,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(evaluate(
        &cond_with_sig_refund,
        &sig_refund_witness,
        &ctx_after_refund_lock,
        &mut idx
    ));

    // Sender refund fails before refund timelock
    let ctx_too_early = EvalContext {
        current_height: 150,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(!evaluate(
        &cond_with_sig_refund,
        &sig_refund_witness,
        &ctx_too_early,
        &mut idx
    ));
}

#[test]
fn integration_vesting_schedule() {
    let kp = crypto::KeyPair::generate();
    let pkh = keypair_pubkey_hash(&kp);

    // Vesting: And(Signature(owner), Timelock(1000))
    let cond = Condition::vesting(pkh, 1000);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);

    let tx_hash = Hash::from_bytes([0xEE; 32]);
    let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());

    let witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig,
        }],
        ..Default::default()
    };

    // Before vesting period
    let ctx_early = EvalContext {
        current_height: 500,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &witness, &ctx_early, &mut idx));

    // After vesting period
    let ctx_vested = EvalContext {
        current_height: 1000,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &witness, &ctx_vested, &mut idx));
}

#[test]
fn integration_witness_encode_decode_roundtrip() {
    let kp1 = crypto::KeyPair::generate();
    let kp2 = crypto::KeyPair::generate();
    let tx_hash = Hash::from_bytes([0xFF; 32]);

    let sig1 = crypto::signature::sign_hash(&tx_hash, kp1.private_key());
    let sig2 = crypto::signature::sign_hash(&tx_hash, kp2.private_key());

    let witness = Witness {
        signatures: vec![
            WitnessSignature {
                pubkey: *kp1.public_key(),
                signature: sig1,
            },
            WitnessSignature {
                pubkey: *kp2.public_key(),
                signature: sig2,
            },
        ],
        preimage: Some([0xAA; 32]),
        or_branches: vec![true, false, true],
    };

    let encoded = witness.encode();
    let decoded = Witness::decode(&encoded).unwrap();

    assert_eq!(witness.signatures.len(), decoded.signatures.len());
    assert_eq!(witness.preimage, decoded.preimage);
    assert_eq!(witness.or_branches, decoded.or_branches);

    // Verify signatures still valid after roundtrip
    for (orig, dec) in witness.signatures.iter().zip(decoded.signatures.iter()) {
        assert_eq!(orig.pubkey, dec.pubkey);
        assert_eq!(orig.signature, dec.signature);
    }
}

#[test]
fn integration_threshold_2_of_3_mixed() {
    // Threshold(2, [Signature(A), Hashlock(H), Timelock(50)])
    let kp = crypto::KeyPair::generate();
    let pkh = keypair_pubkey_hash(&kp);
    let secret = [0x55; 32];
    let hash = hash_with_domain(HASHLOCK_DOMAIN, &secret);

    let cond = Condition::Threshold {
        n: 2,
        conditions: vec![
            Condition::Signature(pkh),
            Condition::Hashlock(hash),
            Condition::Timelock(50),
        ],
    };
    assert!(cond.validate().is_ok());

    let tx_hash = Hash::from_bytes([0x11; 32]);
    let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());

    // Satisfy signature + hashlock (2 of 3)
    let witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig,
        }],
        preimage: Some(secret),
        ..Default::default()
    };
    let ctx = EvalContext {
        current_height: 10, // below timelock
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &witness, &ctx, &mut idx));

    // Satisfy signature + timelock (2 of 3)
    let witness_no_preimage = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig,
        }],
        ..Default::default()
    };
    let ctx_high = EvalContext {
        current_height: 100,
        signing_hash: &tx_hash,
        transaction: None,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &witness_no_preimage, &ctx_high, &mut idx));

    // Only timelock satisfied (1 of 3) — fail
    let witness_empty = Witness::default();
    let mut idx = 0;
    assert!(!evaluate(&cond, &witness_empty, &ctx_high, &mut idx));
}

// ====================================================================
// Guard condition tests
// ====================================================================

use crate::transaction::{Output, OutputType, Transaction, TxType};

/// Helper: build a minimal Transaction with the given outputs.
fn tx_with_outputs(outputs: Vec<Output>) -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![],
        outputs,
        extra_data: vec![],
    }
}

/// Helper: build a Normal output with given amount and pubkey_hash.
fn normal_output(amount: u64, pkh: Hash) -> Output {
    Output {
        output_type: OutputType::Normal,
        amount,
        pubkey_hash: pkh,
        lock_until: 0,
        extra_data: vec![],
    }
}

// ---- AmountGuard roundtrip ----

#[test]
fn test_amount_guard_roundtrip() {
    let cond = Condition::amount_guard(1_000_000, 0);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
    assert_eq!(encoded.len(), 1 + 1 + 8 + 1); // version + tag + amount + index
}

#[test]
fn test_output_type_guard_roundtrip() {
    let cond = Condition::output_type_guard(OutputType::Bond, 2);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
    assert_eq!(encoded.len(), 1 + 1 + 1 + 1); // version + tag + type + index
}

#[test]
fn test_recipient_guard_roundtrip() {
    let cond = Condition::recipient_guard(dummy_hash(0xAA), 1);
    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);
    assert_eq!(encoded.len(), 1 + 1 + 32 + 1); // version + tag + hash + index
}

// ---- AmountGuard evaluation ----

#[test]
fn test_eval_amount_guard_satisfied() {
    let cond = Condition::amount_guard(500, 0);
    let tx = tx_with_outputs(vec![normal_output(500, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_amount_guard_exceeded() {
    let cond = Condition::amount_guard(500, 0);
    let tx = tx_with_outputs(vec![normal_output(1000, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_amount_guard_insufficient() {
    let cond = Condition::amount_guard(500, 0);
    let tx = tx_with_outputs(vec![normal_output(499, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_amount_guard_out_of_bounds() {
    let cond = Condition::amount_guard(500, 5); // index 5 doesn't exist
    let tx = tx_with_outputs(vec![normal_output(1000, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_amount_guard_no_transaction() {
    let cond = Condition::amount_guard(500, 0);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

// ---- OutputTypeGuard evaluation ----

#[test]
fn test_eval_output_type_guard_satisfied() {
    let cond = Condition::output_type_guard(OutputType::Normal, 0);
    let tx = tx_with_outputs(vec![normal_output(100, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_output_type_guard_wrong_type() {
    let cond = Condition::output_type_guard(OutputType::Bond, 0);
    let tx = tx_with_outputs(vec![normal_output(100, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_output_type_guard_out_of_bounds() {
    let cond = Condition::output_type_guard(OutputType::Normal, 10);
    let tx = tx_with_outputs(vec![normal_output(100, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

// ---- RecipientGuard evaluation ----

#[test]
fn test_eval_recipient_guard_satisfied() {
    let recipient = dummy_hash(0xBB);
    let cond = Condition::recipient_guard(recipient, 0);
    let tx = tx_with_outputs(vec![normal_output(100, recipient)]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_recipient_guard_wrong_recipient() {
    let expected = dummy_hash(0xBB);
    let actual = dummy_hash(0xCC);
    let cond = Condition::recipient_guard(expected, 0);
    let tx = tx_with_outputs(vec![normal_output(100, actual)]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_eval_recipient_guard_out_of_bounds() {
    let cond = Condition::recipient_guard(dummy_hash(0xBB), 3);
    let tx = tx_with_outputs(vec![normal_output(100, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

// ---- Guard composition ----

#[test]
fn test_guard_and_composition() {
    // AmountGuard(500, 0) AND RecipientGuard(pkh, 0)
    let recipient = dummy_hash(0xBB);
    let cond = Condition::And(
        Box::new(Condition::amount_guard(500, 0)),
        Box::new(Condition::recipient_guard(recipient, 0)),
    );

    let encoded = cond.encode().unwrap();
    let decoded = Condition::decode(&encoded).unwrap();
    assert_eq!(cond, decoded);

    // Both satisfied
    let tx = tx_with_outputs(vec![normal_output(500, recipient)]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Amount insufficient
    let tx_low = tx_with_outputs(vec![normal_output(499, recipient)]);
    let ctx_low = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx_low),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx_low, &mut 0));

    // Wrong recipient
    let tx_wrong = tx_with_outputs(vec![normal_output(500, dummy_hash(0xCC))]);
    let ctx_wrong = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx_wrong),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx_wrong, &mut 0));
}

#[test]
fn test_limit_order_pattern() {
    // Limit order: Signature(owner) AND AmountGuard(min_price, 1)
    // "My UTXO can only be spent if I sign AND output[1] pays at least min_price"
    let recipient = dummy_hash(0xBB);
    let cond = Condition::And(
        Box::new(Condition::amount_guard(1_000_000, 1)),
        Box::new(Condition::recipient_guard(recipient, 1)),
    );

    let tx = tx_with_outputs(vec![
        normal_output(0, dummy_hash(0xAA)),  // output[0]: change
        normal_output(1_000_000, recipient), // output[1]: payment to seller
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn test_guards_with_multiple_output_indices() {
    // Guard output[0] is Normal AND output[1] pays at least 100
    let cond = Condition::And(
        Box::new(Condition::output_type_guard(OutputType::Normal, 0)),
        Box::new(Condition::amount_guard(100, 1)),
    );

    let tx = tx_with_outputs(vec![
        normal_output(50, dummy_hash(1)),
        normal_output(200, dummy_hash(2)),
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

// ---- contains_guard ----

#[test]
fn test_contains_guard() {
    assert!(!Condition::timelock(100).contains_guard());
    assert!(!Condition::signature(dummy_hash(1)).contains_guard());
    assert!(Condition::amount_guard(100, 0).contains_guard());
    assert!(Condition::output_type_guard(OutputType::Normal, 0).contains_guard());
    assert!(Condition::recipient_guard(dummy_hash(1), 0).contains_guard());

    // Nested in And
    let nested = Condition::And(
        Box::new(Condition::timelock(100)),
        Box::new(Condition::amount_guard(500, 0)),
    );
    assert!(nested.contains_guard());

    // No guard in And
    let no_guard = Condition::And(
        Box::new(Condition::timelock(100)),
        Box::new(Condition::signature(dummy_hash(1))),
    );
    assert!(!no_guard.contains_guard());
}

// ---- Guard ops count is zero (no crypto ops) ----

#[test]
fn test_guard_ops_count() {
    assert_eq!(Condition::amount_guard(100, 0).ops_count(), 0);
    assert_eq!(
        Condition::output_type_guard(OutputType::Normal, 0).ops_count(),
        0
    );
    assert_eq!(Condition::recipient_guard(dummy_hash(1), 0).ops_count(), 0);
}

// ====================================================================
// MaxDeltaGuard tests (M2 DeFi Foundations)
// ====================================================================

#[test]
fn max_delta_guard_rejects_above_threshold() {
    // Guard: max_change_bps=100 (1%), reference=10000, output_index=0
    // Output amount=10200 -> delta=200, 200*10000/10000 = 200 bps (2%) > 100 -> REJECT
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 10_000,
        output_index: 0,
    };
    let tx = tx_with_outputs(vec![normal_output(10_200, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn max_delta_guard_allows_within_threshold() {
    // Guard: max_change_bps=100 (1%), reference=10000, output_index=0
    // Output amount=10050 -> delta=50, 50*10000/10000 = 50 bps (0.5%) <= 100 -> PASS
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 10_000,
        output_index: 0,
    };
    let tx = tx_with_outputs(vec![normal_output(10_050, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn max_delta_guard_exact_threshold_boundary() {
    // Guard: max_change_bps=100 (1%), reference=10000, output_index=0
    // Output amount=10100 -> delta=100, 100*10000/10000 = 100 bps -> exactly at threshold
    // Policy: PASS at exact boundary (strictly greater rejects)
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 10_000,
        output_index: 0,
    };
    let tx = tx_with_outputs(vec![normal_output(10_100, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn max_delta_guard_zero_reference_amount() {
    // reference_amount=0 -> division by zero -> deterministic reject
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 0,
        output_index: 0,
    };
    let tx = tx_with_outputs(vec![normal_output(100, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn max_delta_guard_overflow_resistance() {
    // reference=u64::MAX/2, output=u64::MAX/2 + 1 -> delta=1
    // Must not panic, deterministic result.
    // delta(1) * 10000 / (u64::MAX/2) -> very small -> should pass
    let half_max = u64::MAX / 2;
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: half_max,
        output_index: 0,
    };
    let tx = tx_with_outputs(vec![normal_output(half_max + 1, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    // delta=1, 1*10000 / half_max = 0 bps -> PASS
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn max_delta_guard_large_values_no_panic() {
    // Both at u64::MAX -> delta=0, should pass
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 0,
        reference_amount: u64::MAX,
        output_index: 0,
    };
    let tx = tx_with_outputs(vec![normal_output(u64::MAX, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn max_delta_guard_no_transaction() {
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 10_000,
        output_index: 0,
    };
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn max_delta_guard_out_of_bounds_index() {
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 10_000,
        output_index: 5,
    };
    let tx = tx_with_outputs(vec![normal_output(10_000, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

// ====================================================================
// ReserveRatioGuard tests (M2 DeFi Foundations)
// ====================================================================

#[test]
fn reserve_ratio_rejects_below_min() {
    // min_ratio_bps=15000 (150%), reserve=100, debt=100 -> ratio=10000 bps (100%) < 15000 -> REJECT
    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let tx = tx_with_outputs(vec![
        normal_output(100, dummy_hash(1)),
        normal_output(100, dummy_hash(2)),
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn reserve_ratio_allows_above_min() {
    // min_ratio_bps=15000 (150%), reserve=200, debt=100 -> ratio=20000 bps (200%) >= 15000 -> PASS
    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let tx = tx_with_outputs(vec![
        normal_output(200, dummy_hash(1)),
        normal_output(100, dummy_hash(2)),
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn reserve_ratio_exact_boundary_passes() {
    // min_ratio_bps=15000, reserve=150, debt=100 -> 150*10000/100 = 15000 -> exactly at min -> PASS
    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let tx = tx_with_outputs(vec![
        normal_output(150, dummy_hash(1)),
        normal_output(100, dummy_hash(2)),
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn reserve_ratio_zero_debt_rejects() {
    // debt=0 -> cannot compute ratio -> deterministic reject
    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let tx = tx_with_outputs(vec![
        normal_output(200, dummy_hash(1)),
        normal_output(0, dummy_hash(2)),
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn reserve_ratio_u128_internal_no_overflow() {
    // reserve=u64::MAX, debt=1 -> ratio = u64::MAX * 10000 / 1
    // Must use u128 internally or this overflows. Should not panic.
    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 10_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let tx = tx_with_outputs(vec![
        normal_output(u64::MAX, dummy_hash(1)),
        normal_output(1, dummy_hash(2)),
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    // u64::MAX * 10000 / 1 via u128 -> huge ratio >> 10000 -> PASS
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn reserve_ratio_no_transaction() {
    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: None,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn reserve_ratio_out_of_bounds_index() {
    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 5, // out of bounds
    };
    let tx = tx_with_outputs(vec![normal_output(200, dummy_hash(1))]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

// ====================================================================
// Composition tests with new guards (M2 DeFi Foundations)
// ====================================================================

#[test]
fn max_delta_inside_and_short_circuits() {
    // And(Signature(pk), MaxDeltaGuard{max_change_bps=100, ref=10000, idx=0})
    // Without a valid signature, should fail even if delta is within range
    let kp = crypto::KeyPair::generate();
    let pkh = keypair_pubkey_hash(&kp);

    let cond = Condition::And(
        Box::new(Condition::Signature(pkh)),
        Box::new(Condition::MaxDeltaGuard {
            max_change_bps: 100,
            reference_amount: 10_000,
            output_index: 0,
        }),
    );

    let tx = tx_with_outputs(vec![normal_output(10_050, dummy_hash(1))]);
    let hash = dummy_hash(0);

    // No signature -> left side fails, short-circuits
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // With valid signature + delta within range -> both pass
    let tx_hash = Hash::from_bytes([0x42; 32]);
    let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());
    let witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig,
        }],
        ..Default::default()
    };
    let ctx_with_tx = EvalContext {
        current_height: 100,
        signing_hash: &tx_hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &witness, &ctx_with_tx, &mut 0));
}

#[test]
fn reserve_ratio_inside_or_one_branch_passes() {
    // Or(ReserveRatioGuard{strict: 20000}, ReserveRatioGuard{lax: 10000})
    // reserve=150, debt=100 -> ratio=15000
    // strict (20000) fails, lax (10000) passes
    let strict = Condition::ReserveRatioGuard {
        min_ratio_bps: 20_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let lax = Condition::ReserveRatioGuard {
        min_ratio_bps: 10_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    let cond = Condition::Or(Box::new(strict), Box::new(lax));

    let tx = tx_with_outputs(vec![
        normal_output(150, dummy_hash(1)),
        normal_output(100, dummy_hash(2)),
    ]);
    let hash = dummy_hash(0);
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
        transaction: Some(&tx),
    };
    // Without branch hints, tries left (fail) then right (pass)
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
}

#[test]
fn threshold_with_two_max_deltas() {
    // Threshold{n: 2, conditions: [Signature(pk), MaxDelta{1%, ref=1000, idx=0},
    //                               MaxDelta{5%, ref=1000, idx=0}]}
    // output=1020 -> delta=20, 20*10000/1000 = 200 bps
    // MaxDelta(1%=100 bps): 200 > 100 -> FAIL
    // MaxDelta(5%=500 bps): 200 <= 500 -> PASS
    // Need 2-of-3: Signature + MaxDelta(5%) = 2 -> PASS
    let kp = crypto::KeyPair::generate();
    let pkh = keypair_pubkey_hash(&kp);

    let cond = Condition::Threshold {
        n: 2,
        conditions: vec![
            Condition::Signature(pkh),
            Condition::MaxDeltaGuard {
                max_change_bps: 100,
                reference_amount: 1_000,
                output_index: 0,
            },
            Condition::MaxDeltaGuard {
                max_change_bps: 500,
                reference_amount: 1_000,
                output_index: 0,
            },
        ],
    };

    let tx = tx_with_outputs(vec![normal_output(1_020, dummy_hash(1))]);
    let tx_hash = Hash::from_bytes([0x33; 32]);
    let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());
    let witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig,
        }],
        ..Default::default()
    };
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &tx_hash,
        transaction: Some(&tx),
    };
    assert!(evaluate(&cond, &witness, &ctx, &mut 0));
}

// ====================================================================
// Encoding round-trip tests for new guards (M2 DeFi Foundations)
// ====================================================================

#[test]
fn max_delta_encoding_round_trip() {
    // Test several parameter combinations
    let cases: Vec<(u16, u64, u8)> = vec![
        (100, 10_000, 0),
        (0, 0, 0),
        (10_000, u64::MAX, 255),
        (5_000, 1, 3),
        (1, 999_999_999, 7),
    ];
    for (bps, ref_amt, idx) in cases {
        let cond = Condition::MaxDeltaGuard {
            max_change_bps: bps,
            reference_amount: ref_amt,
            output_index: idx,
        };
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(
            cond, decoded,
            "round-trip failed for bps={bps}, ref={ref_amt}, idx={idx}"
        );
    }
}

#[test]
fn reserve_ratio_encoding_round_trip() {
    // Test several parameter combinations
    let cases: Vec<(u16, u8, u8)> = vec![
        (15_000, 0, 1),
        (10_000, 2, 3),
        (1, 0, 0),
        (u16::MAX, 255, 254),
        (10_001, 5, 7),
    ];
    for (bps, res_idx, debt_idx) in cases {
        let cond = Condition::ReserveRatioGuard {
            min_ratio_bps: bps,
            reserve_output_index: res_idx,
            debt_output_index: debt_idx,
        };
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(
            cond, decoded,
            "round-trip failed for bps={bps}, res={res_idx}, debt={debt_idx}"
        );
    }
}

// ====================================================================
// New guards: ops_count, contains_guard, depth (M2 DeFi Foundations)
// ====================================================================

#[test]
fn new_guards_ops_count_is_zero() {
    assert_eq!(
        Condition::MaxDeltaGuard {
            max_change_bps: 100,
            reference_amount: 10_000,
            output_index: 0,
        }
        .ops_count(),
        0
    );
    assert_eq!(
        Condition::ReserveRatioGuard {
            min_ratio_bps: 15_000,
            reserve_output_index: 0,
            debt_output_index: 1,
        }
        .ops_count(),
        0
    );
}

#[test]
fn new_guards_are_guard_conditions() {
    assert!(Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 10_000,
        output_index: 0,
    }
    .contains_guard());
    assert!(Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    }
    .contains_guard());
}

#[test]
fn new_guards_depth_is_zero() {
    let cond = Condition::MaxDeltaGuard {
        max_change_bps: 100,
        reference_amount: 10_000,
        output_index: 0,
    };
    // Leaf condition -> validate should not complain about depth
    assert!(cond.validate().is_ok());

    let cond = Condition::ReserveRatioGuard {
        min_ratio_bps: 15_000,
        reserve_output_index: 0,
        debt_output_index: 1,
    };
    assert!(cond.validate().is_ok());
}
