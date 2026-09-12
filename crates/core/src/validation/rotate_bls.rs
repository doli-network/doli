//! INC-I-217 M5 — the stateless consensus rules of a `RotateBlsKey` transaction.
//!
//! The check order is a wire contract: structural checks first, the two
//! pairing-bearing checks last, so a block of malformed rotations never makes a
//! validator pay a pairing. Stateful verdicts (registration status, key
//! uniqueness, one-pending) are M6/M7 and are deliberately absent here.

use crate::genesis::genesis_hash;
use crate::transaction::{
    rotation_auth_digest, OutputType, RotateBlsData, Transaction, ROTATE_BLS_DATA_LEN,
};

use super::{ValidationContext, ValidationError};

/// First byte of the compressed G1 identity; the remaining 47 bytes are zero.
const G1_IDENTITY_TAG: u8 = 0xc0;

/// Validate every stateless rule of a `RotateBlsKey` transaction and return the
/// payload it carries.
///
/// # Errors
///
/// One `[ERRTX-ROTnnn]` variant per rule, in the documented order: gate
/// (ROT002), input count (ROT001), output shape (ROT009), payload (ROT010),
/// new-key point (ROT004), producer binding (ROT008), authorisation signature
/// (ROT003), proof of possession (ROT007).
pub fn rotate_stateless(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<RotateBlsData, ValidationError> {
    if ctx.current_height < ctx.bls_key_rotation_activation_height {
        return Err(ValidationError::RotateBlsNotActivated {
            current_height: ctx.current_height,
            activation_height: ctx.bls_key_rotation_activation_height,
        });
    }

    if tx.inputs.len() != 1 {
        return Err(ValidationError::RotateBlsInputCount {
            got: tx.inputs.len(),
        });
    }

    if tx.outputs.len() != 1 || tx.outputs[0].output_type != OutputType::Normal {
        return Err(ValidationError::RotateBlsOutputShape {
            outputs: tx.outputs.len(),
            first_type: tx.outputs.first().map_or_else(
                || "none".to_string(),
                |output| format!("{:?}", output.output_type),
            ),
        });
    }

    let payload =
        RotateBlsData::decode(&tx.extra_data).ok_or(ValidationError::RotateBlsBadPayload {
            len: tx.extra_data.len(),
            expected: ROTATE_BLS_DATA_LEN,
        })?;

    let new_key = decode_new_key(&payload.new_bls_pubkey)?;

    let input = &tx.inputs[0];
    let producer = crypto::PublicKey::from_bytes(payload.producer);
    match input.public_key {
        Some(revealed) if *revealed.as_bytes() == payload.producer => {}
        revealed => {
            return Err(ValidationError::RotateBlsProducerMismatch {
                payload: producer.to_hex(),
                input: revealed.map_or_else(|| "none".to_string(), |key| key.to_hex()),
            })
        }
    }

    let genesis = genesis_hash(ctx.network);

    // The outpoint inside the digest is the replay bind (REQ-ROT-SEC-003): the
    // same tuple on another outpoint produces a different digest.
    let digest = rotation_auth_digest(
        genesis.as_bytes(),
        &payload.new_bls_pubkey,
        input.prev_tx_hash.as_bytes(),
        input.output_index,
    );
    let signature = crypto::Signature::from_bytes(payload.signature);
    crypto::signature::verify(&digest, &signature, &producer).map_err(|_| {
        ValidationError::RotateBlsBadSignature {
            producer: producer.to_hex(),
        }
    })?;

    crypto::verify_rotation_pop(
        &new_key,
        genesis.as_bytes(),
        &payload.producer,
        &crypto::BlsSignature::from_bytes_unchecked(payload.bls_pop),
    )
    .map_err(|_| ValidationError::RotateBlsBadPop {
        producer: producer.to_hex(),
    })?;

    Ok(payload)
}

/// Reject a `new_bls_pubkey` that can never attest, before any pairing runs.
///
/// `try_from_slice` accepts the compressed G1 identity, which is the one
/// "valid-looking" encoding a rogue-key attacker can produce a verifying proof
/// of possession for, so it is rejected explicitly.
fn decode_new_key(bytes: &[u8; 48]) -> Result<crypto::BlsPublicKey, ValidationError> {
    let reason = if bytes.iter().all(|byte| *byte == 0) {
        Some("zero")
    } else if bytes[0] == G1_IDENTITY_TAG && bytes[1..].iter().all(|byte| *byte == 0) {
        Some("identity")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(ValidationError::RotateBlsBadPoint {
            reason: reason.to_string(),
        });
    }

    crypto::BlsPublicKey::try_from_slice(bytes).map_err(|_| ValidationError::RotateBlsBadPoint {
        reason: "not_on_curve".to_string(),
    })
}
