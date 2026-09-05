//! Presence commitment — REQ-BLS-003 (INC-I-178 D6).

use crypto::Hash;

/// `BLAKE3(len_le(bitfield) ‖ bitfield ‖ len_le(aggregate) ‖ aggregate)`.
///
/// Both parts are length-prefixed so no byte can move across the split without
/// changing the commitment. Unconditional: empty/empty is a real hash, never the
/// `Hash::ZERO` sentinel the legacy decoders read as "no attestation data".
pub fn presence_commitment(bitfield: &[u8], aggregate: &[u8]) -> Hash {
    let mut preimage = Vec::with_capacity(8 + bitfield.len() + aggregate.len());
    preimage.extend_from_slice(&(bitfield.len() as u32).to_le_bytes());
    preimage.extend_from_slice(bitfield);
    preimage.extend_from_slice(&(aggregate.len() as u32).to_le_bytes());
    preimage.extend_from_slice(aggregate);
    crypto::hash::hash(&preimage)
}
