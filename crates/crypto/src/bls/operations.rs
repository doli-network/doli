//! BLS12-381 operations: signing, verification, aggregation, and proof-of-possession.

use blst::min_pk::{AggregateSignature, PublicKey as BlstPublicKey};
use blst::BLST_ERROR;

use super::types::{BlsPublicKeyWrapped, BlsSecretKey, BlsSignature};
use super::{BlsError, ATTESTATION_DST, POP_DST};

/// Sign a message with a BLS secret key.
///
/// Uses the DOLI attestation DST for domain separation.
///
/// # Errors
///
/// Returns error if the secret key bytes are invalid.
pub fn bls_sign(message: &[u8], secret_key: &BlsSecretKey) -> Result<BlsSignature, BlsError> {
    let sk = secret_key.to_blst();
    let sig = sk.sign(message, ATTESTATION_DST, &[]);
    Ok(BlsSignature::from_bytes_unchecked(sig.to_bytes()))
}

/// Verify a single BLS signature.
///
/// # Errors
///
/// Returns error if signature or public key is invalid, or verification fails.
pub fn bls_verify(
    message: &[u8],
    signature: &BlsSignature,
    public_key: &BlsPublicKeyWrapped,
) -> Result<(), BlsError> {
    let sig = signature.to_blst()?;
    let pk = public_key.to_blst()?;

    let result = sig.verify(true, message, ATTESTATION_DST, &[], &pk, true);
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BlsError::VerificationFailed);
    }
    Ok(())
}

/// Generate a proof-of-possession: sign the public key with the `PoP` DST.
///
/// This proves the caller possesses the secret key corresponding to the
/// public key, preventing rogue public key attacks on aggregate signatures.
///
/// # Errors
///
/// Returns error if signing fails.
pub fn bls_sign_pop(
    secret_key: &BlsSecretKey,
    public_key: &BlsPublicKeyWrapped,
) -> Result<BlsSignature, BlsError> {
    let sk = secret_key.to_blst();
    let sig = sk.sign(public_key.as_bytes(), POP_DST, &[]);
    Ok(BlsSignature::from_bytes_unchecked(sig.to_bytes()))
}

/// Verify a proof-of-possession for a BLS public key.
///
/// Checks that the `PoP` signature is a valid signature over the public key
/// bytes using the `PoP` DST. Must be verified at registration time before
/// the public key is accepted for aggregate verification.
///
/// # Errors
///
/// Returns error if the `PoP` is invalid.
pub fn bls_verify_pop(
    public_key: &BlsPublicKeyWrapped,
    pop: &BlsSignature,
) -> Result<(), BlsError> {
    let sig = pop.to_blst()?;
    let pk = public_key.to_blst()?;

    let result = sig.verify(true, public_key.as_bytes(), POP_DST, &[], &pk, true);
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BlsError::InvalidProofOfPossession);
    }
    Ok(())
}

/// Aggregate multiple BLS signatures into one.
///
/// The resulting signature can be verified against all corresponding public keys.
///
/// # Errors
///
/// Returns error if any signature is invalid or the set is empty.
pub fn bls_aggregate(signatures: &[BlsSignature]) -> Result<BlsSignature, BlsError> {
    if signatures.is_empty() {
        return Err(BlsError::EmptyAggregation);
    }

    let first = signatures[0].to_blst()?;
    let mut agg = AggregateSignature::from_signature(&first);

    for sig in &signatures[1..] {
        let s = sig.to_blst()?;
        agg.add_signature(&s, true)
            .map_err(|_| BlsError::InvalidSignature)?;
    }

    Ok(BlsSignature::from_bytes_unchecked(
        agg.to_signature().to_bytes(),
    ))
}

/// Verify an aggregate BLS signature against multiple public keys.
///
/// All public keys must have been PoP-validated at registration time.
/// Each public key signed the same `message`. The aggregate signature
/// is valid iff ALL individual signatures were valid.
///
/// # Errors
///
/// Returns error if verification fails, or any key/signature is invalid.
pub fn bls_verify_aggregate(
    message: &[u8],
    aggregate_signature: &BlsSignature,
    public_keys: &[BlsPublicKeyWrapped],
) -> Result<(), BlsError> {
    if public_keys.is_empty() {
        return Err(BlsError::EmptyAggregation);
    }

    let sig = aggregate_signature.to_blst()?;

    let blst_pks: Vec<BlstPublicKey> = public_keys
        .iter()
        .copied()
        .map(BlsPublicKeyWrapped::to_blst)
        .collect::<Result<Vec<_>, _>>()?;

    let pk_refs: Vec<&BlstPublicKey> = blst_pks.iter().collect();

    let result = sig.fast_aggregate_verify(true, message, ATTESTATION_DST, &pk_refs);
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BlsError::VerificationFailed);
    }

    Ok(())
}

/// Build the attestation message that producers sign.
///
/// Format: `block_hash || slot (4 bytes BE)`
///
/// Same structure as Ed25519 attestations for consistency.
#[must_use]
pub fn attestation_message(block_hash: &crate::Hash, slot: u32) -> Vec<u8> {
    let mut msg = Vec::with_capacity(36);
    msg.extend_from_slice(block_hash.as_bytes());
    msg.extend_from_slice(&slot.to_be_bytes());
    msg
}
