//! INC-I-182 producer authorization: the named producer's Ed25519 signature,
//! appended after the `extra_data` payload of Registration, AddBond and
//! RequestWithdrawal. A field-list change bumps the `-V1` tag.

use crypto::{Hash, Hasher, KeyPair};

use super::Transaction;

/// Domain tag of [`producer_auth_digest`].
pub const PRODUCER_AUTH_DOMAIN: &[u8] = b"DOLI-PRODUCER-AUTH-V1";

/// Length of the signature that follows the payload.
pub const PRODUCER_AUTH_SIG_LEN: usize = 64;

/// BLAKE3(domain ‖ genesis_hash ‖ `tx.signing_message()` ‖ payload).
#[must_use]
pub fn producer_auth_digest(genesis_hash: &Hash, tx: &Transaction, payload: &[u8]) -> Hash {
    let mut hasher = Hasher::new();
    hasher.update(PRODUCER_AUTH_DOMAIN);
    hasher.update(genesis_hash.as_bytes());
    hasher.update(tx.signing_message().as_bytes());
    hasher.update(payload);
    hasher.finalize()
}

/// Append `key`'s signature over the current `extra_data` (the payload).
/// Inputs and outputs are untouched, so input signatures stay valid.
pub fn sign_producer_auth(tx: &mut Transaction, genesis_hash: &Hash, key: &KeyPair) {
    let digest = producer_auth_digest(genesis_hash, tx, &tx.extra_data);
    let signature = crypto::signature::sign_hash(&digest, key.private_key());
    tx.extra_data.extend_from_slice(signature.as_bytes());
}
