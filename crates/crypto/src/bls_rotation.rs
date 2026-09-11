//! Proof of possession for a BLS key being rotated in (INC-I-217).
//!
//! Its own DST keeps a registration `PoP` and a rotation `PoP` mutually
//! unacceptable in either direction.

use blst::BLST_ERROR;

use crate::bls::{
    BlsError, BlsPublicKeyWrapped, BlsSecretKey, BlsSignature, BLS_PUBLIC_KEY_SIZE,
    BLS_SIGNATURE_SIZE,
};

/// Domain separation tag of the rotation proof of possession.
pub(crate) const ROTATE_POP_DST: &[u8] = b"DOLI-ROTATE-POP-V1";

/// `genesis_hash ‖ producer_ed25519 ‖ new_bls_pubkey` — genesis first, then
/// two fixed-width fields, so no length prefix is needed.
fn rotation_pop_message(
    genesis_hash: &[u8],
    producer_ed25519: &[u8; 32],
    public_key: &BlsPublicKeyWrapped,
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(genesis_hash.len() + 32 + BLS_PUBLIC_KEY_SIZE);
    msg.extend_from_slice(genesis_hash);
    msg.extend_from_slice(producer_ed25519);
    msg.extend_from_slice(public_key.as_bytes());
    msg
}

/// Sign a rotation proof of possession over [`rotation_pop_message`].
///
/// # Errors
///
/// Returns [`BlsError`] if signing fails.
pub fn sign_rotation_pop(
    secret_key: &BlsSecretKey,
    public_key: &BlsPublicKeyWrapped,
    genesis_hash: &[u8],
    producer_ed25519: &[u8; 32],
) -> Result<BlsSignature, BlsError> {
    let msg = rotation_pop_message(genesis_hash, producer_ed25519, public_key);
    let sig = secret_key.to_blst().sign(&msg, ROTATE_POP_DST, &[]);
    let bytes: [u8; BLS_SIGNATURE_SIZE] = sig.to_bytes();
    Ok(BlsSignature::from_bytes_unchecked(bytes))
}

/// Verify a rotation proof of possession.
///
/// # Errors
///
/// Returns [`BlsError::InvalidProofOfPossession`] when the signature does not
/// verify for this exact `(public_key, genesis_hash, producer_ed25519)`
/// triple under [`ROTATE_POP_DST`]. There is no fallback to the registration
/// DST.
pub fn verify_rotation_pop(
    public_key: &BlsPublicKeyWrapped,
    genesis_hash: &[u8],
    producer_ed25519: &[u8; 32],
    pop: &BlsSignature,
) -> Result<(), BlsError> {
    let sig = pop.to_blst()?;
    let pk = public_key.to_blst()?;
    let msg = rotation_pop_message(genesis_hash, producer_ed25519, public_key);

    if sig.verify(true, &msg, ROTATE_POP_DST, &[], &pk, true) != BLST_ERROR::BLST_SUCCESS {
        return Err(BlsError::InvalidProofOfPossession);
    }
    Ok(())
}
