//! Tests for BLS12-381 operations.

use super::*;

#[test]
fn test_keygen() {
    let kp = BlsKeyPair::generate();
    assert!(!kp.public_key().is_zero());
}

#[test]
fn test_keygen_deterministic_from_bytes() {
    let sk = BlsSecretKey::generate();
    let pk1 = sk.public_key();
    let pk2 = sk.public_key();
    assert_eq!(pk1, pk2);
}

#[test]
fn test_sign_verify() {
    let kp = BlsKeyPair::generate();
    let msg = b"test message";

    let sig = bls_sign(msg, kp.secret_key()).unwrap();
    assert!(!sig.is_zero());

    bls_verify(msg, &sig, kp.public_key()).unwrap();
}

#[test]
fn test_wrong_key_fails() {
    let kp1 = BlsKeyPair::generate();
    let kp2 = BlsKeyPair::generate();
    let msg = b"test message";

    let sig = bls_sign(msg, kp1.secret_key()).unwrap();
    assert!(bls_verify(msg, &sig, kp2.public_key()).is_err());
}

#[test]
fn test_wrong_message_fails() {
    let kp = BlsKeyPair::generate();
    let sig = bls_sign(b"message A", kp.secret_key()).unwrap();
    assert!(bls_verify(b"message B", &sig, kp.public_key()).is_err());
}

#[test]
fn test_aggregate_verify() {
    let keys: Vec<BlsKeyPair> = (0..5).map(|_| BlsKeyPair::generate()).collect();
    let msg = b"shared message";

    let sigs: Vec<BlsSignature> = keys
        .iter()
        .map(|kp| bls_sign(msg, kp.secret_key()).unwrap())
        .collect();

    let agg = bls_aggregate(&sigs).unwrap();

    let pks: Vec<BlsPublicKeyWrapped> = keys.iter().map(|kp| *kp.public_key()).collect();
    bls_verify_aggregate(msg, &agg, &pks).unwrap();
}

#[test]
fn test_aggregate_missing_signer_fails() {
    let keys: Vec<BlsKeyPair> = (0..5).map(|_| BlsKeyPair::generate()).collect();
    let msg = b"shared message";

    // Only sign with first 4
    let sigs: Vec<BlsSignature> = keys[..4]
        .iter()
        .map(|kp| bls_sign(msg, kp.secret_key()).unwrap())
        .collect();

    let agg = bls_aggregate(&sigs).unwrap();

    // Verify claims all 5 signed — should fail
    let all_pks: Vec<BlsPublicKeyWrapped> = keys.iter().map(|kp| *kp.public_key()).collect();
    assert!(bls_verify_aggregate(msg, &agg, &all_pks).is_err());
}

#[test]
fn test_aggregate_wrong_message_fails() {
    let keys: Vec<BlsKeyPair> = (0..3).map(|_| BlsKeyPair::generate()).collect();

    let sigs: Vec<BlsSignature> = keys
        .iter()
        .map(|kp| bls_sign(b"message A", kp.secret_key()).unwrap())
        .collect();

    let agg = bls_aggregate(&sigs).unwrap();
    let pks: Vec<BlsPublicKeyWrapped> = keys.iter().map(|kp| *kp.public_key()).collect();

    assert!(bls_verify_aggregate(b"message B", &agg, &pks).is_err());
}

#[test]
fn test_aggregate_single_signer() {
    let kp = BlsKeyPair::generate();
    let msg = b"solo";

    let sig = bls_sign(msg, kp.secret_key()).unwrap();
    let agg = bls_aggregate(&[sig]).unwrap();

    bls_verify_aggregate(msg, &agg, &[*kp.public_key()]).unwrap();
}

#[test]
fn test_empty_aggregate_fails() {
    assert!(bls_aggregate(&[]).is_err());
}

#[test]
fn test_hex_roundtrip() {
    let kp = BlsKeyPair::generate();

    let pk_hex = kp.public_key().to_hex();
    let pk_recovered = BlsPublicKeyWrapped::from_hex(&pk_hex).unwrap();
    assert_eq!(kp.public_key(), &pk_recovered);

    let sk_hex = kp.secret_key().to_hex();
    let sk_recovered = BlsSecretKey::from_hex(&sk_hex).unwrap();
    assert_eq!(kp.secret_key().as_bytes(), sk_recovered.as_bytes());

    let msg = b"test";
    let sig = bls_sign(msg, kp.secret_key()).unwrap();
    let sig_hex = sig.to_hex();
    let sig_recovered = BlsSignature::from_hex(&sig_hex).unwrap();
    assert_eq!(sig, sig_recovered);
}

#[test]
fn test_serde_json_pubkey() {
    let kp = BlsKeyPair::generate();
    let json = serde_json::to_string(kp.public_key()).unwrap();
    let recovered: BlsPublicKeyWrapped = serde_json::from_str(&json).unwrap();
    assert_eq!(kp.public_key(), &recovered);
}

#[test]
fn test_serde_json_signature() {
    let kp = BlsKeyPair::generate();
    let sig = bls_sign(b"test", kp.secret_key()).unwrap();
    let json = serde_json::to_string(&sig).unwrap();
    let recovered: BlsSignature = serde_json::from_str(&json).unwrap();
    assert_eq!(sig, recovered);
}

#[test]
fn test_attestation_message_format() {
    let hash = crate::hash::hash(b"block data");
    let msg = attestation_message(&hash, 42);
    assert_eq!(msg.len(), 36); // 32 (hash) + 4 (slot)
    assert_eq!(&msg[32..], &42u32.to_be_bytes());
}

#[test]
fn test_attestation_sign_verify_flow() {
    let kp = BlsKeyPair::generate();
    let block_hash = crate::hash::hash(b"block 123");
    let slot = 42u32;

    let msg = attestation_message(&block_hash, slot);
    let sig = bls_sign(&msg, kp.secret_key()).unwrap();
    bls_verify(&msg, &sig, kp.public_key()).unwrap();
}

#[test]
fn test_aggregate_attestation_flow() {
    // Simulate 12 producers attesting the same block
    let producers: Vec<BlsKeyPair> = (0..12).map(|_| BlsKeyPair::generate()).collect();
    let block_hash = crate::hash::hash(b"block at slot 100");
    let msg = attestation_message(&block_hash, 100);

    // Each producer signs
    let sigs: Vec<BlsSignature> = producers
        .iter()
        .map(|kp| bls_sign(&msg, kp.secret_key()).unwrap())
        .collect();

    // Block producer aggregates
    let agg = bls_aggregate(&sigs).unwrap();

    // Validators verify (only need the aggregate + public keys)
    let pks: Vec<BlsPublicKeyWrapped> = producers.iter().map(|kp| *kp.public_key()).collect();
    bls_verify_aggregate(&msg, &agg, &pks).unwrap();

    // Verify the aggregate signature is exactly 96 bytes regardless of signer count
    assert_eq!(agg.as_bytes().len(), BLS_SIGNATURE_SIZE);
}

#[test]
fn test_debug_redacts_secret() {
    let sk = BlsSecretKey::generate();
    let debug = format!("{sk:?}");
    assert!(debug.contains("REDACTED"));
}

#[test]
fn test_zero_pubkey() {
    let zero = BlsPublicKeyWrapped::ZERO;
    assert!(zero.is_zero());
}

// ==================== Proof of Possession Tests ====================

#[test]
fn test_pop_sign_verify() {
    let kp = BlsKeyPair::generate();
    let pop = kp.proof_of_possession().unwrap();
    bls_verify_pop(kp.public_key(), &pop).unwrap();
}

#[test]
fn test_pop_wrong_key_fails() {
    let kp1 = BlsKeyPair::generate();
    let kp2 = BlsKeyPair::generate();
    let pop = kp1.proof_of_possession().unwrap();
    // PoP from kp1 must not verify against kp2's pubkey
    assert!(bls_verify_pop(kp2.public_key(), &pop).is_err());
}

#[test]
fn test_pop_prevents_rogue_key_attack() {
    // Scenario: attacker tries to forge aggregate attestation for victim
    let victim = BlsKeyPair::generate();
    let attacker = BlsKeyPair::generate();

    // Attacker cannot produce a valid PoP for a rogue key
    // (they'd need the secret key corresponding to the rogue pubkey)
    // This test verifies the `PoP` mechanism works — the actual rogue key
    // construction is prevented by requiring PoP at registration
    let attacker_pop = attacker.proof_of_possession().unwrap();
    bls_verify_pop(attacker.public_key(), &attacker_pop).unwrap();

    // Attacker's PoP doesn't verify against victim's key
    assert!(bls_verify_pop(victim.public_key(), &attacker_pop).is_err());
}

#[test]
fn test_pop_domain_separated_from_attestation() {
    // PoP signature must not be valid as an attestation signature and vice versa
    let kp = BlsKeyPair::generate();

    let pop = kp.proof_of_possession().unwrap();
    // PoP should NOT verify as an attestation over the same bytes
    assert!(bls_verify(kp.public_key().as_bytes(), &pop, kp.public_key()).is_err());

    // Attestation over pubkey bytes should NOT verify as PoP
    let attest_sig = bls_sign(kp.public_key().as_bytes(), kp.secret_key()).unwrap();
    assert!(bls_verify_pop(kp.public_key(), &attest_sig).is_err());
}

// ==================== Invalid Input Tests ====================

#[test]
fn test_invalid_scalar_from_bytes() {
    // All zeros is not a valid BLS scalar
    assert!(BlsSecretKey::from_bytes([0u8; 32]).is_err());
}

#[test]
fn test_invalid_pubkey_bytes() {
    // Random bytes are almost certainly not a valid G1 point
    let bad = [0xFFu8; BLS_PUBLIC_KEY_SIZE];
    assert!(BlsPublicKeyWrapped::try_from_slice(&bad).is_err());
}

#[test]
fn test_invalid_signature_bytes() {
    // Random bytes are almost certainly not a valid G2 point
    let bad = [0xFFu8; BLS_SIGNATURE_SIZE];
    assert!(BlsSignature::try_from_slice(&bad).is_err());
}

#[test]
fn test_serde_zero_pubkey_roundtrip() {
    let zero = BlsPublicKeyWrapped::ZERO;
    let json = serde_json::to_string(&zero).unwrap();
    let recovered: BlsPublicKeyWrapped = serde_json::from_str(&json).unwrap();
    assert_eq!(zero, recovered);
    assert!(recovered.is_zero());
}

#[test]
fn test_serde_zero_signature_roundtrip() {
    let zero = BlsSignature::ZERO;
    let json = serde_json::to_string(&zero).unwrap();
    let recovered: BlsSignature = serde_json::from_str(&json).unwrap();
    assert_eq!(zero, recovered);
    assert!(recovered.is_zero());
}
