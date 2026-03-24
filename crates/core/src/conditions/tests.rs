//! Tests for the conditions module.

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
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 100: timelock met but expiry not yet reached
    let ctx = EvalContext {
        current_height: 100,
        signing_hash: &hash,
    };
    assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 200: both satisfied
    let ctx = EvalContext {
        current_height: 200,
        signing_hash: &hash,
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 300: both satisfied
    let ctx = EvalContext {
        current_height: 300,
        signing_hash: &hash,
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
    };
    assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));

    // Height 80: only conditions[2] satisfied → 1 < 2
    let ctx = EvalContext {
        current_height: 80,
        signing_hash: &hash,
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
    // 15 keys = 1 (version) + 1 (tag) + 2 (params) + 15*32 (keys) = 484 bytes < 512
    let keys: Vec<Hash> = (0..15).map(dummy_hash).collect();
    let cond = Condition::multisig(10, keys);
    let encoded = cond.encode().unwrap();
    assert!(encoded.len() <= 512);
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
    };

    let mut branch_idx = 0;
    assert!(evaluate(&cond, &witness, &ctx, &mut branch_idx));

    // Wrong signing hash should fail
    let wrong_hash = Hash::from_bytes([0x99; 32]);
    let ctx_wrong = EvalContext {
        current_height: 100,
        signing_hash: &wrong_hash,
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
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &claim_witness, &ctx_after_lock, &mut idx));

    // Claim path fails before timelock
    let ctx_before_lock = EvalContext {
        current_height: 50,
        signing_hash: &tx_hash,
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
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &witness, &ctx_early, &mut idx));

    // After vesting period
    let ctx_vested = EvalContext {
        current_height: 1000,
        signing_hash: &tx_hash,
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
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &witness_no_preimage, &ctx_high, &mut idx));

    // Only timelock satisfied (1 of 3) — fail
    let witness_empty = Witness::default();
    let mut idx = 0;
    assert!(!evaluate(&cond, &witness_empty, &ctx_high, &mut idx));
}
