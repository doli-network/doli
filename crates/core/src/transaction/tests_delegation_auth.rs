//! INC-I-078 M2: DelegateBond / RevokeDelegation Ed25519 signature
//! authentication — wire-format and signing-commitment tests.
//!
//! The on-chain (height-gated) verification logic lives in
//! `bins/node/src/node/apply_block/tx_processing.rs` and is exercised by node
//! integration tests; this file pins the *data-structure* invariants:
//!
//! OUTPUT CONTRACT: fn signing_message(&self) -> Hash
//! O1: BLAKE3 hash of `DOMAIN || delegate || (bond_count_le)?`
//!
//! OUTPUT CONTRACT: fn to_bytes/from_bytes round-trip
//! O1: legacy (68/64 B) form round-trips with `signature == default`
//! O2: authenticated (132/128 B) form round-trips preserving signature
//! O3: from_bytes returns None on lengths between legacy and authenticated
//!     (no partial signatures allowed)
//!
//! INPUT PARTITIONS:
//!   I1 = legacy form (no signature)        → from_bytes Ok, signature=default
//!   I2 = authenticated form (with sig)     → from_bytes Ok, signature preserved
//!   I3 = malformed length (between forms)  → from_bytes None
//!   I4 = re-targeting changes signing_message (sec property)
//!   I5 = re-sizing changes signing_message (DelegateBond only)
//!   I6 = signing & verification round-trips with the delegator's key
//!
//! MATRIX:
//!   O1×I4: hash(target=A) != hash(target=B)
//!   O1×I5: hash(bond_count=1) != hash(bond_count=2)
//!   O2×I1: round-trip preserves all fields including default signature
//!   O2×I2: round-trip preserves authentic signature
//!   O3×I3: returns None on lengths 69..=131 / 65..=127

use crate::transaction::{
    DelegateBondData, RevokeDelegationData, DELEGATE_BOND_SIGNING_DOMAIN,
    REVOKE_DELEGATION_SIGNING_DOMAIN,
};
use crypto::{signature, KeyPair};

// ---------------------------------------------------------------------------
// signing_message: commitment correctness
// ---------------------------------------------------------------------------

#[test]
fn test_delegate_signing_message_includes_domain_and_fields() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let kp_c = KeyPair::generate();
    let d1 = DelegateBondData::new(*kp_a.public_key(), *kp_b.public_key(), 3);
    let d2 = DelegateBondData::new(*kp_a.public_key(), *kp_c.public_key(), 3);

    // O1×I4: re-targeting must change the signing message
    assert_ne!(
        d1.signing_message(),
        d2.signing_message(),
        "different delegate must produce different signing message"
    );

    // O1×I5: re-sizing must change the signing message
    let d3 = DelegateBondData::new(*kp_a.public_key(), *kp_b.public_key(), 7);
    assert_ne!(
        d1.signing_message(),
        d3.signing_message(),
        "different bond_count must produce different signing message"
    );
}

#[test]
fn test_delegate_signing_message_is_blake3_of_domain_delegate_count() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let d = DelegateBondData::new(*kp_a.public_key(), *kp_b.public_key(), 42);
    // Replicate the spec exactly: BLAKE3(domain || delegate || bond_count_le).
    let mut buf = Vec::new();
    buf.extend_from_slice(DELEGATE_BOND_SIGNING_DOMAIN);
    buf.extend_from_slice(kp_b.public_key().as_bytes());
    buf.extend_from_slice(&42u32.to_le_bytes());
    let expected = crypto::hash::hash(&buf);
    assert_eq!(d.signing_message(), expected);
}

#[test]
fn test_revoke_signing_message_is_blake3_of_domain_delegate() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let d = RevokeDelegationData::new(*kp_a.public_key(), *kp_b.public_key());
    let mut buf = Vec::new();
    buf.extend_from_slice(REVOKE_DELEGATION_SIGNING_DOMAIN);
    buf.extend_from_slice(kp_b.public_key().as_bytes());
    let expected = crypto::hash::hash(&buf);
    assert_eq!(d.signing_message(), expected);
}

// ---------------------------------------------------------------------------
// Wire-format compatibility (F3): legacy 68/64 B and authenticated 132/128 B
// ---------------------------------------------------------------------------

#[test]
fn test_delegate_legacy_form_roundtrips_with_default_signature() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let mut d = DelegateBondData::new(*kp_a.public_key(), *kp_b.public_key(), 5);
    // Pretend the sender used the pre-auth signing scheme: the legacy
    // 68-byte payload has no signature bytes.
    d.signature = signature::Signature::default();
    let legacy_bytes = d.to_bytes_legacy();
    assert_eq!(legacy_bytes.len(), DelegateBondData::LEGACY_BYTES_LEN);

    let parsed = DelegateBondData::from_bytes(&legacy_bytes)
        .expect("legacy form must parse via from_bytes (backward compat)");
    assert_eq!(parsed.delegator, d.delegator);
    assert_eq!(parsed.delegate, d.delegate);
    assert_eq!(parsed.bond_count, d.bond_count);
    assert_eq!(parsed.signature, signature::Signature::default());
}

#[test]
fn test_delegate_authenticated_form_roundtrips() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let mut d = DelegateBondData::new(*kp_a.public_key(), *kp_b.public_key(), 7);
    d.signature = signature::sign_hash(&d.signing_message(), kp_a.private_key());
    let bytes = d.to_bytes();
    assert_eq!(bytes.len(), DelegateBondData::AUTHENTICATED_BYTES_LEN);

    let parsed = DelegateBondData::from_bytes(&bytes).expect("authenticated form must parse");
    assert_eq!(parsed.delegator, d.delegator);
    assert_eq!(parsed.delegate, d.delegate);
    assert_eq!(parsed.bond_count, d.bond_count);
    assert_eq!(
        parsed.signature, d.signature,
        "signature bytes must round-trip exactly"
    );
}

#[test]
fn test_delegate_from_bytes_rejects_partial_signature_lengths() {
    // Any length between 68 and 132 must be REJECTED — partial signatures
    // aren't a thing. Either you sent the legacy form (68B) or you sent the
    // authenticated form (132B).
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let mut d = DelegateBondData::new(*kp_a.public_key(), *kp_b.public_key(), 1);
    d.signature = signature::sign_hash(&d.signing_message(), kp_a.private_key());
    let full = d.to_bytes();
    for trim in 1..=63 {
        let truncated = &full[..full.len() - trim];
        assert!(
            DelegateBondData::from_bytes(truncated).is_none(),
            "length {} must be rejected (partial-signature bytes)",
            truncated.len()
        );
    }
}

#[test]
fn test_revoke_legacy_form_roundtrips_with_default_signature() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let d = RevokeDelegationData::new(*kp_a.public_key(), *kp_b.public_key());
    let legacy = d.to_bytes_legacy();
    assert_eq!(legacy.len(), RevokeDelegationData::LEGACY_BYTES_LEN);
    let parsed = RevokeDelegationData::from_bytes(&legacy).expect("legacy revoke form must parse");
    assert_eq!(parsed.delegator, d.delegator);
    assert_eq!(parsed.delegate, d.delegate);
    assert_eq!(parsed.signature, signature::Signature::default());
}

#[test]
fn test_revoke_authenticated_form_roundtrips() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let mut d = RevokeDelegationData::new(*kp_a.public_key(), *kp_b.public_key());
    d.signature = signature::sign_hash(&d.signing_message(), kp_a.private_key());
    let bytes = d.to_bytes();
    assert_eq!(bytes.len(), RevokeDelegationData::AUTHENTICATED_BYTES_LEN);
    let parsed = RevokeDelegationData::from_bytes(&bytes).expect("auth revoke parse");
    assert_eq!(parsed.delegator, d.delegator);
    assert_eq!(parsed.delegate, d.delegate);
    assert_eq!(parsed.signature, d.signature);
}

#[test]
fn test_revoke_from_bytes_rejects_partial_signature_lengths() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let mut d = RevokeDelegationData::new(*kp_a.public_key(), *kp_b.public_key());
    d.signature = signature::sign_hash(&d.signing_message(), kp_a.private_key());
    let full = d.to_bytes();
    for trim in 1..=63 {
        let truncated = &full[..full.len() - trim];
        assert!(
            RevokeDelegationData::from_bytes(truncated).is_none(),
            "length {} must be rejected",
            truncated.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Signing + verification round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_delegate_signature_verifies_against_delegator_pubkey() {
    let delegator = KeyPair::generate();
    let delegate = KeyPair::generate();
    let mut d = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 9);
    d.signature = signature::sign_hash(&d.signing_message(), delegator.private_key());

    // Round-trip through bytes (simulating wire submission)
    let bytes = d.to_bytes();
    let parsed = DelegateBondData::from_bytes(&bytes).unwrap();

    // Verification with the delegator's public key MUST succeed.
    let r = crypto::signature::verify_hash(
        &parsed.signing_message(),
        &parsed.signature,
        &parsed.delegator,
    );
    assert!(r.is_ok(), "valid signature must verify: {r:?}");
}

#[test]
fn test_delegate_signature_rejects_wrong_signer() {
    let delegator = KeyPair::generate();
    let imposter = KeyPair::generate();
    let delegate = KeyPair::generate();
    let mut d = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 9);
    // Imposter signs the same message but is NOT the declared delegator.
    d.signature = signature::sign_hash(&d.signing_message(), imposter.private_key());

    let r = crypto::signature::verify_hash(&d.signing_message(), &d.signature, &d.delegator);
    assert!(
        r.is_err(),
        "signature by a different key must fail verification"
    );
}

#[test]
fn test_delegate_signature_rejects_tampered_bond_count() {
    let delegator = KeyPair::generate();
    let delegate = KeyPair::generate();
    let mut signed = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 3);
    signed.signature = signature::sign_hash(&signed.signing_message(), delegator.private_key());

    // Attacker tampers with bond_count after signing.
    let mut tampered = signed.clone();
    tampered.bond_count = 30;

    let r = crypto::signature::verify_hash(
        &tampered.signing_message(),
        &tampered.signature,
        &tampered.delegator,
    );
    assert!(r.is_err(), "tampered bond_count must invalidate signature");
}

#[test]
fn test_delegate_signature_rejects_tampered_delegate() {
    let delegator = KeyPair::generate();
    let delegate_a = KeyPair::generate();
    let delegate_b = KeyPair::generate();
    let mut signed = DelegateBondData::new(*delegator.public_key(), *delegate_a.public_key(), 5);
    signed.signature = signature::sign_hash(&signed.signing_message(), delegator.private_key());

    let mut tampered = signed.clone();
    tampered.delegate = *delegate_b.public_key();

    let r = crypto::signature::verify_hash(
        &tampered.signing_message(),
        &tampered.signature,
        &tampered.delegator,
    );
    assert!(
        r.is_err(),
        "redirecting delegation must invalidate signature"
    );
}

#[test]
fn test_revoke_signature_verifies() {
    let delegator = KeyPair::generate();
    let delegate = KeyPair::generate();
    let mut d = RevokeDelegationData::new(*delegator.public_key(), *delegate.public_key());
    d.signature = signature::sign_hash(&d.signing_message(), delegator.private_key());
    assert!(
        crypto::signature::verify_hash(&d.signing_message(), &d.signature, &d.delegator).is_ok()
    );
}

#[test]
fn test_revoke_signature_rejects_wrong_signer() {
    let delegator = KeyPair::generate();
    let imposter = KeyPair::generate();
    let delegate = KeyPair::generate();
    let mut d = RevokeDelegationData::new(*delegator.public_key(), *delegate.public_key());
    d.signature = signature::sign_hash(&d.signing_message(), imposter.private_key());
    assert!(
        crypto::signature::verify_hash(&d.signing_message(), &d.signature, &d.delegator).is_err(),
        "forgery (zero-input revoke from another key) must be rejected"
    );
}

#[test]
fn test_revoke_signature_rejects_tampered_delegate() {
    let delegator = KeyPair::generate();
    let delegate_a = KeyPair::generate();
    let delegate_b = KeyPair::generate();
    let mut signed = RevokeDelegationData::new(*delegator.public_key(), *delegate_a.public_key());
    signed.signature = signature::sign_hash(&signed.signing_message(), delegator.private_key());

    let mut tampered = signed.clone();
    tampered.delegate = *delegate_b.public_key();
    assert!(
        crypto::signature::verify_hash(
            &tampered.signing_message(),
            &tampered.signature,
            &tampered.delegator,
        )
        .is_err(),
        "retargeting a signed revoke must invalidate the signature"
    );
}

// ---------------------------------------------------------------------------
// Default (all-zeros) signature must FAIL verification — fail-closed for
// the legacy form post-activation.
// ---------------------------------------------------------------------------

#[test]
fn test_default_signature_fails_verification() {
    let delegator = KeyPair::generate();
    let delegate = KeyPair::generate();
    let d = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 1);
    // signature is default (all zeros) from `new()`.
    let r = crypto::signature::verify_hash(&d.signing_message(), &d.signature, &d.delegator);
    assert!(
        r.is_err(),
        "all-zero signature must fail verification — fail-closed for legacy form post-activation"
    );
}
